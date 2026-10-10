//! Flare, Reshape, Shaper and Graph tools.
//!
//! Flare: drag sets the centre and its size (click without dragging opens the Flare Tool Options),
//! then a click sets the end point of the rings; the rest comes from the tool's options (the Flare
//! Tool Options, which last). Reshape: drag a point of a selected path; the engine's
//! `path.reshape` adds an anchor there if needed and moves the neighbourhood smoothly.

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
    began: bool,
    /// Ray count changed with ↑/↓ during the gesture (None = the options' count).
    rays: Option<u64>,
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
            began: false,
            rays: None,
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

impl FlareTool {
    /// `shape.flare`'s params: the options, with the dragged centre and diameter, the ray count
    /// ↑/↓ chose and the rings' end point once clicked.
    fn params(&self, c: Point, diameter: f64, end: Option<Point>) -> Value {
        let mut v = flare_params(&self.options(), c);
        v["diameter"] = json!(diameter.max(1.0));
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
                }
                vec![]
            }
            PointerKind::Drag => {
                if let Some((c, _)) = self.drag {
                    self.drag = Some((c, ev.pos));
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
                } else if let Some((c, d)) = self.placed {
                    return vec![Action::Preview("shape.flare".into(), self.params(c, d, Some(ev.pos)))];
                }
                vec![]
            }
            PointerKind::Up => {
                if let Some((c, d)) = self.placed.take() {
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

/// Reshape Tool: drag a point on a selected path.
#[derive(Default)]
pub struct ReshapeTool {
    /// (path, grab point) while dragging.
    grab: Option<(vectorcraft_doc::NodeId, Point)>,
    began: bool,
}

impl ReshapeTool {
    /// The selected path under `p` (Reshape works on selected paths only, like Illustrator).
    fn hit(cx: &ToolContext, p: Point) -> Option<(vectorcraft_doc::NodeId, Point)> {
        let tol = cx.tol(4.0);
        cx.selection.objects.iter().copied().filter(|id| cx.doc.is_editable(*id)).find_map(|id| {
            let pd = cx.doc.node(id)?.path_data()?;
            let (_, _, _, q, d) = pd.nearest(p)?;
            (d <= tol).then_some((id, q))
        })
    }
}

impl Tool for ReshapeTool {
    fn id(&self) -> &'static str {
        "reshape"
    }
    fn busy(&self) -> bool {
        self.grab.is_some()
    }
    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        match ev.kind {
            PointerKind::Down => {
                self.grab = Self::hit(cx, ev.pos);
                self.began = false;
                vec![]
            }
            PointerKind::Drag => {
                let Some((id, at)) = self.grab else { return vec![] };
                let mut d: Vec2 = ev.pos - at;
                if ev.mods.shift {
                    d = vectorcraft_geom::constrain_angle(d, 45.0);
                }
                let mut out = vec![];
                if !self.began {
                    if d.hypot() < cx.tol(2.0) {
                        return out;
                    }
                    self.began = true;
                    out.push(Action::Begin("Reshape".into()));
                }
                out.push(Action::Preview(
                    "path.reshape".into(),
                    json!({ "id": id.0, "x": at.x, "y": at.y, "dx": d.x, "dy": d.y, "tol": cx.tol(4.0) }),
                ));
                out
            }
            PointerKind::Up => {
                self.grab = None;
                if std::mem::take(&mut self.began) { vec![Action::Commit] } else { vec![] }
            }
            _ => vec![],
        }
    }
    fn key(&mut self, _cx: &ToolContext, key: ToolKey, _mods: Mods) -> Vec<Action> {
        if key == ToolKey::Escape && self.grab.take().is_some() && std::mem::take(&mut self.began) {
            return vec![Action::Cancel];
        }
        vec![]
    }
    fn cursor(&self, cx: &ToolContext, p: Point, _mods: Mods) -> Cursor {
        if self.grab.is_some() || Self::hit(cx, p).is_some() { Cursor::Move } else { Cursor::Arrow }
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
                    return vec![Action::Exec("shaper.select".into(), json!({"point": [ev.pos.x, ev.pos.y]}))];
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
        Cursor::Crosshair
    }
}

/// Graph tools: drag the plot rectangle (Shift = square, Alt = from the centre); a click opens the
/// size dialog. A new graph opens the Graph Data dialog.
pub struct GraphTool {
    id: &'static str,
    start: Option<Point>,
    began: bool,
}

impl GraphTool {
    fn new(id: &str) -> Self {
        let id = crate::catalog::tool_info(id).map(|t| t.id).unwrap_or("columnGraph");
        Self { id, start: None, began: false }
    }
    fn kind(&self) -> &'static str {
        vectorcraft_doc::GraphKind::parse(self.id).unwrap_or_default().id()
    }
    fn params(&self, s: Point, p: Point, m: Mods) -> Value {
        let mut d = p - s;
        if m.shift {
            let k = d.x.abs().max(d.y.abs());
            d = Vec2::new(k * d.x.signum(), k * d.y.signum());
        }
        let r = if m.alt { vectorcraft_geom::Rect::from_points(s - d, s + d) } else { vectorcraft_geom::Rect::from_points(s, s + d) };
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
                self.began = false;
                vec![]
            }
            PointerKind::Drag => {
                let Some(s) = self.start else { return vec![] };
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
