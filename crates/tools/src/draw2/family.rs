//! Line Segment family drag tools: Arc, Spiral, Rectangular Grid and Polar Grid.
//!
//! Drag draws (Shift = equal axes / square, Alt = from the centre for arc and grids, Space held
//! moves the shape being drawn); a click without dragging asks the UI for the options dialog. The
//! start point and the dragged corner snap to Smart Guides ([`DrawSnap`]), hovering too.
//! While dragging, ↑/↓ change the spiral's segments, the grid rows or the concentric dividers; ←/→
//! change grid columns / radial dividers.

use serde_json::{Value, json};
use vectorcraft_geom::Point;

use crate::guides::{DrawSnap, Leave, square};
use crate::shape::drag_rect;
use crate::{Action, Cursor, Mods, Overlay, PointerEvent, PointerKind, Tool, ToolContext, ToolKey};

pub struct FamilyTool {
    id: &'static str,
    start: Option<Point>,
    last: Point,
    mods: Mods,
    began: bool,
    pub closed: bool,
    pub decay: f64,
    pub segments: u32,
    pub clockwise: bool,
    /// Rows / concentric dividers.
    pub rows: u32,
    /// Columns / radial dividers.
    pub columns: u32,
    /// The grids' divider skews in percent: rows / concentric, then columns / radial.
    pub skew: [f64; 2],
    /// Rectangular Grid: the outside is one rectangle (else its four sides).
    pub frame: bool,
    /// Polar Grid: the ellipses pair up into compound paths.
    pub compound: bool,
    /// The grids take the current fill.
    pub fill: bool,
    snap: DrawSnap,
}

impl FamilyTool {
    pub fn new(id: &str) -> Self {
        let id: &'static str = match id {
            "spiral" => "spiral",
            "rectangularGrid" => "rectangularGrid",
            "polarGrid" => "polarGrid",
            _ => "arc",
        };
        Self {
            id,
            start: None,
            last: Point::ZERO,
            mods: Mods::default(),
            began: false,
            closed: false,
            decay: 80.0,
            segments: 10,
            clockwise: true,
            rows: 5,
            columns: 5,
            skew: [0.0; 2],
            frame: true,
            compound: false,
            fill: false,
            snap: DrawSnap::default(),
        }
    }

    fn label(&self) -> &'static str {
        match self.id {
            "spiral" => "Spiral",
            "rectangularGrid" => "Rectangular Grid",
            "polarGrid" => "Polar Grid",
            _ => "Arc",
        }
    }

    /// How the dragged point keeps to the start: a spiral's radius goes anywhere, the arc's and
    /// the grids' corner keeps to a diagonal with Shift.
    fn leave(&self, start: Point, m: Mods) -> Option<Leave> {
        (self.id != "spiral").then(|| Leave::diagonal(start, m.shift))
    }

    /// The command for a drag from `start` to `p`.
    pub fn command(&self, start: Point, p: Point, m: Mods) -> (String, Value) {
        match self.id {
            "spiral" => {
                let r = p.distance(start).max(0.01);
                (
                    "shape.spiral".into(),
                    json!({"cx": start.x, "cy": start.y, "radius": r, "decay": self.decay, "segments": self.segments, "clockwise": self.clockwise}),
                )
            }
            // The grids: the dragged box with the tool's options.
            "rectangularGrid" | "polarGrid" => {
                let r = drag_rect(start, p, m);
                let mut params = self.options();
                if let Some(o) = params.as_object_mut() {
                    o.extend([("x", r.x0), ("y", r.y0), ("width", r.width()), ("height", r.height())].map(|(k, v)| (k.to_string(), json!(v))));
                }
                (format!("shape.{}", self.id), params)
            }
            _ => {
                let d = if m.shift { square(p - start) } else { p - start };
                let (a, b) = if m.alt { (start - d, start + d) } else { (start, start + d) };
                ("shape.arc".into(), json!({"x1": a.x, "y1": a.y, "x2": b.x, "y2": b.y, "closed": self.closed}))
            }
        }
    }
}

impl Tool for FamilyTool {
    fn id(&self) -> &'static str {
        self.id
    }
    fn busy(&self) -> bool {
        self.start.is_some()
    }
    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        match ev.kind {
            PointerKind::Move => {
                self.snap.hover(cx, ev.pos, &[], None);
                vec![]
            }
            PointerKind::Down => {
                let p = self.snap.press(cx, ev.pos, &[], None);
                self.start = Some(p);
                self.last = p;
                self.began = false;
                vec![]
            }
            PointerKind::Drag => {
                let Some(mut s) = self.start else { return vec![] };
                let pos = self.snap.drag(cx, ev.pos, self.leave(s, ev.mods).as_ref());
                let ev = &PointerEvent { pos, ..*ev };
                crate::shape::space_moves(&mut s, &mut self.last, ev, self.began);
                self.start = Some(s);
                self.mods = ev.mods;
                let mut out = vec![];
                if !self.began {
                    if ev.pos.distance(s) < cx.tol(2.0) {
                        return out;
                    }
                    self.began = true;
                    out.push(Action::Begin(self.label().into()));
                }
                let (c, v) = self.command(s, ev.pos, ev.mods);
                out.push(Action::Preview(c, v));
                out
            }
            PointerKind::Up => {
                let Some(s) = self.start.take() else { return vec![] };
                self.snap.clear();
                if std::mem::take(&mut self.began) { vec![Action::Commit] } else { vec![Action::Dialog(self.id.into(), json!({"x": s.x, "y": s.y}))] }
            }
            _ => vec![],
        }
    }
    fn key(&mut self, _cx: &ToolContext, key: ToolKey, _m: Mods) -> Vec<Action> {
        let Some(s) = self.start.filter(|_| self.began) else { return vec![] };
        let changed = match (self.id, key) {
            ("spiral", ToolKey::Up) => {
                self.segments = (self.segments + 1).min(1000);
                true
            }
            ("spiral", ToolKey::Down) => {
                self.segments = self.segments.saturating_sub(1).max(2);
                true
            }
            ("rectangularGrid" | "polarGrid", ToolKey::Up) => {
                self.rows = (self.rows + 1).min(999);
                true
            }
            ("rectangularGrid" | "polarGrid", ToolKey::Down) => {
                self.rows = self.rows.saturating_sub(1);
                true
            }
            ("rectangularGrid" | "polarGrid", ToolKey::Right) => {
                self.columns = (self.columns + 1).min(999);
                true
            }
            ("rectangularGrid" | "polarGrid", ToolKey::Left) => {
                self.columns = self.columns.saturating_sub(1);
                true
            }
            _ => false,
        };
        if changed {
            let (c, v) = self.command(s, self.last, self.mods);
            vec![Action::Preview(c, v)]
        } else if key == ToolKey::Escape {
            self.start = None;
            self.began = false;
            self.snap.clear();
            vec![Action::Cancel]
        } else {
            vec![]
        }
    }
    fn overlays(&self, cx: &ToolContext) -> Vec<Overlay> {
        let mut o = self.snap.guides().to_vec();
        if let Some(s) = self.start.filter(|_| self.began && cx.measurement_labels) {
            // The size as drawn (from the centre with Alt, square with Shift).
            let r = drag_rect(s, self.last, self.mods);
            let (w, h) = if self.id == "spiral" { ((self.last - s).x.abs(), (self.last - s).y.abs()) } else { (r.width(), r.height()) };
            o.push(Overlay::Measure { p: self.last, text: cx.size_label(w, h) });
        }
        o
    }
    fn cursor(&self, _cx: &ToolContext, _p: Point, _m: Mods) -> Cursor {
        Cursor::Crosshair
    }
    fn options(&self) -> Value {
        match self.id {
            "spiral" => json!({"decay": self.decay, "segments": self.segments, "clockwise": self.clockwise}),
            "rectangularGrid" => json!({
                "rows": self.rows, "columns": self.columns, "rowSkew": self.skew[0], "columnSkew": self.skew[1], "frame": self.frame, "fill": self.fill,
            }),
            "polarGrid" => json!({
                "concentric": self.rows, "radial": self.columns, "concentricSkew": self.skew[0], "radialSkew": self.skew[1], "compound": self.compound, "fill": self.fill,
            }),
            _ => json!({"closed": self.closed}),
        }
    }
    fn set_option(&mut self, key: &str, v: &Value) {
        match key {
            "closed" => self.closed = v.as_bool().unwrap_or(self.closed),
            // Report and persist the decay that shapes::spiral can actually draw.
            "decay" => self.decay = v.as_f64().unwrap_or(self.decay).clamp(5.0, 99.99),
            "segments" => self.segments = v.as_u64().unwrap_or(10).clamp(2, 1000) as u32,
            "clockwise" => self.clockwise = v.as_bool().unwrap_or(self.clockwise),
            "rows" | "concentric" => self.rows = v.as_u64().unwrap_or(5).min(999) as u32,
            "columns" | "radial" => self.columns = v.as_u64().unwrap_or(5).min(999) as u32,
            "rowSkew" | "concentricSkew" => self.skew[0] = grid_skew(v, self.skew[0]),
            "columnSkew" | "radialSkew" => self.skew[1] = grid_skew(v, self.skew[1]),
            "frame" => self.frame = v.as_bool().unwrap_or(self.frame),
            "compound" => self.compound = v.as_bool().unwrap_or(self.compound),
            "fill" => self.fill = v.as_bool().unwrap_or(self.fill),
            _ => {}
        }
    }
}

/// A grid divider skew from option value `v`: a finite number within the range, else `current`.
fn grid_skew(v: &Value, current: f64) -> f64 {
    let max = vectorcraft_geom::shapes::GRID_SKEW_MAX;
    v.as_f64().filter(|s| s.is_finite()).map_or(current, |s| s.clamp(-max, max))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::*;
    use vectorcraft_doc::Selection;

    #[test]
    fn spiral_reports_the_decay_its_command_draws() {
        let mut t = FamilyTool::new("spiral");
        assert_eq!(t.options()["decay"], json!(80.0));
        for (requested, expected) in [(-10.0, 5.0), (5.0, 5.0), (80.0, 80.0), (99.99, 99.99), (100.0, 99.99), (150.0, 99.99), (200.0, 99.99)] {
            t.set_option("decay", &json!(requested));
            assert_eq!(t.options()["decay"], json!(expected));
            let (id, params) = t.command(Point::ZERO, Point::new(100.0, 0.0), Mods::default());
            assert_eq!(id, "shape.spiral");
            assert_eq!(params["decay"], json!(expected));
            // Inspect the actual adjacent radii, not a second copy of the tool's clamp.
            let path = vectorcraft_geom::shapes::spiral(Point::ZERO, 100.0, params["decay"].as_f64().unwrap(), 2, true);
            let anchors = &path.subpaths[0].anchors;
            let outer = anchors[2].p.distance(Point::ZERO);
            let next = anchors[1].p.distance(Point::ZERO);
            assert!((100.0 * next / outer - expected).abs() < 1e-10);
        }
        for invalid in [Value::Null, json!("80"), json!(false), json!({})] {
            t.set_option("decay", &invalid);
            assert_eq!(t.options()["decay"], json!(99.99));
        }
    }

    #[test]
    fn normalized_spiral_decay_preserves_existing_capped_paths_within_roundoff() {
        for requested in [100.0, 150.0] {
            let mut t = FamilyTool::new("spiral");
            t.set_option("decay", &json!(requested));
            for clockwise in [false, true] {
                for segments in [2, 10, 1000] {
                    for radius in [1.0, 100.0, 1e6] {
                        let previous = vectorcraft_geom::shapes::spiral(Point::ZERO, radius, requested, segments, clockwise);
                        let normalized = vectorcraft_geom::shapes::spiral(Point::ZERO, radius, t.decay, segments, clockwise);
                        let old = &previous.subpaths[0].anchors;
                        let new = &normalized.subpaths[0].anchors;
                        assert_eq!(old.len(), new.len());
                        for (a, b) in old.iter().zip(new) {
                            assert_eq!(a.kind, b.kind);
                            for (p, q) in [(a.p, b.p), (a.h_in, b.h_in), (a.h_out, b.h_out)] {
                                assert!(p.distance(q) <= 1e-10 * (1.0 + radius), "{requested}% / {segments} / {radius}: {p:?} != {q:?}");
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn grid_drag_and_arrows() {
        let (d, _) = doc_with_rect();
        let s = Selection::default();
        let p = paint();
        let mut cx = cx(&d, &s, &p);
        cx.smart_guides = false;
        let mut t = FamilyTool::new("rectangularGrid");
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 10.0, 10.0));
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 60.0, 40.0));
        assert_eq!(a[0], Action::Begin("Rectangular Grid".into()));
        assert_eq!(
            a[1],
            Action::Preview(
                "shape.rectangularGrid".into(),
                json!({"x": 10.0, "y": 10.0, "width": 50.0, "height": 30.0, "rows": 5, "columns": 5, "rowSkew": 0.0, "columnSkew": 0.0, "frame": true, "fill": false})
            )
        );
        let a = t.key(&cx, ToolKey::Right, Mods::default());
        assert!(matches!(&a[0], Action::Preview(_, v) if v["columns"] == 6));
        // Space held moves the grid at its size; let go, it grows again from there.
        let space = Mods { space: true, ..Mods::default() };
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 80.0, 50.0).with_mods(space));
        assert!(matches!(&a[..], [Action::Preview(_, v)] if v["x"] == 30.0 && v["y"] == 20.0 && v["width"] == 50.0 && v["height"] == 30.0), "{a:?}");
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 90.0, 60.0));
        assert!(matches!(&a[..], [Action::Preview(_, v)] if v["x"] == 30.0 && v["width"] == 60.0 && v["height"] == 40.0), "{a:?}");
        assert_eq!(t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 90.0, 60.0)), vec![Action::Commit]);
    }

    /// #1377: a drag draws with the tool's stored options, which keep to their types and range.
    #[test]
    fn a_grid_drag_uses_the_stored_options() {
        let mut t = FamilyTool::new("polarGrid");
        for (key, value) in [
            ("concentric", json!(2)),
            ("concentricSkew", json!(-40)),
            ("radialSkew", json!(9000)),
            ("compound", json!(true)),
            ("fill", json!(true)),
            // Not this tool's, and not values: ignored.
            ("frame", json!("no")),
            ("concentricSkew", json!("far")),
            ("fill", json!(null)),
        ] {
            t.set_option(key, &value);
        }
        let options = json!({"concentric": 2, "radial": 5, "concentricSkew": -40.0, "radialSkew": 500.0, "compound": true, "fill": true});
        assert_eq!(t.options(), options);
        let (command, params) = t.command(Point::new(10.0, 20.0), Point::new(40.0, 80.0), Mods::default());
        assert_eq!(command, "shape.polarGrid");
        let mut want = options;
        want.as_object_mut().unwrap().extend(json!({"x": 10.0, "y": 20.0, "width": 30.0, "height": 60.0}).as_object().unwrap().clone());
        assert_eq!(params, want);
        let mut t = FamilyTool::new("rectangularGrid");
        t.set_option("frame", &json!(false));
        t.set_option("rowSkew", &json!(12.5));
        assert_eq!(t.options(), json!({"rows": 5, "columns": 5, "rowSkew": 12.5, "columnSkew": 0.0, "frame": false, "fill": false}));
    }

    #[test]
    fn click_opens_dialog_and_arc_shift() {
        let (d, _) = doc_with_rect();
        let s = Selection::default();
        let p = paint();
        let mut cx = cx(&d, &s, &p);
        cx.smart_guides = false;
        let mut t = FamilyTool::new("spiral");
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 10.0, 10.0));
        assert_eq!(
            t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 10.0, 10.0)),
            vec![Action::Dialog("spiral".into(), json!({"x": 10.0, "y": 10.0}))]
        );
        let t = FamilyTool::new("arc");
        let (c, v) = t.command(Point::new(0.0, 0.0), Point::new(10.0, 4.0), Mods { shift: true, ..Default::default() });
        assert_eq!(c, "shape.arc");
        assert_eq!(v, json!({"x1": 0.0, "y1": 0.0, "x2": 10.0, "y2": 10.0, "closed": false}));
    }

    /// The start and the dragged corner snap to Smart Guides (#506), Shift sliding the corner of
    /// a square grid along its diagonal into line.
    #[test]
    fn corners_snap_to_smart_guides() {
        let (d, _) = doc_with_rect();
        let s = Selection::default();
        let p = paint();
        let cx = cx(&d, &s, &p);
        let mut t = FamilyTool::new("rectangularGrid");
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 202.0, 99.0));
        let shift = Mods { shift: true, ..Mods::default() };
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 255.0, 147.0).with_mods(shift));
        assert!(
            matches!(&a[..], [Action::Begin(_), Action::Preview(_, v)] if v["x"] == 200.0 && v["y"] == 100.0 && v["width"] == 50.0 && v["height"] == 50.0),
            "{a:?}"
        );
        assert!(t.overlays(&cx).iter().any(|o| matches!(o, Overlay::Label { text, .. } if text == "align")));
    }
}
