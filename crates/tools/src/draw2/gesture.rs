//! Freehand gesture tools: Pencil (N), Paintbrush (B), Blob Brush (Shift+B), Eraser (Shift+E),
//! Knife, Smooth, Path Eraser and Join.
//!
//! Pencil and Paintbrush preview the fitted path live (`path.freehand` re-applied on the snapshot)
//! and commit on release; with Live curve fitting off, the Pencil's preview joins the stroke's
//! points with straight segments and the release fits them; Alt on release (or ending near the start) closes the path, and starting
//! near an end of a selected open path continues it. Shift held at the press draws a straight
//! Pencil line at 45° steps from the Constrain Angle instead, and Alt held at the press (with Alt
//! key toggles to Smooth Tool off) draws a straight line at any angle. The Pencil's new path starts and ends on
//! Smart Guides ([`DrawSnap`]: an anchor, a path, in line with the art), hovering too; the points
//! between follow the hand. The Paintbrush records the pen pressure of each point (a stroke a pen
//! pressed less than fully on sends `[x, y, pressure]` points) for pressure-sensitive brushes. The
//! other tools collect the drag polyline (shown as an overlay) and run
//! one command on release. With Shift held, the Blob Brush's points stay on the horizontal or
//! vertical line through the press point. Alt held at the press, or before the Blob Brush's drag
//! has moved, switches the Blob Brush to the Smooth tool: the drag smooths the selected paths.

use serde_json::{Value, json};
use vectorcraft_doc::{NodeId, NodeKind};
use vectorcraft_geom::Point;

use super::{FEEDBACK, eraser_area, points_json, polyline};
use crate::guides::DrawSnap;
use crate::{Action, Cursor, Mods, Overlay, PointerEvent, PointerKind, Tool, ToolContext, ToolKey};

/// How far from its drag the Smooth tool smooths, in screen pixels. The Pencil uses the same
/// radius while Alt switches it to the Smooth tool.
const SMOOTH_RADIUS: f64 = 12.0;

/// The finest Fidelity (screen pixels) a Pencil or Paintbrush stroke is fitted at while it is drawn:
/// a fit's cost grows as its Fidelity shrinks (at 0.01 pt a long pen stroke lagged, #4565), and
/// a preview within half a pixel of the stroke looks the same. The release fits it at its Fidelity.
const PREVIEW_FIDELITY_PX: f64 = 0.5;

pub struct GestureTool {
    /// How many points the last preview of the stroke had ([`crate::previews_at`]); 0 when it was
    /// fitted coarser than the stroke kept ([`PREVIEW_FIDELITY_PX`]).
    previewed: usize,
    id: &'static str,
    points: Vec<Point>,
    /// The pen pressure of each of `points` (the Paintbrush only).
    pressures: Vec<f32>,
    active: bool,
    began: bool,
    /// A release where the pointer last dragged to would close the stroke because it ends near
    /// its start (Close Paths When Ends Are Within): the cursor shows the closing ring.
    near_start: bool,
    /// Pencil/Paintbrush continuing a selected open path: (path, continue from its start).
    extend: Option<(NodeId, bool)>,
    /// A Pencil stroke continuing a selected open path: the other paths selected at the press,
    /// whose open ends it connects to.
    connect_to: Vec<NodeId>,
    /// Pencil/Paintbrush starting on a selected path away from its ends: (path, Edit Selected
    /// Paths' distance in points); the stroke redraws the section it ends on (#2850).
    redraw: Option<(NodeId, f64)>,
    /// Fit tolerance in points (Pencil/Paintbrush/Smooth/Blob Brush "Fidelity").
    pub fidelity: f64,
    /// Brush / eraser diameter in points (the Eraser's across its tip's long axis).
    pub size: f64,
    /// The angle of the Eraser tip's long axis, in degrees counter-clockwise from horizontal.
    pub angle: f64,
    /// The Eraser tip's short axis in percent of its long one (100: round).
    pub roundness: f64,
    /// Fill new pencil/brush strokes.
    pub fill: bool,
    /// "Keep selected": select the path a Pencil, Paintbrush or Blob Brush stroke draws, continues
    /// or merges into (on for the Pencil and Paintbrush, off for the Blob Brush).
    pub keep_selected: bool,
    /// "Alt key toggles to Smooth Tool": holding Alt with the Pencil tool smooths selected paths
    /// instead of closing the path (Pencil only, off by default).
    pub alt_smooth: bool,
    /// "Live preview" (Pencil only, on by default): the stroke shows with its fill and stroke as it
    /// is drawn. Off, only a thin line follows the pen while drawing, and the path takes its fill
    /// and stroke on release.
    pub live_preview: bool,
    /// "Live curve fitting": the Pencil's preview fits the stroke while it is drawn; off, the
    /// preview shows the points joined by straight segments and the release fits them (Pencil
    /// only, on by default).
    pub live_fit: bool,
    /// Active drag is smoothing with the Pencil or the Blob Brush (Alt held).
    smoothing: bool,
    /// The pointer has moved from the press point, by any distance (a move too short to add a
    /// point counts).
    moved: bool,
    /// Active Pencil drag draws a straight line (Shift or Alt held at the press): the stroke is
    /// the press point and the pointer.
    straight: bool,
    /// The straight line is Alt's: the pointer is not constrained to 45° steps, and the line's end
    /// snaps to Smart Guides as a stroke's end does.
    unconstrained: bool,
    /// Axis selected during the current uninterrupted Shift interval, in degrees.
    shift_line: Option<f64>,
    /// "Merge only with selection": a Blob Brush stroke merges only with selected paths.
    pub merge_only_selection: bool,
    /// "Edit selected paths within N pixels".
    pub edit_within: f64,
    /// "Close paths when ends are within N pixels".
    pub close_within: f64,
    /// Smart Guides for the Pencil's ends.
    snap: DrawSnap,
}

/// The paths a stroke can redraw for selected object `id`: the path itself, or a compound path's
/// member paths.
fn redraw_targets(cx: &ToolContext, id: NodeId) -> Vec<NodeId> {
    let Some(n) = cx.doc.node(id) else { return vec![] };
    match &n.kind {
        NodeKind::Path { .. } => vec![id],
        NodeKind::Compound { children, .. } => children.iter().filter(|c| c.path_data().is_some()).map(|c| c.id).collect(),
        _ => vec![],
    }
}

impl GestureTool {
    pub fn new(id: &str) -> Self {
        let id: &'static str = match id {
            "paintbrush" => "paintbrush",
            "blobBrush" => "blobBrush",
            "eraser" => "eraser",
            "knife" => "knife",
            "smooth" => "smooth",
            "pathEraser" => "pathEraser",
            "join" => "join",
            _ => "pencil",
        };
        let fidelity = if id == "smooth" { 2.5 } else { 1.5 };
        Self {
            id,
            previewed: 0,
            points: vec![],
            pressures: vec![],
            active: false,
            began: false,
            near_start: false,
            extend: None,
            connect_to: vec![],
            redraw: None,
            fidelity,
            size: 10.0,
            angle: 0.0,
            roundness: 100.0,
            fill: false,
            keep_selected: id != "blobBrush",
            // Off, as Pencil Tool Options ships it: Alt keeps drawing a closed path until it is on.
            alt_smooth: false,
            live_preview: true,
            live_fit: true,
            smoothing: false,
            moved: false,
            straight: false,
            unconstrained: false,
            shift_line: None,
            merge_only_selection: false,
            edit_within: 12.0,
            close_within: 15.0,
            snap: DrawSnap::default(),
        }
    }

    /// Do the ends of its strokes snap (the Pencil)?
    fn snaps(&self) -> bool {
        self.id == "pencil"
    }

    fn draws_path(&self) -> bool {
        matches!(self.id, "pencil" | "paintbrush")
    }

    /// Does a stroke show as the path it draws while it is drawn? Not with the Pencil's Live preview
    /// off: then the drag shows as a thin line (an overlay) until the release.
    fn previews(&self) -> bool {
        self.id != "pencil" || self.live_preview
    }

    /// Add a stroke sample.
    fn push(&mut self, p: Point, pressure: f32) {
        self.points.push(p);
        if self.id == "paintbrush" {
            self.pressures.push(pressure);
        }
    }

    /// Would a press with `m` held draw a straight Pencil line? Shift or Alt held, unless Alt lends
    /// the Smooth tool: the rule `PointerKind::Down` sets `straight` by.
    fn straight_at_press(&self, m: Mods) -> bool {
        self.id == "pencil" && !(self.alt_smooth && m.alt) && (m.shift || m.alt)
    }

    /// Would Alt held now switch the Blob Brush to the Smooth tool? It does before a press, and
    /// during a drag that has not moved from the press point yet.
    fn blob_alt_smooths(&self, m: Mods) -> bool {
        self.id == "blobBrush" && m.alt && !self.moved
    }

    /// Constrain the Pencil line or the current Blob Brush sample. Blob Brush chooses an
    /// axis once per uninterrupted Shift interval; samples already drawn never move.
    /// A smoothing drag follows the pointer without a Shift constraint.
    fn constrained(&mut self, cx: &ToolContext, p: Point, m: Mods) -> Point {
        if self.straight && self.unconstrained {
            return p;
        }
        if self.straight
            && let Some(&start) = self.points.first()
        {
            return start + vectorcraft_geom::constrain_angle_from(p - start, 45.0, cx.constrain_angle);
        }
        let Some(&start) = self.points.first().filter(|_| self.id == "blobBrush" && m.shift && !self.smoothing) else {
            self.shift_line = None;
            return p;
        };
        if self.shift_line.is_none() {
            // Raw press-to-pointer displacement in logical screen pixels along the constraint
            // axes. Zoom changes the document distance, not the activation tolerance.
            let v = p - start;
            let v = vectorcraft_geom::constrain_axes(cx.constrain_angle).map_or(Point::new(v.x, v.y), |axes| axes.inverse() * Point::new(v.x, v.y));
            let x = v.x.abs() * cx.zoom;
            let y = v.y.abs() * cx.zoom;
            self.shift_line = if x >= 1.0 && y < 0.5 {
                Some(cx.constrain_angle)
            } else if y >= 1.0 && x < 0.5 {
                Some(cx.constrain_angle + 90.0)
            } else {
                // No unconstrained trail while pending, including an initially diagonal drag.
                return self.points.last().copied().unwrap_or(start);
            };
        }
        start + vectorcraft_geom::constrain_angle_from(p - start, 180.0, self.shift_line.unwrap_or(cx.constrain_angle))
    }

    /// Forget the stroke.
    fn clear(&mut self) {
        self.moved = false;
        self.points.clear();
        self.pressures.clear();
        self.previewed = 0;
        self.shift_line = None;
    }

    /// The stroke's points, each `[x, y, pressure]` when a pen pressed less than fully anywhere
    /// along it (a mouse always presses fully).
    fn stroke_json(&self) -> Value {
        if self.pressures.len() == self.points.len() && self.pressures.iter().any(|p| *p < 1.0) {
            Value::Array(self.points.iter().zip(&self.pressures).map(|(p, f)| json!([p.x, p.y, (f64::from(*f) * 1000.0).round() / 1000.0])).collect())
        } else {
            points_json(&self.points)
        }
    }

    fn label(&self) -> &'static str {
        match self.id {
            "paintbrush" => "Paintbrush",
            "blobBrush" => "Blob Brush",
            "eraser" => "Eraser",
            "knife" => "Knife",
            "smooth" => "Smooth",
            "pathEraser" => "Path Eraser",
            "join" => "Join",
            _ => "Pencil",
        }
    }

    /// The selected open path whose end is near `p` (Edit Selected Paths, off at 0 pixels), and
    /// that end.
    fn find_extend(&self, cx: &ToolContext, p: Point) -> Option<(NodeId, bool, Point)> {
        self.find_end(cx, p, &cx.selection.objects)
    }

    /// [`Self::find_extend`] among `ids`.
    fn find_end(&self, cx: &ToolContext, p: Point, ids: &[NodeId]) -> Option<(NodeId, bool, Point)> {
        if self.edit_within <= 0.0 {
            return None;
        }
        let tol = cx.tol(self.edit_within);
        let editable = cx.doc.editable();
        for id in ids {
            let Some(pd) = cx.doc.node(*id).and_then(|n| n.path_data()) else { continue };
            if !editable(*id) {
                continue;
            }
            let Some(sp) = pd.subpaths.iter().rev().find(|s| !s.closed && !s.anchors.is_empty()) else { continue };
            let (first, last) = (sp.anchors[0].p, sp.anchors[sp.anchors.len() - 1].p);
            if p.distance(last) <= tol {
                return Some((*id, false, last));
            }
            if p.distance(first) <= tol {
                return Some((*id, true, first));
            }
        }
        None
    }

    /// The selected path whose outline is near `p` (Edit Selected Paths, off at 0 pixels).
    fn find_redraw(&self, cx: &ToolContext, p: Point) -> Option<(NodeId, f64)> {
        let tol = cx.tol(self.edit_within);
        if self.edit_within <= 0.0 || !tol.is_finite() {
            return None;
        }
        // A selected compound path (a shape with a hole) redraws the member path the stroke starts
        // on, the nearest of them: a compound path has no path of its own.
        cx.selection.objects.iter().find_map(|id| {
            redraw_targets(cx, *id)
                .into_iter()
                .filter(|m| cx.doc.is_editable(*m))
                .filter_map(|m| Some((m, cx.doc.node(m)?.path_data()?.nearest(p)?.4)))
                .filter(|(_, d)| *d <= tol)
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(m, _)| (m, tol))
        })
    }

    /// Does a stroke that ends at `end` end near its start (Close Paths When Ends Are Within, off
    /// at 0 pixels)?
    fn ends_near_start(&self, cx: &ToolContext, end: Point) -> bool {
        self.close_within > 0.0 && self.points.len() > 2 && self.points[0].distance(end) <= cx.tol(self.close_within)
    }

    /// Does a release now, with the stroke ending at `end`, close a freehand stroke because it ends
    /// near its start (`finish`)? A stroke that redraws a selected path closes only when the engine
    /// finds it does not end on that path, so it counts as not closing.
    fn release_closes_near_start(&self, cx: &ToolContext, end: Point) -> bool {
        self.draws_path() && !self.smoothing && !self.straight && self.extend.is_none() && self.redraw.is_none() && self.ends_near_start(cx, end)
    }

    /// Parameters of `path.freehand` for the current stroke.
    pub fn freehand_params(&self, closed: bool) -> Value {
        let mut v = json!({
            "points": self.stroke_json(),
            "fidelity": self.fidelity,
            "closed": closed,
            "style": if self.id == "paintbrush" { "brush" } else { "pencil" },
            "fill": self.fill,
            "keepSelected": self.keep_selected,
        });
        if let Some((id, start)) = self.extend {
            v["extend"] = json!({"id": id.0, "end": if start { "start" } else { "end" }});
        } else if let Some((id, within)) = self.redraw {
            v["redraw"] = json!({"id": id.0, "within": within});
        }
        v
    }

    fn finish(&mut self, cx: &ToolContext, m: Mods) -> Vec<Action> {
        let pts = &self.points;
        let has_sel = !cx.selection.is_empty();
        let tol = cx.tol(self.edit_within);
        let radius = cx.tol(SMOOTH_RADIUS);
        if self.smoothing {
            if has_sel && pts.len() >= 2 {
                return vec![Action::Exec(
                    "path.smoothRegion".into(),
                    json!({"points": points_json(pts), "radius": radius, "fidelity": self.fidelity}),
                )];
            }
            return vec![];
        }
        match self.id {
            "pencil" | "paintbrush" => {
                if !self.began {
                    return vec![];
                }
                // Close Paths When Ends Are Within is off at 0 pixels.
                let near = pts.last().is_some_and(|&end| self.ends_near_start(cx, end));
                // A straight line has two points: closing it would draw it back over itself.
                let closed = !self.straight && ((m.alt && !self.alt_smooth) || (self.extend.is_none() && near));
                let mut params = self.freehand_params(closed);
                // A Pencil stroke that continues a selected path and ends on an open end of another
                // path selected at the press connects the two.
                let connect = pts.last().and_then(|&end| self.find_end(cx, end, &self.connect_to));
                if let Some((other, at_start, _)) = connect {
                    params["connect"] = json!({"id": other.0, "end": if at_start { "start" } else { "end" }});
                }
                vec![Action::Preview("path.freehand".into(), params), Action::Commit]
            }
            "blobBrush" => vec![Action::Exec(
                "path.blob".into(),
                json!({
                    "points": points_json(pts),
                    "size": self.size,
                    "fidelity": self.fidelity,
                    "keepSelected": self.keep_selected,
                    "mergeOnlySelection": self.merge_only_selection
                }),
            )],
            "eraser" => vec![Action::Exec(
                "path.eraseRegion".into(),
                json!({"points": points_json(pts), "size": self.size, "angle": self.angle, "roundness": self.roundness}),
            )],
            "knife" if pts.len() >= 2 => vec![Action::Exec("path.knife".into(), json!({"points": points_json(pts)}))],
            "smooth" if has_sel && pts.len() >= 2 => {
                vec![Action::Exec("path.smoothRegion".into(), json!({"points": points_json(pts), "radius": radius, "fidelity": self.fidelity}))]
            }
            "pathEraser" if has_sel => vec![Action::Exec("path.eraseSegments".into(), json!({"points": points_json(pts), "width": cx.tol(8.0)}))],
            "join" if pts.len() >= 2 => vec![Action::Exec("path.joinScrub".into(), json!({"points": points_json(pts), "tolerance": tol}))],
            _ => vec![],
        }
    }
}

impl Tool for GestureTool {
    fn id(&self) -> &'static str {
        self.id
    }
    fn busy(&self) -> bool {
        self.active
    }
    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        let p = ev.pos;
        match ev.kind {
            PointerKind::Move if self.snaps() => {
                self.snap.hover(cx, p, &[], None);
                vec![]
            }
            PointerKind::Down => {
                self.active = true;
                self.began = false;
                self.near_start = false;
                // Alt switches the Pencil to the Smooth tool while Alt key toggles to Smooth Tool is on, and always
                // switches the Blob Brush.
                self.smoothing = ev.mods.alt && (self.id == "blobBrush" || (self.id == "pencil" && self.alt_smooth));
                // Alt draws a straight line only while it doesn't lend the Smooth tool; with Shift
                // too, the line keeps Shift's 45° steps.
                self.straight = self.id == "pencil" && !self.smoothing && (ev.mods.shift || ev.mods.alt);
                self.unconstrained = self.straight && !ev.mods.shift;
                let extend = if self.draws_path() && !self.smoothing { self.find_extend(cx, p) } else { None };
                self.extend = extend.map(|(id, at_start, _)| (id, at_start));
                // The stroke's previews select the path it continues, so the others are kept now.
                self.connect_to = match self.extend {
                    Some((id, _)) if self.id == "pencil" => cx.selection.objects.iter().copied().filter(|o| *o != id).collect(),
                    _ => vec![],
                };
                self.redraw = if self.draws_path() && !self.smoothing && self.extend.is_none() { self.find_redraw(cx, p) } else { None };
                // A path continued starts at its end, one redrawn on it; a new one where it snaps.
                // A straight line is constrained from the end it continues, so it starts there.
                let start = if let Some((_, _, end)) = extend.filter(|_| self.straight) {
                    end
                } else if self.snaps() && !self.smoothing && self.extend.is_none() && self.redraw.is_none() {
                    self.snap.press(cx, p, &[], None)
                } else {
                    p
                };
                self.clear();
                self.push(start, ev.pressure);
                vec![]
            }
            PointerKind::Drag => {
                if !self.active {
                    return vec![];
                }
                if (!self.began && !self.straight && self.id == "pencil" && self.alt_smooth && ev.mods.alt) || self.blob_alt_smooths(ev.mods) {
                    self.smoothing = true;
                }
                self.moved |= self.points.first().is_some_and(|start| p.distance(*start) > vectorcraft_geom::EPS);
                let p = self.constrained(cx, p, ev.mods);
                let mut end = p;
                if self.snaps() && !self.smoothing && !self.straight {
                    // Where the stroke would end: shown, taken on release.
                    end = self.snap.drag(cx, p, None);
                }
                let p = if self.straight && self.unconstrained && self.snaps() {
                    // Alt's line ends where a stroke would: shown, taken on release.
                    self.snap.drag(cx, p, None)
                } else {
                    p
                };
                if self.straight {
                    if !self.unconstrained {
                        // The press's Smart Guides would stay on screen: the line's end doesn't snap.
                        self.snap.clear();
                    }
                    // The line runs from the press point to the pointer, wherever the drag went.
                    if self.points.first().is_some_and(|s| s.distance(p) < cx.tol(1.0)) {
                        // Back at the press point there is no line: the one shown goes.
                        self.points.truncate(1);
                        return if std::mem::take(&mut self.began) { vec![Action::Cancel] } else { vec![] };
                    }
                    self.points.truncate(1);
                } else if self.points.last().is_some_and(|l| l.distance(p) < cx.tol(1.0)) || self.points.len() >= crate::MAX_STROKE_SAMPLES {
                    // A release here keeps the last point unless it snaps (`Up`), so the ring
                    // follows the point `finish` will test.
                    let kept = self.points.last().copied().filter(|l| l.distance(p) < cx.tol(1.0) && !self.snaps());
                    let release_end = kept.unwrap_or(end);
                    self.near_start = self.release_closes_near_start(cx, release_end);
                    return vec![];
                }
                self.push(p, ev.pressure);
                self.near_start = self.release_closes_near_start(cx, end);
                if !self.draws_path() || self.smoothing {
                    return vec![];
                }
                let mut out = vec![];
                if !self.began {
                    self.began = true;
                    out.push(Action::Begin(self.label().into()));
                }
                // With the Pencil's Live preview off a thin line shows instead (`overlays`).
                if self.previews() && crate::previews_at(self.points.len()) {
                    let mut v = self.freehand_params(false);
                    let fidelity = self.fidelity.max(cx.tol(PREVIEW_FIDELITY_PX));
                    v["fidelity"] = json!(fidelity);
                    // A preview fitted coarser than the stroke kept is stale (`deactivate`).
                    self.previewed = if fidelity > self.fidelity { 0 } else { self.points.len() };
                    if !self.live_fit {
                        // Live curve fitting off: the release fits the stroke (`finish`).
                        v["fit"] = json!(false);
                    }
                    out.push(Action::Preview("path.freehand".into(), v));
                }
                out
            }
            PointerKind::Up => {
                if !self.active {
                    return vec![];
                }
                // A release away from the press point counts as movement.
                self.moved |= self.points.first().is_some_and(|start| p.distance(*start) > vectorcraft_geom::EPS);
                // Alt pressed while the pointer stood still at the press point switches the release too.
                if self.blob_alt_smooths(ev.mods) {
                    self.smoothing = true;
                }
                let p = self.constrained(cx, p, ev.mods);
                let end = if self.snaps() && !self.smoothing && (!self.straight || self.unconstrained) { self.snap.drag(cx, p, None) } else { p };
                self.snap.clear();
                let mut cancel = false;
                if self.straight {
                    self.points.truncate(1);
                    if self.points.first().is_some_and(|s| s.distance(end) >= cx.tol(1.0)) {
                        self.push(end, ev.pressure);
                    } else {
                        cancel = self.began;
                    }
                } else if self.points.last().is_some_and(|l| l.distance(p) >= cx.tol(1.0)) {
                    self.push(end, ev.pressure);
                } else if self.snaps()
                    && !self.smoothing
                    && self.points.len() > 1
                    && let Some(last) = self.points.last_mut()
                {
                    // The last step was where the button went up: it moves where that snaps.
                    *last = end;
                }
                let out = if cancel { vec![Action::Cancel] } else { self.finish(cx, ev.mods) };
                self.active = false;
                self.began = false;
                self.near_start = false;
                self.smoothing = false;
                self.straight = false;
                self.unconstrained = false;
                self.clear();
                self.extend = None;
                self.connect_to.clear();
                self.redraw = None;
                out
            }
            _ => vec![],
        }
    }
    /// The freehand tools build their stroke from every position the pointer passed; the Join tool
    /// only needs where its scribble crosses the ends.
    fn wants_samples(&self) -> bool {
        matches!(self.id, "pencil" | "paintbrush" | "blobBrush" | "eraser" | "knife" | "smooth" | "pathEraser")
    }
    /// `[` and `]` are the Blob Brush's and the Eraser's: their size (#1661).
    fn claims_key(&self, _cx: &ToolContext, key: ToolKey) -> bool {
        matches!(key, ToolKey::BracketLeft | ToolKey::BracketRight) && matches!(self.id, "blobBrush" | "eraser")
    }
    fn key(&mut self, _cx: &ToolContext, key: ToolKey, _m: Mods) -> Vec<Action> {
        match key {
            ToolKey::Escape if self.active => {
                let began = self.began;
                self.snap.clear();
                self.active = false;
                self.began = false;
                self.near_start = false;
                self.smoothing = false;
                self.straight = false;
                self.unconstrained = false;
                self.clear();
                if began { vec![Action::Cancel] } else { vec![] }
            }
            ToolKey::BracketLeft if matches!(self.id, "blobBrush" | "eraser") => {
                self.size = (self.size - 1.0).max(1.0);
                vec![]
            }
            ToolKey::BracketRight if matches!(self.id, "blobBrush" | "eraser") => {
                self.size = (self.size + 1.0).min(1000.0);
                vec![]
            }
            _ => vec![],
        }
    }
    fn deactivate(&mut self, _cx: &ToolContext) -> Vec<Action> {
        let began = self.began;
        // Drags past the last preview are in the stroke kept, too, and with Live curve fitting
        // off the stroke kept is fitted.
        let stale = (began && (self.previewed != self.points.len() || !self.live_fit) && self.draws_path() && !self.smoothing)
            .then(|| Action::Preview("path.freehand".into(), self.freehand_params(false)));
        self.snap.clear();
        self.active = false;
        self.near_start = false;
        self.began = false;
        self.smoothing = false;
        self.straight = false;
        self.unconstrained = false;
        self.clear();
        if began { stale.into_iter().chain([Action::Commit]).collect() } else { vec![] }
    }
    fn overlays(&self, cx: &ToolContext) -> Vec<Overlay> {
        let mut o = self.snap.guides().to_vec();
        // A path being drawn shows itself, except with the Pencil's Live preview off.
        if !self.active || self.points.len() < 2 || (self.began && self.previews()) {
            return o;
        }
        // A Blob Brush drag that smooths shows the Smooth tool's thin line.
        let width = match self.id {
            "blobBrush" | "eraser" if !self.smoothing => (self.size * cx.zoom) as f32,
            _ => 1.0,
        };
        let color = if self.id == "eraser" { [0x80, 0x80, 0x80] } else { FEEDBACK };
        // A tip that is not round shows the outline of the area it covers.
        if self.id == "eraser" && self.roundness < 100.0 {
            let area = eraser_area(&self.points, self.size, self.angle, self.roundness);
            o.push(Overlay::Path { path: area.to_bezpath(), color, width: 1.0, dashed: false });
            return o;
        }
        o.push(Overlay::Path { path: polyline(&self.points), color, width, dashed: self.id == "knife" });
        if cx.measurement_labels && self.id == "blobBrush" && !self.smoothing {
            let last = self.points[self.points.len() - 1];
            let length = self.points.windows(2).map(|w| w[0].distance(w[1])).sum();
            o.push(Overlay::Measure { p: last + vectorcraft_geom::Vec2::new(cx.tol(12.0), cx.tol(12.0)), text: format!("L: {}", cx.len(length)) });
        }
        o
    }
    fn cursor(&self, cx: &ToolContext, p: Point, m: Mods) -> Cursor {
        // A drag that smooths, or Alt before a Pencil or Blob Brush stroke has begun to draw (a
        // stroke that has begun keeps drawing with Alt).
        if self.smoothing || (self.id == "pencil" && self.alt_smooth && m.alt && !self.began) || self.blob_alt_smooths(m) {
            return Cursor::Smooth;
        }
        // Before a press that would draw a straight line: over an end of a selected open path,
        // the line continues that path from the end (`PointerKind::Down`).
        if !self.active && self.straight_at_press(m) {
            return if self.find_extend(cx, p).is_some() { Cursor::PencilContinue } else { Cursor::PencilStraight };
        }
        // Alt held at the release closes the path (`finish`).
        // A straight line stays open, so its release has nothing to close, and before a press Alt
        // gives the Pencil a straight line.
        if self.draws_path() && !self.alt_smooth && m.alt && !self.straight && (self.active || self.id != "pencil") {
            return if self.id == "paintbrush" { Cursor::PaintbrushClose } else { Cursor::PencilClose };
        }
        // A release near a freehand stroke's start closes it too (`finish`): the ring shows while
        // the pointer is there.
        if self.draws_path() && self.active && self.near_start {
            return if self.id == "paintbrush" { Cursor::PaintbrushClose } else { Cursor::PencilClose };
        }
        match self.id {
            "paintbrush" => Cursor::Paintbrush,
            "blobBrush" => Cursor::BlobBrush,
            "eraser" => Cursor::Eraser,
            "knife" => Cursor::Knife,
            "smooth" => Cursor::Smooth,
            "pathEraser" => Cursor::PathEraser,
            "join" => Cursor::Join,
            // Before a press that draws a new path the Pencil shows its star; over a selected path
            // that the press continues from an end or redraws (`PointerKind::Down`), it does not.
            _ if !self.active && self.find_extend(cx, p).is_none() && self.find_redraw(cx, p).is_none() => Cursor::PencilNew,
            _ => Cursor::Pencil,
        }
    }
    fn options(&self) -> Value {
        match self.id {
            "pencil" => {
                json!({
                    "fidelity": self.fidelity,
                    "fill": self.fill,
                    "altSmooth": self.alt_smooth,
                    "livePreview": self.live_preview,
                    "liveFit": self.live_fit,
                    "keepSelected": self.keep_selected,
                    "editWithin": self.edit_within,
                    "closeWithin": self.close_within
                })
            }
            "paintbrush" => {
                json!({
                    "fidelity": self.fidelity,
                    "fill": self.fill,
                    "keepSelected": self.keep_selected,
                    "editWithin": self.edit_within,
                    "closeWithin": self.close_within
                })
            }
            "smooth" => json!({"fidelity": self.fidelity}),
            "blobBrush" => {
                json!({"fidelity": self.fidelity, "size": self.size, "keepSelected": self.keep_selected, "mergeOnlySelection": self.merge_only_selection})
            }
            "eraser" => json!({"size": self.size, "angle": self.angle, "roundness": self.roundness}),
            _ => Value::Null,
        }
    }
    fn set_option(&mut self, key: &str, v: &Value) {
        match key {
            "fidelity" => self.fidelity = v.as_f64().unwrap_or(self.fidelity).clamp(super::min_fidelity(self.id), 100.0),
            "size" => self.size = v.as_f64().unwrap_or(self.size).clamp(0.1, 1000.0),
            "angle" => self.angle = v.as_f64().unwrap_or(self.angle).clamp(-180.0, 180.0),
            "roundness" => self.roundness = v.as_f64().unwrap_or(self.roundness).clamp(1.0, 100.0),
            "fill" => self.fill = v.as_bool().unwrap_or(self.fill),
            "altSmooth" => self.alt_smooth = v.as_bool().unwrap_or(self.alt_smooth),
            "livePreview" if self.id == "pencil" => self.live_preview = v.as_bool().unwrap_or(self.live_preview),
            "liveFit" if self.id == "pencil" => self.live_fit = v.as_bool().unwrap_or(self.live_fit),
            "keepSelected" => self.keep_selected = v.as_bool().unwrap_or(self.keep_selected),
            "mergeOnlySelection" => self.merge_only_selection = v.as_bool().unwrap_or(self.merge_only_selection),
            "editWithin" => self.edit_within = v.as_f64().unwrap_or(self.edit_within).clamp(0.0, 100.0),
            "closeWithin" => self.close_within = v.as_f64().unwrap_or(self.close_within).clamp(0.0, 100.0),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests;
