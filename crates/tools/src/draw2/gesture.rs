//! Freehand gesture tools: Pencil (N), Paintbrush (B), Blob Brush (Shift+B), Eraser (Shift+E),
//! Knife, Smooth, Path Eraser and Join.
//!
//! Pencil and Paintbrush preview the fitted path live (`path.freehand` re-applied on the snapshot)
//! and commit on release; Alt on release (or ending near the start) closes the path, and starting
//! near an end of a selected open path continues it. The Pencil's new path starts and ends on
//! Smart Guides ([`DrawSnap`]: an anchor, a path, in line with the art), hovering too; the points
//! between follow the hand. The Paintbrush records the pen pressure of each point (a stroke a pen
//! pressed less than fully on sends `[x, y, pressure]` points) for pressure-sensitive brushes. The
//! other tools collect the drag polyline (shown as an overlay) and run
//! one command on release.

use serde_json::{Value, json};
use vectorcraft_doc::NodeId;
use vectorcraft_geom::Point;

use super::{FEEDBACK, points_json, polyline};
use crate::guides::DrawSnap;
use crate::{Action, Cursor, Mods, Overlay, PointerEvent, PointerKind, Tool, ToolContext, ToolKey};

pub struct GestureTool {
    id: &'static str,
    points: Vec<Point>,
    /// The pen pressure of each of `points` (the Paintbrush only).
    pressures: Vec<f32>,
    active: bool,
    began: bool,
    /// Pencil/Paintbrush continuing a selected open path: (path, continue from its start).
    extend: Option<(NodeId, bool)>,
    /// Fit tolerance in points (Pencil/Paintbrush/Smooth "Fidelity").
    pub fidelity: f64,
    /// Brush / eraser diameter in points.
    pub size: f64,
    /// Fill new pencil/brush strokes.
    pub fill: bool,
    /// "Edit selected paths within N pixels".
    pub edit_within: f64,
    /// "Close paths when ends are within N pixels".
    pub close_within: f64,
    /// Smart Guides for the Pencil's ends.
    snap: DrawSnap,
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
            points: vec![],
            pressures: vec![],
            active: false,
            began: false,
            extend: None,
            fidelity,
            size: 10.0,
            fill: false,
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

    /// Add a stroke sample.
    fn push(&mut self, p: Point, pressure: f32) {
        self.points.push(p);
        if self.id == "paintbrush" {
            self.pressures.push(pressure);
        }
    }

    /// Forget the stroke.
    fn clear(&mut self) {
        self.points.clear();
        self.pressures.clear();
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

    /// The selected open path whose end is near `p`.
    fn find_extend(&self, cx: &ToolContext, p: Point) -> Option<(NodeId, bool)> {
        let tol = cx.tol(self.edit_within);
        for id in &cx.selection.objects {
            let Some(pd) = cx.doc.node(*id).and_then(|n| n.path_data()) else { continue };
            if !cx.doc.is_editable(*id) {
                continue;
            }
            let Some(sp) = pd.subpaths.iter().rev().find(|s| !s.closed && !s.anchors.is_empty()) else { continue };
            let (first, last) = (sp.anchors[0].p, sp.anchors[sp.anchors.len() - 1].p);
            if p.distance(last) <= tol {
                return Some((*id, false));
            }
            if p.distance(first) <= tol {
                return Some((*id, true));
            }
        }
        None
    }

    /// Parameters of `path.freehand` for the current stroke.
    pub fn freehand_params(&self, closed: bool) -> Value {
        let mut v = json!({
            "points": self.stroke_json(),
            "fidelity": self.fidelity,
            "closed": closed,
            "style": if self.id == "paintbrush" { "brush" } else { "pencil" },
            "fill": self.fill,
        });
        if let Some((id, start)) = self.extend {
            v["extend"] = json!({"id": id.0, "end": if start { "start" } else { "end" }});
        }
        v
    }

    fn finish(&mut self, cx: &ToolContext, m: Mods) -> Vec<Action> {
        let pts = &self.points;
        let has_sel = !cx.selection.is_empty();
        let tol = cx.tol(self.edit_within);
        match self.id {
            "pencil" | "paintbrush" => {
                if !self.began {
                    return vec![];
                }
                let near = pts.len() > 2 && pts[0].distance(pts[pts.len() - 1]) <= cx.tol(self.close_within);
                let closed = m.alt || (self.extend.is_none() && near);
                vec![Action::Preview("path.freehand".into(), self.freehand_params(closed)), Action::Commit]
            }
            "blobBrush" => vec![Action::Exec("path.blob".into(), json!({"points": points_json(pts), "size": self.size}))],
            "eraser" => vec![Action::Exec("path.eraseRegion".into(), json!({"points": points_json(pts), "size": self.size}))],
            "knife" if pts.len() >= 2 => vec![Action::Exec("path.knife".into(), json!({"points": points_json(pts)}))],
            "smooth" if has_sel && pts.len() >= 2 => {
                vec![Action::Exec("path.smoothRegion".into(), json!({"points": points_json(pts), "radius": tol, "fidelity": self.fidelity}))]
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
                self.extend = if self.draws_path() { self.find_extend(cx, p) } else { None };
                // A path continued starts at its end; a new one where it snaps.
                let start = if self.snaps() && self.extend.is_none() { self.snap.press(cx, p, &[], None) } else { p };
                self.clear();
                self.push(start, ev.pressure);
                vec![]
            }
            PointerKind::Drag => {
                if !self.active {
                    return vec![];
                }
                if self.snaps() {
                    // Where the stroke would end: shown, taken on release.
                    self.snap.drag(cx, p, None);
                }
                if self.points.last().is_some_and(|l| l.distance(p) < cx.tol(1.0)) {
                    return vec![];
                }
                self.push(p, ev.pressure);
                if !self.draws_path() {
                    return vec![];
                }
                let mut out = vec![];
                if !self.began {
                    self.began = true;
                    out.push(Action::Begin(self.label().into()));
                }
                out.push(Action::Preview("path.freehand".into(), self.freehand_params(false)));
                out
            }
            PointerKind::Up => {
                if !self.active {
                    return vec![];
                }
                let end = if self.snaps() { self.snap.drag(cx, p, None) } else { p };
                self.snap.clear();
                if self.points.last().is_some_and(|l| l.distance(p) >= cx.tol(1.0)) {
                    self.push(end, ev.pressure);
                } else if self.snaps()
                    && self.points.len() > 1
                    && let Some(last) = self.points.last_mut()
                {
                    // The last step was where the button went up: it moves where that snaps.
                    *last = end;
                }
                let out = self.finish(cx, ev.mods);
                self.active = false;
                self.began = false;
                self.clear();
                self.extend = None;
                out
            }
            _ => vec![],
        }
    }
    fn key(&mut self, _cx: &ToolContext, key: ToolKey, _m: Mods) -> Vec<Action> {
        match key {
            ToolKey::Escape if self.active => {
                let began = self.began;
                self.snap.clear();
                self.active = false;
                self.began = false;
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
        self.snap.clear();
        self.active = false;
        self.began = false;
        self.clear();
        if began { vec![Action::Commit] } else { vec![] }
    }
    fn overlays(&self, cx: &ToolContext) -> Vec<Overlay> {
        let mut o = self.snap.guides().to_vec();
        if !self.active || self.points.len() < 2 || self.began {
            return o;
        }
        let width = match self.id {
            "blobBrush" | "eraser" => (self.size * cx.zoom) as f32,
            _ => 1.0,
        };
        let color = if self.id == "eraser" { [0x80, 0x80, 0x80] } else { FEEDBACK };
        o.push(Overlay::Path { path: polyline(&self.points), color, width, dashed: self.id == "knife" });
        o
    }
    fn cursor(&self, _cx: &ToolContext, _p: Point, _m: Mods) -> Cursor {
        Cursor::Crosshair
    }
    fn options(&self) -> Value {
        match self.id {
            "pencil" | "paintbrush" => {
                json!({"fidelity": self.fidelity, "fill": self.fill, "editWithin": self.edit_within, "closeWithin": self.close_within})
            }
            "smooth" => json!({"fidelity": self.fidelity}),
            "blobBrush" | "eraser" => json!({"size": self.size}),
            _ => Value::Null,
        }
    }
    fn set_option(&mut self, key: &str, v: &Value) {
        match key {
            "fidelity" => self.fidelity = v.as_f64().unwrap_or(self.fidelity).clamp(0.05, 100.0),
            "size" => self.size = v.as_f64().unwrap_or(self.size).clamp(0.1, 1000.0),
            "fill" => self.fill = v.as_bool().unwrap_or(self.fill),
            "editWithin" => self.edit_within = v.as_f64().unwrap_or(self.edit_within).clamp(0.0, 100.0),
            "closeWithin" => self.close_within = v.as_f64().unwrap_or(self.close_within).clamp(0.0, 100.0),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::*;
    use vectorcraft_doc::Selection;

    fn drag(t: &mut GestureTool, cx: &ToolContext, pts: &[(f64, f64)], m: Mods) -> Vec<Vec<Action>> {
        let mut v = vec![t.pointer(cx, &PointerEvent::new(PointerKind::Down, pts[0].0, pts[0].1))];
        for &(x, y) in &pts[1..] {
            v.push(t.pointer(cx, &PointerEvent::new(PointerKind::Drag, x, y)));
        }
        let l = pts[pts.len() - 1];
        v.push(t.pointer(cx, &PointerEvent::new(PointerKind::Up, l.0, l.1).with_mods(m)));
        v
    }

    /// The Pencil's new path starts and ends on Smart Guides (#506), the points between where
    /// the hand went.
    #[test]
    fn pencil_ends_snap_to_smart_guides() {
        let (d, _) = doc_with_rect();
        let s = Selection::default();
        let p = paint();
        let cx = cx(&d, &s, &p);
        let mut t = GestureTool::new("pencil");
        let acts = drag(&mut t, &cx, &[(102.0, 98.0), (151.0, 300.0), (250.0, 320.0), (198.0, 203.0)], Mods::default());
        let Some(Action::Preview(_, v)) = acts.last().and_then(|a| a.first()) else { panic!("{acts:?}") };
        assert_eq!(v["points"], json!([[100.0, 100.0], [151.0, 300.0], [250.0, 320.0], [200.0, 200.0]]));
        // The Paintbrush follows the hand all the way.
        let mut t = GestureTool::new("paintbrush");
        let acts = drag(&mut t, &cx, &[(102.0, 98.0), (151.0, 300.0)], Mods::default());
        let Some(Action::Preview(_, v)) = acts.last().and_then(|a| a.first()) else { panic!("{acts:?}") };
        assert_eq!(v["points"][0], json!([102.0, 98.0]));
    }

    /// #852: the Paintbrush sends each point's pen pressure; a mouse (always fully pressed) and
    /// the Pencil send plain points.
    #[test]
    fn paintbrush_sends_pen_pressure() {
        let (d, _) = doc_with_rect();
        let s = Selection::default();
        let p = paint();
        let cx = cx(&d, &s, &p);
        let stroke = |id: &str, pressures: [f32; 3]| {
            let mut t = GestureTool::new(id);
            let kinds = [PointerKind::Down, PointerKind::Drag, PointerKind::Up];
            let mut last = vec![];
            for ((k, x), f) in kinds.into_iter().zip([0.0, 40.0, 80.0]).zip(pressures) {
                last = t.pointer(&cx, &PointerEvent { pressure: f, ..PointerEvent::new(k, x, 0.0) });
            }
            let Some(Action::Preview(_, v)) = last.first() else { panic!("{last:?}") };
            v["points"].clone()
        };
        assert_eq!(stroke("paintbrush", [0.25, 0.5, 1.0]), json!([[0.0, 0.0, 0.25], [40.0, 0.0, 0.5], [80.0, 0.0, 1.0]]));
        assert_eq!(stroke("paintbrush", [1.0; 3]), json!([[0.0, 0.0], [40.0, 0.0], [80.0, 0.0]]));
        assert_eq!(stroke("pencil", [0.25, 0.5, 1.0]), json!([[0.0, 0.0], [40.0, 0.0], [80.0, 0.0]]));
    }

    #[test]
    fn pencil_previews_and_commits() {
        let (d, _) = doc_with_rect();
        let s = Selection::default();
        let p = paint();
        let cx = cx(&d, &s, &p);
        let mut t = GestureTool::new("pencil");
        let acts = drag(&mut t, &cx, &[(0.0, 0.0), (10.0, 5.0), (20.0, 0.0)], Mods::default());
        assert!(acts[0].is_empty());
        assert_eq!(acts[1][0], Action::Begin("Pencil".into()));
        assert!(matches!(&acts[1][1], Action::Preview(c, v) if c == "path.freehand" && v["points"].as_array().unwrap().len() == 2));
        let last = acts.last().unwrap();
        assert!(matches!(&last[0], Action::Preview(_, v) if v["closed"] == false && v["style"] == "pencil"));
        assert_eq!(last[1], Action::Commit);
        assert!(!t.busy());
    }

    #[test]
    fn pencil_alt_closes_and_continues_selected_path() {
        let (d, id) = doc_with_rect();
        let s = Selection::default();
        let p = paint();
        let cx = cx(&d, &s, &p);
        let mut t = GestureTool::new("paintbrush");
        let acts = drag(&mut t, &cx, &[(0.0, 0.0), (10.0, 5.0), (20.0, 0.0)], Mods { alt: true, ..Default::default() });
        assert!(matches!(&acts.last().unwrap()[0], Action::Preview(_, v) if v["closed"] == true && v["style"] == "brush"));
        // A selected closed rect is not continued; an open path would be (see engine tests).
        let mut s2 = Selection::default();
        s2.set([id]);
        let cx2 = crate::testutil::cx(&d, &s2, &p);
        let acts = drag(&mut t, &cx2, &[(100.0, 100.0), (90.0, 90.0)], Mods::default());
        assert!(matches!(&acts[1][1], Action::Preview(_, v) if v.get("extend").is_none()));
    }

    #[test]
    fn eraser_and_knife_exec_on_release() {
        let (d, _) = doc_with_rect();
        let s = Selection::default();
        let p = paint();
        let cx = cx(&d, &s, &p);
        let mut t = GestureTool::new("eraser");
        let acts = drag(&mut t, &cx, &[(50.0, 150.0), (250.0, 150.0)], Mods::default());
        assert!(acts[1].is_empty());
        assert!(matches!(&acts[2][0], Action::Exec(c, v) if c == "path.eraseRegion" && v["size"] == 10.0));
        let mut k = GestureTool::new("knife");
        k.pointer(&cx, &PointerEvent::new(PointerKind::Down, 50.0, 150.0));
        k.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 150.0, 150.0));
        assert_eq!(k.overlays(&cx).len(), 1);
        let a = k.pointer(&cx, &PointerEvent::new(PointerKind::Up, 250.0, 150.0));
        assert!(matches!(&a[0], Action::Exec(c, v) if c == "path.knife" && v["points"].as_array().unwrap().len() == 3));
    }

    #[test]
    fn smooth_needs_selection_and_brackets_resize() {
        let (d, _) = doc_with_rect();
        let s = Selection::default();
        let p = paint();
        let cx = cx(&d, &s, &p);
        let mut t = GestureTool::new("smooth");
        let acts = drag(&mut t, &cx, &[(50.0, 150.0), (250.0, 150.0)], Mods::default());
        assert!(acts.iter().all(|a| a.is_empty()));
        let mut b = GestureTool::new("blobBrush");
        b.key(&cx, ToolKey::BracketRight, Mods::default());
        assert_eq!(b.options()["size"], 11.0);
    }
}
