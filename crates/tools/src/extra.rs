//! Flare, Reshape, Shaper and Graph tools.
//!
//! Flare: drag sets the center and its size and turns the rays as the pointer moves up or down
//! (Shift holds their turn; click without dragging opens the Flare Tool Options), then a click
//! sets the end point of the rings; Space held during either drag moves the flare. The rest comes
//! from the tool's options (the Flare Tool Options, which last). Reshape: a click on a selected
//! path sets a focal point (adding an anchor on a segment), Shift-click adds or removes more, and
//! a drag from a focal point moves them all; the engine's `path.reshape` stretches the selected
//! anchors between them and the anchors that stay.

use serde_json::{Value, json};
use vectorcraft_geom::{Point, Vec2};

use crate::distort::{BLUE, ellipse_path};
use crate::{Action, Cursor, Mods, Overlay, PointerEvent, PointerKind, Tool, ToolContext, ToolKey};

pub fn create(id: &str) -> Option<Box<dyn Tool>> {
    Some(match id {
        "flare" => Box::new(FlareTool::default()),
        "reshape" => Box::new(ReshapeTool::default()),
        "shaper" => Box::new(ShaperTool::default()),
        id if is_graph_tool(id) => Box::new(GraphTool::new(id)),
        _ => return None,
    })
}

/// Whether `id` is one of the graph tools (`columnGraph`, `pieGraph`…).
pub fn is_graph_tool(id: &str) -> bool {
    id.ends_with("Graph") && vectorcraft_doc::GraphKind::parse(id).is_some()
}

/// The Flare Tool Options as `shape.flare` takes them: (key, default, least, most). The diameter
/// and the rings' path length are in points, the counts whole, the direction in degrees, the
/// rest percentages.
pub const FLARE_OPTIONS: [(&str, f64, f64, f64); 12] = [
    ("diameter", 100.0, 0.0, 1000.0),
    ("opacity", 50.0, 0.0, 100.0),
    ("brightness", 30.0, 0.0, 100.0),
    ("growth", 20.0, 0.0, 300.0),
    ("fuzziness", 50.0, 0.0, 100.0),
    ("rays", 15.0, 0.0, 50.0),
    ("longest", 300.0, 0.0, 1000.0),
    ("rayFuzziness", 100.0, 0.0, 100.0),
    ("pathLength", 300.0, 0.0, 1000.0),
    ("rings", 10.0, 0.0, 50.0),
    ("largest", 50.0, 0.0, 250.0),
    ("direction", 45.0, 0.0, 360.0),
];

/// Flare Tool: centre drag, then a click for the rings' end point.
pub struct FlareTool {
    /// Pressed at (centre) and the current drag point.
    drag: Option<(Point, Point)>,
    /// Centre placed, waiting for the end-point click: (centre, diameter).
    placed: Option<(Point, f64)>,
    hover: Point,
    /// The pointer at the last press or drag, from which Space held moves the flare.
    last: Point,
    began: bool,
    /// Ray count changed with ↑/↓ during the gesture (None = the options' count).
    rays: Option<u64>,
    /// The rays' turn (`shape.flare`'s `rotation`, degrees) the center drag set.
    turn: f64,
    /// The options' values, in [`FLARE_OPTIONS`] order.
    values: [f64; 12],
    /// Whether the flare has rays and rings (the options' Rays and Rings checkboxes).
    rays_on: bool,
    rings_on: bool,
}

impl Default for FlareTool {
    fn default() -> Self {
        Self {
            drag: None,
            placed: None,
            hover: Point::ZERO,
            last: Point::ZERO,
            began: false,
            rays: None,
            turn: 0.0,
            values: FLARE_OPTIONS.map(|(_, default, _, _)| default),
            rays_on: true,
            rings_on: true,
        }
    }
}

/// Whether Flare Tool Options value `key` is a count (whole).
fn is_count(key: &str) -> bool {
    matches!(key, "rays" | "rings")
}

/// Flare Tool Options value `x` of `key` as JSON: a count as a whole number.
fn option_json(key: &str, x: f64) -> Value {
    if is_count(key) { json!(x as u64) } else { json!(x) }
}

/// `shape.flare`'s params for a flare at `c` drawn with the Flare Tool Options `o` (as the tool
/// reports them, [`Tool::options`]): with Rays or Rings off it has none.
pub fn flare_params(o: &Value, c: Point) -> Value {
    let mut v = json!({ "cx": c.x, "cy": c.y });
    for (key, default, ..) in FLARE_OPTIONS {
        v[key] = o.get(key).cloned().unwrap_or_else(|| option_json(key, default));
    }
    for (on, key) in [("raysOn", "rays"), ("ringsOn", "rings")] {
        if o.get(on).and_then(Value::as_bool) == Some(false) {
            v[key] = json!(0);
        }
    }
    v
}

/// The chosen rotation rate: one degree per document point of vertical pointer travel.
/// This is independent of pointer event frequency and elapsed time.
const RAY_DEGREES_PER_POINT: f64 = 1.0;

/// Accumulate travel rather than an event-count step. Frozen/moving samples still update
/// the pointer baseline, so releasing a modifier never catches up skipped rotation.
fn ray_turn(turn: f64, dy: f64, shift: bool) -> f64 {
    if shift || !dy.is_finite() {
        return turn;
    }
    (turn + dy * RAY_DEGREES_PER_POINT).rem_euclid(360.0)
}

impl FlareTool {
    /// `shape.flare`'s params: the options, with the dragged center and diameter, the rays' turn,
    /// the ray count ↑/↓ chose and the rings' end point once clicked.
    fn params(&self, c: Point, diameter: f64, end: Option<Point>) -> Value {
        let mut v = flare_params(&self.options(), c);
        v["diameter"] = json!(diameter.max(1.0));
        v["rotation"] = json!(self.turn);
        if let Some(r) = self.rays.filter(|_| self.rays_on) {
            v["rays"] = json!(r);
        }
        if let Some(e) = end {
            v["x2"] = json!(e.x);
            v["y2"] = json!(e.y);
        }
        v
    }
}

impl Tool for FlareTool {
    fn id(&self) -> &'static str {
        "flare"
    }
    fn busy(&self) -> bool {
        self.drag.is_some() || self.placed.is_some()
    }
    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        self.hover = ev.pos;
        match ev.kind {
            PointerKind::Down => {
                if self.placed.is_none() {
                    self.drag = Some((ev.pos, ev.pos));
                    self.rays = None;
                    self.turn = 0.0;
                }
                self.last = ev.pos;
                vec![]
            }
            PointerKind::Drag => {
                if let Some((mut c, _)) = self.drag {
                    // Space held moves the center with the pointer, so the size and the turn stay.
                    let dy = ev.pos.y - self.last.y;
                    crate::shape::space_moves(&mut c, &mut self.last, ev, self.began);
                    self.drag = Some((c, ev.pos));
                    if !ev.mods.space {
                        self.turn = ray_turn(self.turn, dy, ev.mods.shift);
                    }
                    if !self.began && ev.pos.distance(c) >= cx.tol(3.0) {
                        self.began = true;
                        return vec![
                            Action::Begin("Flare".into()),
                            Action::Preview("shape.flare".into(), self.params(c, 2.0 * ev.pos.distance(c), None)),
                        ];
                    }
                    if self.began {
                        return vec![Action::Preview("shape.flare".into(), self.params(c, 2.0 * ev.pos.distance(c), None))];
                    }
                } else if let Some((mut c, d)) = self.placed {
                    // Space held moves the whole flare, rings' end point included.
                    crate::shape::space_moves(&mut c, &mut self.last, ev, true);
                    self.placed = Some((c, d));
                    return vec![Action::Preview("shape.flare".into(), self.params(c, d, Some(ev.pos)))];
                }
                vec![]
            }
            PointerKind::Up => {
                if let Some((mut c, d)) = self.placed.take() {
                    crate::shape::space_moves(&mut c, &mut self.last, ev, true);
                    self.began = false;
                    return vec![Action::Preview("shape.flare".into(), self.params(c, d, Some(ev.pos))), Action::Commit];
                }
                let Some((c, p)) = self.drag.take() else { return vec![] };
                if self.began {
                    // Keep the interaction open: the next click places the end point.
                    self.placed = Some((c, 2.0 * p.distance(c)));
                    vec![]
                } else {
                    vec![Action::Dialog("flare".into(), json!({ "x": c.x, "y": c.y }))]
                }
            }
            PointerKind::Move => {
                if let Some((c, d)) = self.placed {
                    return vec![Action::Preview("shape.flare".into(), self.params(c, d, Some(ev.pos)))];
                }
                vec![]
            }
            _ => vec![],
        }
    }
    fn key(&mut self, _cx: &ToolContext, key: ToolKey, _mods: Mods) -> Vec<Action> {
        // ↑/↓ while drawing add/remove rays.
        if matches!(key, ToolKey::Up | ToolKey::Down) && self.began && self.rays_on {
            let r = self.rays.unwrap_or_else(|| self.options()["rays"].as_u64().unwrap_or(0));
            self.rays = Some(if key == ToolKey::Up { (r + 1).min(50) } else { r.saturating_sub(1) });
            let preview = match (self.drag, self.placed) {
                (Some((c, p)), _) => self.params(c, 2.0 * p.distance(c), None),
                (_, Some((c, d))) => self.params(c, d, Some(self.hover)),
                _ => return vec![],
            };
            return vec![Action::Preview("shape.flare".into(), preview)];
        }
        if key == ToolKey::Escape && self.busy() {
            self.rays = None;
            self.drag = None;
            self.placed = None;
            let began = std::mem::take(&mut self.began);
            return if began { vec![Action::Cancel] } else { vec![] };
        }
        vec![]
    }
    fn deactivate(&mut self, _cx: &ToolContext) -> Vec<Action> {
        // Switching away after the centre drag keeps the flare with the rings its options give.
        self.drag = None;
        if self.placed.take().is_some() && std::mem::take(&mut self.began) { vec![Action::Commit] } else { vec![] }
    }
    fn overlays(&self, _cx: &ToolContext) -> Vec<Overlay> {
        match (self.drag, self.placed) {
            (Some((c, p)), _) => {
                vec![Overlay::Path { path: ellipse_path(c, p.distance(c), p.distance(c), 0.0), color: BLUE, width: 1.0, dashed: false }]
            }
            (_, Some((c, _))) => vec![Overlay::Line { a: c, b: self.hover, color: BLUE, dashed: true }],
            _ => vec![],
        }
    }
    fn cursor(&self, _cx: &ToolContext, _p: Point, _mods: Mods) -> Cursor {
        Cursor::Crosshair
    }
    fn options(&self) -> Value {
        let mut v = json!({ "raysOn": self.rays_on, "ringsOn": self.rings_on });
        for ((key, ..), x) in FLARE_OPTIONS.iter().zip(self.values) {
            v[*key] = option_json(key, x);
        }
        v
    }
    fn set_option(&mut self, key: &str, v: &Value) {
        match key {
            "raysOn" => self.rays_on = v.as_bool().unwrap_or(true),
            "ringsOn" => self.rings_on = v.as_bool().unwrap_or(true),
            _ => {
                let Some((slot, (_, default, lo, hi))) = self.values.iter_mut().zip(FLARE_OPTIONS).find(|(_, (k, ..))| *k == key) else { return };
                let x = v.as_f64().filter(|x| x.is_finite()).unwrap_or(default).clamp(lo, hi);
                *slot = if is_count(key) { x.round() } else { x };
            }
        }
    }
}

/// Where a Reshape press landed on a selected path.
#[derive(Clone, Copy, Debug, PartialEq)]
enum ReshapeHit {
    /// On an anchor: (subpath, anchor).
    Anchor(usize, usize),
    /// On a segment, away from its anchors: (subpath, the index of the anchor `path.reshape` adds
    /// there).
    Segment(usize, usize),
}

impl ReshapeHit {
    /// The anchor the press grabs, as `(subpath, anchor)` once any anchor has been added.
    fn anchor(self) -> (usize, usize) {
        match self {
            Self::Anchor(si, ai) | Self::Segment(si, ai) => (si, ai),
        }
    }

    /// Where the anchor `r` is once the press has added its anchor: one further on after an
    /// anchor added before it on its subpath.
    fn renumber(self, r: (usize, usize)) -> (usize, usize) {
        match self {
            Self::Segment(si, added) if r.0 == si && r.1 >= added => (r.0, r.1.saturating_add(1)),
            _ => r,
        }
    }
}

/// A Reshape press: the path, the point on it, what it landed on and whether Shift was held.
#[derive(Clone, Debug)]
struct ReshapePress {
    id: vectorcraft_doc::NodeId,
    at: Point,
    hit: ReshapeHit,
    shift: bool,
    /// The drag so far.
    d: Vec2,
    /// The other focal points the drag moves, found when it starts, before the preview moves them.
    others: Vec<((usize, usize), Point)>,
}

/// How near a focal point's stored position an anchor must be to be that focal point. An anchor
/// added on a segment lies where the curve's point was found, to within rounding.
const FOCAL_MATCH: f64 = 1e-6;

/// A focal point: its `(subpath, anchor)` and where that anchor lies.
type FocalPoint = ((usize, usize), Point);

/// Reshape Tool: a click on a selected path makes the anchor or the point on a segment there the
/// focal point (on a segment it adds an anchor), Shift-click adds or removes more, and a drag
/// moves the focal points with the drag and stretches the selected anchors around them.
#[derive(Default)]
pub struct ReshapeTool {
    /// The press, until the button is released.
    press: Option<ReshapePress>,
    began: bool,
    /// Pointer and offset readout while a reshape drag is in progress.
    measure: Option<(Point, String)>,
    /// The focal points the tool highlighted: their path, and each one's `(subpath, anchor)` and
    /// where that anchor lies. An anchor the tool adds before one renumbers it. A focal point
    /// whose anchor no longer lies there (another edit moved or removed it) shows nothing and
    /// moves nothing.
    focal: Option<(vectorcraft_doc::NodeId, Vec<FocalPoint>)>,
}

impl ReshapeTool {
    /// The selected path under `p` (Reshape works on selected paths only), the point on it and
    /// what the point is.
    fn hit(cx: &ToolContext, p: Point) -> Option<(vectorcraft_doc::NodeId, Point, ReshapeHit)> {
        let tol = cx.tol(4.0);
        Self::paths(cx).into_iter().find_map(|id| {
            let pd = cx.doc.node(id)?.path_data()?;
            let (si, seg, _, q, d) = pd.nearest(p)?;
            if d > tol {
                return None;
            }
            // `path.reshape` takes the anchor within `tol` of the point on the path.
            let anchor = pd
                .subpaths
                .get(si)?
                .anchors
                .iter()
                .enumerate()
                .map(|(i, a)| (i, a.p.distance(q)))
                .filter(|&(_, dist)| dist <= tol)
                .min_by(|a, b| a.1.total_cmp(&b.1));
            Some(match anchor {
                Some((ai, _)) => (id, pd.subpaths.get(si)?.anchors.get(ai)?.p, ReshapeHit::Anchor(si, ai)),
                // `SubPath::insert_anchor` puts the new anchor after the segment's first anchor.
                None => (id, q, ReshapeHit::Segment(si, seg.saturating_add(1))),
            })
        })
    }

    /// The paths Reshape works on: the selected paths that can be edited, in selection order.
    fn paths(cx: &ToolContext) -> Vec<vectorcraft_doc::NodeId> {
        let editable = cx.doc.editable();
        cx.selection.objects.iter().copied().filter(|&id| editable(id) && cx.doc.node(id).is_some_and(|n| n.path_data().is_some())).collect()
    }

    /// The highlighted focal points of path `id` as `(subpath, anchor)`, with their positions: the
    /// anchors of the path that lie where the tool stored them. None while Reshape does not work
    /// on the path.
    fn focal_points(&self, cx: &ToolContext, id: vectorcraft_doc::NodeId) -> Vec<((usize, usize), Point)> {
        let Some((fid, points)) = &self.focal else { return vec![] };
        if *fid != id || !Self::paths(cx).contains(&id) {
            return vec![];
        }
        let Some(pd) = cx.doc.node(id).and_then(|n| n.path_data()) else { return vec![] };
        let mut out: Vec<((usize, usize), Point)> = vec![];
        for &((si, ai), p) in points {
            let here = pd.subpaths.get(si).and_then(|sp| sp.anchors.get(ai)).map(|a| a.p);
            if let Some(q) = here.filter(|q| q.distance(p) <= FOCAL_MATCH)
                && !out.iter().any(|(o, _)| *o == (si, ai))
            {
                out.push(((si, ai), q));
            }
        }
        out
    }

    /// The focal points other than the pressed point that a drag from `press` moves: all the
    /// highlighted ones when the press is on one of them, else none (Shift only keeps the drag's
    /// angle).
    fn dragged_focal(&self, cx: &ToolContext, press: &ReshapePress) -> Vec<((usize, usize), Point)> {
        let focal = self.focal_points(cx, press.id);
        let pressed = |r: &(usize, usize)| matches!(press.hit, ReshapeHit::Anchor(si, ai) if (si, ai) == *r);
        if focal.iter().any(|(r, _)| pressed(r)) { focal.into_iter().filter(|(r, _)| !pressed(r)).collect() } else { vec![] }
    }

    /// `path.reshape`'s params for `press` dragged by `d`.
    fn params(cx: &ToolContext, press: &ReshapePress, d: Vec2) -> Value {
        let others: Vec<[usize; 2]> = press.others.iter().map(|&((si, ai), _)| [si, ai]).collect();
        // `focal` is sent when empty too: without it, `path.reshape` follows the earlier rule.
        json!({ "id": press.id.0, "x": press.at.x, "y": press.at.y, "dx": d.x, "dy": d.y, "tol": cx.tol(4.0), "focal": others })
    }
}

impl Tool for ReshapeTool {
    fn id(&self) -> &'static str {
        "reshape"
    }
    fn busy(&self) -> bool {
        self.press.is_some()
    }
    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        match ev.kind {
            PointerKind::Down => {
                self.press =
                    Self::hit(cx, ev.pos).map(|(id, at, hit)| ReshapePress { id, at, hit, shift: ev.mods.shift, d: Vec2::ZERO, others: vec![] });
                self.began = false;
                self.measure = None;
                vec![]
            }
            PointerKind::Drag => {
                let Some(at) = self.press.as_ref().map(|p| p.at) else { return vec![] };
                let mut d: Vec2 = ev.pos - at;
                if ev.mods.shift {
                    d = vectorcraft_geom::constrain_angle_from(d, 45.0, cx.constrain_angle);
                }
                let mut out = vec![];
                if !self.began {
                    if d.hypot() < cx.tol(2.0) {
                        return out;
                    }
                    self.began = true;
                    let others = self.press.as_ref().map(|p| self.dragged_focal(cx, p)).unwrap_or_default();
                    if let Some(p) = self.press.as_mut() {
                        p.others = others;
                    }
                    out.push(Action::Begin("Reshape".into()));
                }
                let Some(press) = self.press.as_mut() else { return out };
                press.d = d;
                self.measure = cx.measurement_labels.then(|| (ev.pos + Vec2::new(cx.tol(12.0), cx.tol(12.0)), cx.offset_label(d.x, d.y)));
                out.push(Action::Preview("path.reshape".into(), Self::params(cx, press, d)));
                out
            }
            PointerKind::Up => {
                self.measure = None;
                let Some(press) = self.press.take() else { return vec![] };
                if std::mem::take(&mut self.began) {
                    // The focal points move with the whole drag, the grabbed point too, and are
                    // renumbered after an anchor added on a segment.
                    let mut points: Vec<((usize, usize), Point)> = press.others.iter().map(|&(r, p)| (press.hit.renumber(r), p + press.d)).collect();
                    points.push((press.hit.anchor(), press.at + press.d));
                    self.focal = Some((press.id, points));
                    return vec![Action::Commit];
                }
                // A click: the point becomes the focal point, or with Shift is added to them or,
                // when it is one already, removed.
                let mut points: Vec<((usize, usize), Point)> = if press.shift { self.focal_points(cx, press.id) } else { vec![] };
                let mut out = vec![];
                match press.hit {
                    ReshapeHit::Anchor(si, ai) => {
                        let before = points.len();
                        points.retain(|&(r, _)| r != (si, ai));
                        if points.len() == before {
                            points.push(((si, ai), press.at));
                        }
                    }
                    ReshapeHit::Segment(..) => {
                        // A reshape by no distance adds the anchor and keeps the selection as it is.
                        out.push(Action::Exec("path.reshape".into(), Self::params(cx, &press, Vec2::ZERO)));
                        for (r, _) in &mut points {
                            *r = press.hit.renumber(*r);
                        }
                        points.push((press.hit.anchor(), press.at));
                    }
                }
                self.focal = (!points.is_empty()).then_some((press.id, points));
                out
            }
            _ => vec![],
        }
    }
    fn key(&mut self, _cx: &ToolContext, key: ToolKey, _mods: Mods) -> Vec<Action> {
        if key == ToolKey::Escape && self.press.take().is_some() {
            self.measure = None;
            if std::mem::take(&mut self.began) {
                return vec![Action::Cancel];
            }
        }
        vec![]
    }
    fn deactivate(&mut self, _cx: &ToolContext) -> Vec<Action> {
        self.focal = None;
        self.measure = None;
        if self.press.take().is_some() && std::mem::take(&mut self.began) { vec![Action::Cancel] } else { vec![] }
    }
    fn overlays(&self, cx: &ToolContext) -> Vec<Overlay> {
        let mut out = vec![];
        // Each focal point: a filled anchor square inside a larger hollow one.
        if let Some((id, _)) = &self.focal {
            let color = cx.doc.layer_color(*id);
            for (_, p) in self.focal_points(cx, *id) {
                out.push(Overlay::Anchor { p, color, filled: false, size: 10.0 });
                out.push(Overlay::Anchor { p, color, filled: true, size: 5.0 });
            }
        }
        if cx.measurement_labels
            && let Some((p, t)) = &self.measure
        {
            out.push(Overlay::Measure { p: *p, text: t.clone() });
        }
        out
    }
    fn cursor(&self, cx: &ToolContext, p: Point, _mods: Mods) -> Cursor {
        if self.press.is_some() || Self::hit(cx, p).is_some() { Cursor::Move } else { Cursor::Arrow }
    }
}

/// Shaper Tool: draw a rough shape and it becomes a clean live shape (rectangle, ellipse,
/// triangle/polygon, line); scribble over art to delete it.
#[derive(Default)]
pub struct ShaperTool {
    points: Vec<Point>,
}

impl Tool for ShaperTool {
    fn id(&self) -> &'static str {
        "shaper"
    }
    fn busy(&self) -> bool {
        !self.points.is_empty()
    }
    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        use vectorcraft_geom::recognize::{Recognized, recognize};
        match ev.kind {
            PointerKind::Down => {
                if let Some(a) = crate::shaperwidget::click(cx, ev.pos) {
                    self.points.clear();
                    return vec![a];
                }
                self.points = vec![ev.pos];
                vec![]
            }
            PointerKind::Drag => {
                if !self.points.is_empty() && self.points.len() < 4095 && self.points.last().is_none_or(|l| l.distance(ev.pos) >= cx.tol(1.5)) {
                    self.points.push(ev.pos);
                }
                vec![]
            }
            PointerKind::Up => {
                // A fast final segment may arrive only on release; it still closes the stroke.
                if !self.points.is_empty() && self.points.last().is_none_or(|p| *p != ev.pos) {
                    self.points.push(ev.pos);
                }
                let pts = std::mem::take(&mut self.points);
                if pts.first().is_some_and(|first| pts.iter().all(|p| p.distance(*first) < cx.tol(3.0))) {
                    // In Construction mode a tap selects a source shape.
                    let construction =
                        cx.isolation.and_then(|id| cx.doc.node(id)).is_some_and(|n| n.name.as_deref() == Some(vectorcraft_doc::shaper::SOURCES));
                    let mut params = json!({"point": [ev.pos.x, ev.pos.y]});
                    if construction {
                        params["source"] = json!(true);
                    }
                    return vec![Action::Exec("shaper.select".into(), params)];
                }
                let r = |x: vectorcraft_geom::Rect, rotation: f64| json!({ "x": x.x0, "y": x.y0, "width": x.width(), "height": x.height(), "rotation": rotation });
                match recognize(&pts) {
                    Some(Recognized::Line { a, b }) => vec![Action::Exec("shape.line".into(), json!({ "x1": a.x, "y1": a.y, "x2": b.x, "y2": b.y }))],
                    Some(Recognized::Rectangle { rect, rotation }) => vec![Action::Exec("shape.rectangle".into(), r(rect, rotation))],
                    Some(Recognized::Ellipse { rect, rotation }) => vec![Action::Exec("shape.ellipse".into(), r(rect, rotation))],
                    Some(Recognized::Polygon { center, radius, sides, rotation }) => {
                        vec![Action::Exec(
                            "shape.polygon".into(),
                            json!({ "cx": center.x, "cy": center.y, "radius": radius, "sides": sides, "rotation": rotation }),
                        )]
                    }
                    Some(Recognized::Scribble(_)) => vec![Action::Exec(
                        "shaper.scribble".into(),
                        json!({"points": pts.iter().map(|p| [p.x, p.y]).collect::<Vec<_>>(), "tolerance": cx.tol(4.0)}),
                    )],
                    None => vec![],
                }
            }
            // The mode widget answers each click of a double-click on it.
            PointerKind::DoubleClick if crate::shaperwidget::over_widget(cx, ev.pos) => vec![],
            PointerKind::DoubleClick => vec![
                Action::Exec("shaper.select".into(), json!({"point": [ev.pos.x, ev.pos.y], "source": true})),
                Action::SwitchTool("selection".into()),
            ],
            _ => vec![],
        }
    }
    fn key(&mut self, _cx: &ToolContext, key: ToolKey, _mods: Mods) -> Vec<Action> {
        if key == ToolKey::Escape {
            self.points.clear();
        }
        vec![]
    }
    fn overlays(&self, _cx: &ToolContext) -> Vec<Overlay> {
        if self.points.len() < 2 {
            return vec![];
        }
        vec![Overlay::Path { path: crate::draw2::polyline(&self.points), color: BLUE, width: 1.5, dashed: false }]
    }
    fn cursor(&self, _cx: &ToolContext, _p: Point, _mods: Mods) -> Cursor {
        Cursor::Shaper
    }
}

/// Graph tools: drag the plot rectangle (Shift = square, Alt = from the centre); a click opens the
/// size dialog. A new graph opens the Graph Data dialog.
pub struct GraphTool {
    id: &'static str,
    start: Option<Point>,
    last: Point,
    mods: Mods,
    began: bool,
}

impl GraphTool {
    fn new(id: &str) -> Self {
        let id = crate::catalog::tool_info(id).map(|t| t.id).unwrap_or("columnGraph");
        Self { id, start: None, last: Point::ZERO, mods: Mods::default(), began: false }
    }
    fn kind(&self) -> &'static str {
        vectorcraft_doc::GraphKind::parse(self.id).unwrap_or_default().id()
    }
    /// The plot rectangle a drag from `s` to `p` draws (Shift = square, Alt = from the centre).
    fn plot_rect(s: Point, p: Point, m: Mods) -> vectorcraft_geom::Rect {
        let mut d = p - s;
        if m.shift {
            let k = d.x.abs().max(d.y.abs());
            d = Vec2::new(k * d.x.signum(), k * d.y.signum());
        }
        if m.alt { vectorcraft_geom::Rect::from_points(s - d, s + d) } else { vectorcraft_geom::Rect::from_points(s, s + d) }
    }
    fn params(&self, s: Point, p: Point, m: Mods) -> Value {
        let r = Self::plot_rect(s, p, m);
        json!({ "type": self.kind(), "x": r.x0, "y": r.y0, "width": r.width().max(1.0), "height": r.height().max(1.0) })
    }
}

impl Tool for GraphTool {
    fn id(&self) -> &'static str {
        self.id
    }
    fn busy(&self) -> bool {
        self.start.is_some()
    }
    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        match ev.kind {
            PointerKind::Down => {
                self.start = Some(ev.pos);
                self.last = ev.pos;
                self.began = false;
                vec![]
            }
            PointerKind::Drag => {
                let Some(s) = self.start else { return vec![] };
                self.last = ev.pos;
                self.mods = ev.mods;
                let mut out = vec![];
                if !self.began {
                    if ev.pos.distance(s) < cx.tol(3.0) {
                        return out;
                    }
                    self.began = true;
                    out.push(Action::Begin("Graph".into()));
                }
                out.push(Action::Preview("graph.create".into(), self.params(s, ev.pos, ev.mods)));
                out
            }
            PointerKind::Up => {
                let Some(s) = self.start.take() else { return vec![] };
                if std::mem::take(&mut self.began) {
                    vec![
                        Action::Preview("graph.create".into(), self.params(s, ev.pos, ev.mods)),
                        Action::Commit,
                        Action::Dialog("graphData".into(), json!({})),
                    ]
                } else {
                    vec![Action::Dialog("graph".into(), json!({ "x": s.x, "y": s.y, "type": self.kind() }))]
                }
            }
            _ => vec![],
        }
    }
    fn key(&mut self, _cx: &ToolContext, key: ToolKey, _mods: Mods) -> Vec<Action> {
        if key == ToolKey::Escape && self.start.take().is_some() && std::mem::take(&mut self.began) {
            return vec![Action::Cancel];
        }
        vec![]
    }
    fn overlays(&self, cx: &ToolContext) -> Vec<Overlay> {
        if let Some(s) = self.start.filter(|_| self.began && cx.measurement_labels) {
            let r = Self::plot_rect(s, self.last, self.mods);
            vec![Overlay::Measure { p: self.last, text: cx.size_label(r.width(), r.height()) }]
        } else {
            vec![]
        }
    }
    fn cursor(&self, _cx: &ToolContext, _p: Point, _mods: Mods) -> Cursor {
        Cursor::Crosshair
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The Flare Tool Options: kept within their ranges, and what the next drag draws with.
    #[test]
    fn flare_options_last_and_draw_the_next_flare() {
        let mut t = FlareTool::default();
        assert_eq!(t.options()["rays"], json!(15));
        for (k, v) in [("diameter", json!(5000)), ("rays", json!(7.6)), ("opacity", json!("x")), ("ringsOn", json!(false)), ("nope", json!(1))] {
            t.set_option(k, &v);
        }
        let o = t.options();
        assert_eq!(
            (o["diameter"].as_f64(), o["rays"].as_u64(), o["opacity"].as_f64(), o["ringsOn"].as_bool()),
            (Some(1000.0), Some(8), Some(50.0), Some(false))
        );
        assert!(o.get("nope").is_none());
        // A drag sets the centre and diameter; the options give the rest, rings off.
        let p = t.params(Point::new(10.0, 20.0), 40.0, None);
        assert_eq!((p["cx"].as_f64(), p["diameter"].as_f64(), p["rays"].as_u64(), p["rings"].as_u64()), (Some(10.0), Some(40.0), Some(8), Some(0)));
        assert_eq!(p["longest"].as_f64(), Some(300.0));
        t.set_option("raysOn", &json!(false));
        assert_eq!(t.params(Point::ZERO, 1.0, None)["rays"].as_u64(), Some(0));
    }

    /// The last `shape.flare` preview of `a`.
    fn flare_preview(a: &[Action]) -> Value {
        match a.iter().rev().find(|a| matches!(a, Action::Preview(..))) {
            Some(Action::Preview(c, v)) if c == "shape.flare" => v.clone(),
            other => panic!("no flare preview: {other:?}"),
        }
    }

    /// Equal vertical travel gives equal rotation, regardless of event partitioning.
    #[test]
    fn flare_drag_turns_the_rays_and_shift_holds_their_turn() {
        use crate::testutil::*;
        use vectorcraft_doc::Selection;
        let (d, _) = doc_with_rect();
        let s = Selection::default();
        let p = paint();
        let cx = cx(&d, &s, &p);
        let shift = Mods { shift: true, ..Default::default() };
        let space = Mods { space: true, ..Default::default() };
        let mut results = vec![];
        for steps in [1, 8, 64] {
            let mut t = FlareTool::default();
            t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 100.0, 100.0));
            for i in 1..=steps {
                t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 140.0, 100.0 + 80.0 * i as f64 / steps as f64));
            }
            assert_eq!(t.turn, 80.0);
            t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 180.0, 180.0));
            assert_eq!(t.turn, 80.0, "horizontal movement doesn't turn");
            t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 180.0, 150.0));
            assert_eq!(t.turn, 50.0, "reverse travel subtracts its distance");
            let v = flare_preview(&t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 100.0, 200.0).with_mods(shift)));
            assert_eq!(v["rotation"].as_f64(), Some(50.0));
            assert_eq!(v["diameter"].as_f64(), Some(200.0));
            t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 120.0, 220.0));
            assert_eq!(t.turn, 70.0, "release does not catch up frozen movement");
            t.key(&cx, ToolKey::Up, Mods::default());
            assert_eq!(t.turn, 70.0);
            t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 120.0, 220.0));
            t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 300.0, 200.0));
            let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 300.0, 260.0));
            assert_eq!(a.last(), Some(&Action::Commit));
            assert_eq!(flare_preview(&a)["rotation"].as_f64(), Some(70.0));
            results.push(t.turn);
        }
        assert!(results.windows(2).all(|p| p[0] == p[1]));
        // Space held before the first drag update freezes rotation too.
        let mut t = FlareTool::default();
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 100.0, 100.0));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 140.0, 140.0).with_mods(space));
        assert_eq!(t.turn, 0.0);
        t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 140.0, 150.0));
        assert_eq!(t.turn, 10.0);
    }

    /// Space held while dragging out a flare moves it at its current size and turn, in the center
    /// drag and in the drag that places the rings' end point; let go, the drag goes on from there.
    #[test]
    fn space_moves_the_flare_being_drawn() {
        use crate::testutil::*;
        use vectorcraft_doc::Selection;

        let (d, _) = doc_with_rect();
        let s = Selection::default();
        let p = paint();
        let cx = cx(&d, &s, &p);
        let space = Mods { space: true, ..Default::default() };
        let step = 30.0;
        let mut t = FlareTool::default();
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 100.0, 100.0));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 100.0, 130.0));
        // Space held: the pointer's y grows, but the flare moves without turning.
        let v = flare_preview(&t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 150.0, 180.0).with_mods(space)));
        let at = |v: &Value| (v["cx"].as_f64(), v["cy"].as_f64(), v["diameter"].as_f64());
        let turned = |v: &Value, by: f64| v["rotation"].as_f64().is_some_and(|r| (r - by).abs() < 1e-9);
        assert_eq!(at(&v), (Some(150.0), Some(150.0), Some(60.0)));
        assert!(turned(&v, step));
        // Space let go: the size follows the pointer again, from the moved center, and so does the turn.
        let v = flare_preview(&t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 150.0, 190.0)));
        assert_eq!(at(&v), (Some(150.0), Some(150.0), Some(80.0)));
        assert!(turned(&v, step + 10.0));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 150.0, 190.0));
        // The second drag: Space held moves the flare, its rings' end point with it.
        t.pointer(&cx, &PointerEvent::new(PointerKind::Move, 300.0, 300.0));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 300.0, 300.0));
        let v = flare_preview(&t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 320.0, 310.0).with_mods(space)));
        assert_eq!((at(&v), v["x2"].as_f64(), v["y2"].as_f64()), ((Some(170.0), Some(160.0), Some(80.0)), Some(320.0), Some(310.0)));
        assert!(turned(&v, step + 10.0));
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 340.0, 310.0));
        assert_eq!(a.last(), Some(&Action::Commit));
        let v = flare_preview(&a);
        assert_eq!((v["cx"].as_f64(), v["cy"].as_f64(), v["x2"].as_f64()), (Some(170.0), Some(160.0), Some(340.0)));
        // A release with Space held away from the last drag moves the flare there too.
        let mut t = FlareTool::default();
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 100.0, 100.0));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 130.0, 100.0));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 130.0, 100.0));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 300.0, 300.0));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 320.0, 300.0));
        let v = flare_preview(&t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 340.0, 310.0).with_mods(space)));
        assert_eq!((v["cx"].as_f64(), v["cy"].as_f64(), v["x2"].as_f64()), (Some(120.0), Some(110.0), Some(340.0)));
    }

    #[test]
    fn reshape_drag_honors_measurement_labels() {
        use crate::testutil::*;
        use vectorcraft_doc::Selection;

        let (d, id) = doc_with_rect();
        let s = Selection { objects: vec![id], ..Selection::default() };
        let p = paint();
        let mut t = ReshapeTool::default();
        let drag = |t: &mut ReshapeTool, c: &ToolContext| {
            t.pointer(c, &PointerEvent::new(PointerKind::Down, 100.0, 150.0));
            t.pointer(c, &PointerEvent::new(PointerKind::Drag, 130.0, 150.0));
        };
        let on = ToolContext { measurement_labels: true, ..cx(&d, &s, &p) };
        drag(&mut t, &on);
        let want = on.offset_label(30.0, 0.0);
        assert!(t.overlays(&on).iter().any(|o| matches!(o, Overlay::Measure { text, .. } if text == &want)));
        let off = ToolContext { measurement_labels: false, ..cx(&d, &s, &p) };
        assert!(!t.overlays(&off).iter().any(|o| matches!(o, Overlay::Measure { .. })), "overlay follows the current pref");
        let mut quiet = ReshapeTool::default();
        drag(&mut quiet, &off);
        assert!(!quiet.overlays(&off).iter().any(|o| matches!(o, Overlay::Measure { .. })));
    }

    #[test]
    fn graph_plot_measure_labels() {
        use crate::testutil::*;
        use vectorcraft_doc::Selection;

        let (d, _) = doc_with_rect();
        let s = Selection::default();
        let p = paint();
        let measure_count = |t: &GraphTool, c: &ToolContext| t.overlays(c).iter().filter(|o| matches!(o, Overlay::Measure { .. })).count();
        let measure_text = |t: &GraphTool, c: &ToolContext| {
            t.overlays(c).iter().find_map(|o| match o {
                Overlay::Measure { text, .. } => Some(text.clone()),
                _ => None,
            })
        };
        let mut t = GraphTool::new("columnGraph");
        let drag = |t: &mut GraphTool, c: &ToolContext| {
            t.pointer(c, &PointerEvent::new(PointerKind::Down, 10.0, 10.0));
            t.pointer(c, &PointerEvent::new(PointerKind::Drag, 110.0, 60.0));
        };
        let on = ToolContext { measurement_labels: true, ..cx(&d, &s, &p) };
        drag(&mut t, &on);
        assert_eq!(measure_count(&t, &on), 1);
        let text = measure_text(&t, &on).unwrap_or_default();
        assert!(text.starts_with("W: "), "{text}");
        assert!(text.contains("\nH: "), "{text}");
        let off = ToolContext { measurement_labels: false, ..cx(&d, &s, &p) };
        let mut quiet = GraphTool::new("columnGraph");
        drag(&mut quiet, &off);
        assert_eq!(measure_count(&quiet, &off), 0);
    }

    #[test]
    fn create_covers_both() {
        assert_eq!(create("flare").unwrap().id(), "flare");
        assert_eq!(create("reshape").unwrap().id(), "reshape");
        assert_eq!(create("shaper").unwrap().id(), "shaper");
        assert!(create("pen").is_none());
        for g in
            ["columnGraph", "stackedColumnGraph", "barGraph", "stackedBarGraph", "lineGraph", "areaGraph", "scatterGraph", "pieGraph", "radarGraph"]
        {
            assert_eq!(create(g).unwrap().id(), g);
        }
    }
}
