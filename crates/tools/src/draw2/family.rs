//! Line Segment family drag tools: Arc, Spiral, Rectangular Grid and Polar Grid.
//!
//! Drag draws (Shift = equal axes / square, Alt = from the centre for arc and grids, Space held
//! moves the shape being drawn); a click without dragging asks the UI for the options dialog. The
//! start point and the dragged corner snap to Smart Guides ([`DrawSnap`]), hovering too.
//! While dragging, ↑/↓ change the arc's slope, the spiral's segments, the grid rows or the
//! concentric dividers; ←/→ change grid columns / radial dividers. C switches the arc between open
//! and closed, and F switches its base between the X and Y axes. While a spiral is dragged out, Alt
//! and Ctrl/Cmd keep its inner end at the distance from the center on the first modified drag as
//! the outer end follows the pointer: Alt adds or removes whole winds of four segments, and
//! Ctrl/Cmd changes the decay.

use serde_json::{Value, json};
use vectorcraft_geom::Point;

use crate::guides::{DrawSnap, Leave};
use crate::shape::{box_params, drag_rect, turned_box};
use crate::{Action, Cursor, Mods, Overlay, PointerEvent, PointerKind, Tool, ToolContext, ToolKey};

/// How much ↑ or ↓ changes the arc's slope while it is dragged out.
const ARC_SLOPE_STEP: f64 = 10.0;

pub struct FamilyTool {
    id: &'static str,
    start: Option<Point>,
    last: Point,
    mods: Mods,
    began: bool,
    pub closed: bool,
    length_x: f64,
    length_y: f64,
    reference: usize,
    base_along: &'static str,
    slope: f64,
    fill_arc: bool,
    pub decay: f64,
    pub segments: u32,
    pub clockwise: bool,
    /// The natural logarithm of the radius of the spiral's inner end, held while Alt or Ctrl/Cmd
    /// is down in a drag.
    spiral_end: Option<f64>,
    /// The segment count when Alt alone starts adjusting winds.
    spiral_alt_base: Option<u32>,
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
    /// The grids' Default Size (width, height): the size a click's dialog opens with (a drag sets
    /// its own). The Reference Point is `reference`.
    pub grid_size: (f64, f64),
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
            length_x: 100.0,
            length_y: 100.0,
            reference: 0,
            base_along: "x",
            slope: 50.0,
            fill_arc: false,
            decay: 80.0,
            segments: 10,
            clockwise: true,
            spiral_end: None,
            spiral_alt_base: None,
            rows: 5,
            columns: 5,
            skew: [0.0; 2],
            frame: true,
            compound: false,
            fill: false,
            grid_size: (100.0, 100.0),
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
    /// the grids' corner keeps to a diagonal with Shift (a grid's square is taken along the
    /// Constrain Angle's axes instead when it turns them, [`Self::command_at`]).
    fn leave(&self, cx: &ToolContext, start: Point, m: Mods) -> Option<Leave> {
        let turned = self.is_grid() && vectorcraft_geom::constrain_axes(cx.constrain_angle).is_some();
        (self.id != "spiral").then(|| Leave::diagonal(start, m.shift && !turned))
    }

    fn is_grid(&self) -> bool {
        matches!(self.id, "rectangularGrid" | "polarGrid")
    }

    /// The natural logarithm of the radius of the inner end of a spiral of outer radius `r` drawn
    /// with the tool's decay and segments (a logarithm, so that a low decay with many segments does
    /// not round to 0). The decay ratio is clamped to 0.05…0.9999 as `shapes::spiral` clamps it.
    fn spiral_inner_end_ln(&self, r: f64) -> f64 {
        r.ln() + f64::from(self.segments) * (self.decay / 100.0).clamp(0.05, 0.9999).ln()
    }

    /// Alt or Ctrl/Cmd in a spiral drag from `start` to `p`: the inner end keeps the distance from
    /// the center it had on the first modified drag while the outer end follows the pointer. With
    /// Ctrl/Cmd the decay changes to fit (within 5…99.99 %, the decays `shapes::spiral` draws);
    /// with Alt alone the count changes in whole winds of four segments from its Alt entry count
    /// (within 2…1000). Ctrl/Cmd takes precedence when both modifiers are held.
    /// The new values stay once the key is let go.
    fn hold_spiral_end(&mut self, start: Point, p: Point, m: Mods) {
        let r = p.distance(start).max(0.01);
        match (m.cmd || m.alt, self.spiral_end) {
            (true, None) => self.spiral_end = Some(self.spiral_inner_end_ln(r)),
            (false, Some(_)) => self.spiral_end = None,
            _ => {}
        }
        if m.alt && !m.cmd {
            self.spiral_alt_base.get_or_insert(self.segments);
        } else {
            self.spiral_alt_base = None;
        }
        let Some(end) = self.spiral_end.filter(|e| e.is_finite()) else { return };
        if m.cmd {
            let ratio = ((end - r.ln()) / f64::from(self.segments)).exp();
            if ratio.is_finite() {
                self.decay = (ratio * 100.0).clamp(5.0, 99.99);
            }
        } else {
            let fitted = (r.ln() - end) / -(self.decay / 100.0).clamp(0.05, 0.9999).ln();
            if fitted.is_finite() {
                let base = f64::from(self.spiral_alt_base.unwrap_or(self.segments));
                self.segments = (base + 4.0 * ((fitted - base) / 4.0).round()).clamp(2.0, 1000.0) as u32;
            }
        }
    }

    /// A grid's options a drag draws with: its dividers, skews and switches (the Default Size and
    /// Reference Point are for a click's dialog).
    fn grid_options(&self) -> Value {
        if self.id == "polarGrid" {
            json!({
                "concentric": self.rows, "radial": self.columns, "concentricSkew": self.skew[0], "radialSkew": self.skew[1], "compound": self.compound, "fill": self.fill,
            })
        } else {
            json!({
                "rows": self.rows, "columns": self.columns, "rowSkew": self.skew[0], "columnSkew": self.skew[1], "frame": self.frame, "fill": self.fill,
            })
        }
    }

    /// The command for a drag from `start` to `p`.
    pub fn command(&self, start: Point, p: Point, m: Mods) -> (String, Value) {
        self.command_at(start, p, m, 0.0)
    }

    /// [`Self::command`] with General › Constrain Angle `base` (degrees, counter-clockwise): the
    /// grids are drawn along the x and y axes turned by it, as the rectangle is.
    pub fn command_at(&self, start: Point, p: Point, m: Mods, base: f64) -> (String, Value) {
        match self.id {
            "spiral" => {
                let r = p.distance(start).max(0.01);
                (
                    "shape.spiral".into(),
                    json!({"cx": start.x, "cy": start.y, "radius": r, "decay": self.decay, "segments": self.segments, "clockwise": self.clockwise}),
                )
            }
            // The grids: the dragged box (along the Constrain Angle's axes) with the tool's options.
            "rectangularGrid" | "polarGrid" => {
                let mut params = self.grid_options();
                if let (Some(o), Value::Object(b)) = (params.as_object_mut(), box_params(start, p, m, base)) {
                    o.extend(b);
                }
                (format!("shape.{}", self.id), params)
            }
            _ => {
                let r = drag_rect(start, p, m).abs();
                let (w, h) = (r.width().max(1e-9), r.height().max(1e-9));
                let rp = vectorcraft_geom::reference_point(vectorcraft_geom::Rect::new(0.0, 0.0, w, h), self.reference.min(8));
                let anchor = Point::new(r.x0 + rp.x, r.y0 + rp.y);
                (
                    "shape.arc".into(),
                    json!({
                        "x": anchor.x,
                        "y": anchor.y,
                        "lengthX": w,
                        "lengthY": h,
                        "reference": self.reference,
                        "baseAlong": self.base_along,
                        "slope": self.slope,
                        "closed": self.closed,
                        "fillArc": self.fill_arc,
                    }),
                )
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
                self.spiral_end = None;
                self.spiral_alt_base = None;
                vec![]
            }
            PointerKind::Drag => {
                let Some(mut s) = self.start else { return vec![] };
                let pos = self.snap.drag(cx, ev.pos, self.leave(cx, s, ev.mods).as_ref());
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
                if self.id == "spiral" {
                    self.hold_spiral_end(s, ev.pos, ev.mods);
                }
                let (c, v) = self.command_at(s, ev.pos, ev.mods, cx.constrain_angle);
                out.push(Action::Preview(c, v));
                out
            }
            PointerKind::Up => {
                let Some(s) = self.start.take() else { return vec![] };
                self.snap.clear();
                self.spiral_end = None;
                self.spiral_alt_base = None;
                if std::mem::take(&mut self.began) { vec![Action::Commit] } else { vec![Action::Dialog(self.id.into(), json!({"x": s.x, "y": s.y}))] }
            }
            _ => vec![],
        }
    }
    fn key(&mut self, cx: &ToolContext, key: ToolKey, _m: Mods) -> Vec<Action> {
        let Some(s) = self.start.filter(|_| self.began) else { return vec![] };
        let changed = match (self.id, key) {
            ("arc", ToolKey::Up) => {
                self.slope = (self.slope + ARC_SLOPE_STEP).min(100.0);
                true
            }
            ("arc", ToolKey::Down) => {
                self.slope = (self.slope - ARC_SLOPE_STEP).max(-100.0);
                true
            }
            ("arc", ToolKey::Letter('c')) => {
                self.closed = !self.closed;
                true
            }
            ("arc", ToolKey::Letter('f')) => {
                self.base_along = if self.base_along == "y" { "x" } else { "y" };
                true
            }
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
            // An arrow with Alt or Ctrl/Cmd still held: the held inner end moves with the new segment count.
            if self.spiral_end.is_some() {
                self.spiral_end = Some(self.spiral_inner_end_ln(self.last.distance(s).max(0.01)));
                self.spiral_alt_base = (self.mods.alt && !self.mods.cmd).then_some(self.segments);
            }
            let (c, v) = self.command_at(s, self.last, self.mods, cx.constrain_angle);
            vec![Action::Preview(c, v)]
        } else if key == ToolKey::Escape {
            self.start = None;
            self.began = false;
            self.spiral_end = None;
            self.spiral_alt_base = None;
            self.snap.clear();
            vec![Action::Cancel]
        } else {
            vec![]
        }
    }
    /// The Arc tool takes C and F while an arc is dragged out: C switches it between open and closed, and F switches its base axis.
    fn claims_key(&self, _cx: &ToolContext, key: ToolKey) -> bool {
        self.id == "arc" && self.began && self.start.is_some() && matches!(key, ToolKey::Letter('c' | 'f'))
    }
    fn overlays(&self, cx: &ToolContext) -> Vec<Overlay> {
        let mut o = self.snap.guides().to_vec();
        if let Some(s) = self.start.filter(|_| self.began && cx.measurement_labels) {
            // The size as drawn (from the centre with Alt, square with Shift).
            // A grid's along the Constrain Angle's axes, as it is drawn.
            let r = if self.is_grid() { turned_box(s, self.last, self.mods, cx.constrain_angle).0 } else { drag_rect(s, self.last, self.mods) };
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
            "rectangularGrid" | "polarGrid" => {
                let mut o = self.grid_options();
                o["width"] = json!(self.grid_size.0);
                o["height"] = json!(self.grid_size.1);
                o["reference"] = json!(self.reference);
                o
            }
            _ => json!({
                "closed": self.closed,
                "lengthX": self.length_x,
                "lengthY": self.length_y,
                "reference": self.reference,
                "baseAlong": self.base_along,
                "slope": self.slope,
                "fillArc": self.fill_arc,
            }),
        }
    }
    fn set_option(&mut self, key: &str, v: &Value) {
        match key {
            "closed" => self.closed = v.as_bool().unwrap_or(self.closed),
            "lengthX" => self.length_x = arc_length(v, self.length_x),
            "lengthY" => self.length_y = arc_length(v, self.length_y),
            "reference" => self.reference = v.as_u64().unwrap_or(self.reference as u64).min(8) as usize,
            "baseAlong" => {
                if let Some(s) = v.as_str() {
                    self.base_along = if s.eq_ignore_ascii_case("y") { "y" } else { "x" };
                }
            }
            "slope" => self.slope = v.as_f64().filter(|n| n.is_finite()).map_or(self.slope, |n| n.clamp(-100.0, 100.0)),
            "fillArc" => self.fill_arc = v.as_bool().unwrap_or(self.fill_arc),
            "decay" => self.decay = v.as_f64().unwrap_or(self.decay).clamp(5.0, 150.0),
            "segments" => self.segments = v.as_u64().unwrap_or(10).clamp(2, 1000) as u32,
            "clockwise" => self.clockwise = v.as_bool().unwrap_or(self.clockwise),
            "rows" | "concentric" => self.rows = v.as_u64().unwrap_or(5).min(999) as u32,
            "columns" | "radial" => self.columns = v.as_u64().unwrap_or(5).min(999) as u32,
            "rowSkew" | "concentricSkew" => self.skew[0] = grid_skew(v, self.skew[0]),
            "columnSkew" | "radialSkew" => self.skew[1] = grid_skew(v, self.skew[1]),
            "frame" => self.frame = v.as_bool().unwrap_or(self.frame),
            "compound" => self.compound = v.as_bool().unwrap_or(self.compound),
            "fill" => self.fill = v.as_bool().unwrap_or(self.fill),
            "width" => self.grid_size.0 = grid_length(v, self.grid_size.0),
            "height" => self.grid_size.1 = grid_length(v, self.grid_size.1),
            _ => {}
        }
    }
}

/// Arc segment length from a dialog or preference value.
fn arc_length(v: &Value, current: f64) -> f64 {
    v.as_f64().filter(|n| n.is_finite()).map_or(current, |n| n.abs().max(1e-9))
}

/// A grid's Default Size side from option value `v`: a finite positive length (at most 1e6 pt),
/// else `current`.
fn grid_length(v: &Value, current: f64) -> f64 {
    v.as_f64().filter(|n| n.is_finite() && *n > 0.0).map_or(current, |n| n.min(1e6))
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
        let mut reported = options.clone();
        reported.as_object_mut().unwrap().extend(json!({"width": 100.0, "height": 100.0, "reference": 0}).as_object().unwrap().clone());
        assert_eq!(t.options(), reported);
        let (command, params) = t.command(Point::new(10.0, 20.0), Point::new(40.0, 80.0), Mods::default());
        assert_eq!(command, "shape.polarGrid");
        let mut want = options;
        want.as_object_mut().unwrap().extend(json!({"x": 10.0, "y": 20.0, "width": 30.0, "height": 60.0}).as_object().unwrap().clone());
        assert_eq!(params, want);
        let mut t = FamilyTool::new("rectangularGrid");
        t.set_option("frame", &json!(false));
        t.set_option("rowSkew", &json!(12.5));
        assert_eq!(
            t.options(),
            json!({"rows": 5, "columns": 5, "rowSkew": 12.5, "columnSkew": 0.0, "frame": false, "fill": false, "width": 100.0, "height": 100.0, "reference": 0})
        );
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
        assert_eq!(
            v,
            json!({
                "x": 0.0,
                "y": 0.0,
                "lengthX": 10.0,
                "lengthY": 10.0,
                "reference": 0,
                "baseAlong": "x",
                "slope": 50.0,
                "closed": false,
                "fillArc": false,
            })
        );
    }

    /// While a spiral is dragged out, Alt and Ctrl/Cmd keep its inner end at the distance from the
    /// center it had when the key went down as the outer end follows the pointer: Alt adds or
    /// removes whole winds of four segments and Ctrl/Cmd changes the decay. The new values stay
    /// once the key is let go, and for the next spiral. Both keys work with the lowest decay and
    /// many segments.
    #[test]
    fn spiral_drag_alt_changes_its_segments_and_ctrl_its_decay() {
        let (d, _) = doc_with_rect();
        let s = Selection::default();
        let p = paint();
        let mut cx = cx(&d, &s, &p);
        cx.smart_guides = false;
        let alt = Mods { alt: true, ..Mods::default() };
        let ctrl = Mods { cmd: true, ..Mods::default() };
        let spiral = |a: Vec<Action>| match a.last() {
            Some(Action::Preview(c, v)) if c == "shape.spiral" => {
                (v["radius"].as_f64().unwrap(), v["decay"].as_f64().unwrap(), v["segments"].as_u64().unwrap())
            }
            _ => panic!("{a:?}"),
        };
        let mut t = FamilyTool::new("spiral");
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 100.0, 100.0));
        assert_eq!(spiral(t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 200.0, 100.0))), (100.0, 80.0, 10));
        // Alt rounds the fitted count to whole winds from the count when Alt goes down.
        assert_eq!(spiral(t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 200.0, 100.0).with_mods(alt))), (100.0, 80.0, 10));
        assert_eq!(spiral(t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 300.0, 100.0).with_mods(alt))), (200.0, 80.0, 14));
        // ↑ with Alt held adds one, and the held inner end moves with it.
        assert_eq!(spiral(t.key(&cx, ToolKey::Up, alt)).2, 15);
        assert_eq!(spiral(t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 300.0, 100.0).with_mods(alt))).2, 15);
        assert_eq!(spiral(t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 150.0, 100.0).with_mods(alt))), (50.0, 80.0, 7));
        assert_eq!(spiral(t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 101.0, 100.0).with_mods(alt))).2, 2, "at least two");
        assert_eq!(spiral(t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 150.0, 100.0).with_mods(alt))).2, 7);
        // The count stays as the radius follows the pointer again.
        assert_eq!(spiral(t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 200.0, 100.0))), (100.0, 80.0, 7));
        // Ctrl/Cmd fits the decay to the held inner end.
        t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 200.0, 100.0).with_mods(ctrl));
        let (r, decay, n) = spiral(t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 150.0, 100.0).with_mods(ctrl)));
        assert!((r, n) == (50.0, 7) && (decay - 80.0 * 2f64.powf(1.0 / 7.0)).abs() < 1e-9, "{r} {decay} {n}");
        let (_, decay, _) = spiral(t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 400.0, 100.0).with_mods(ctrl)));
        assert!((decay - 80.0 * (1.0f64 / 3.0).powf(1.0 / 7.0)).abs() < 1e-9, "{decay}");
        assert_eq!(spiral(t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 100.5, 100.0).with_mods(ctrl))).1, 99.99, "at most 99.99 %");
        assert_eq!(spiral(t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 1e20, 100.0).with_mods(ctrl))).1, 5.0, "at least 5 %");
        let (_, decay, _) = spiral(t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 300.0, 100.0).with_mods(ctrl)));
        assert!((decay - 80.0 * 0.5f64.powf(1.0 / 7.0)).abs() < 1e-9, "{decay}");
        // Let go: the decay stays, and the spiral drawn next keeps both.
        assert_eq!(spiral(t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 250.0, 100.0))), (150.0, decay, 7));
        assert_eq!(t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 250.0, 100.0)), vec![Action::Commit]);
        assert_eq!((t.options()["decay"].as_f64(), t.options()["segments"].clone()), (Some(decay), json!(7)));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 100.0, 100.0));
        let (_, d2, n2) = spiral(t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 100.0, 140.0).with_mods(alt)));
        assert_eq!((d2, n2), (decay, 7), "a new drag holds the end anew");
        // 5 % and 300 segments: an inner radius of 100 × 0.05^300 is below the smallest f64.
        let mut t = FamilyTool::new("spiral");
        t.set_option("decay", &json!(5.0));
        t.set_option("segments", &json!(300));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 100.0, 100.0));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 200.0, 100.0).with_mods(ctrl));
        let (_, decay, _) = spiral(t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 150.0, 100.0).with_mods(ctrl)));
        assert!((decay - 5.0 * 2f64.powf(1.0 / 300.0)).abs() < 1e-9, "{decay}");
        t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 150.0, 100.0));
        // Alt: a large radius increase adds a whole wind.
        t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 120.0, 100.0).with_mods(alt));
        assert_eq!(spiral(t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 200100.0, 100.0).with_mods(alt))).2, 304);
    }

    /// Switching between Alt and Ctrl/Cmd keeps the held inner end and uses the current segment
    /// count when Alt resumes. Escape clears the held end before the next drag.
    #[test]
    fn spiral_drag_switches_between_alt_and_ctrl_and_escape_lets_go_of_the_inner_end() {
        let (d, _) = doc_with_rect();
        let s = Selection::default();
        let p = paint();
        let mut cx = cx(&d, &s, &p);
        cx.smart_guides = false;
        let alt = Mods { alt: true, ..Mods::default() };
        let ctrl = Mods { cmd: true, ..Mods::default() };
        let both = Mods { alt: true, cmd: true, ..Mods::default() };
        let mut t = FamilyTool::new("spiral");
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 100.0, 100.0));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 200.0, 100.0).with_mods(alt));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 300.0, 100.0).with_mods(alt));
        assert_eq!(t.segments, 14);
        t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 200.0, 100.0).with_mods(both));
        assert_eq!(t.segments, 14, "Ctrl/Cmd takes precedence");
        let decay = t.decay;
        assert!(decay > 80.0);
        t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 200.0, 100.0).with_mods(ctrl));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 300.0, 100.0).with_mods(alt));
        assert_eq!(t.segments, 18, "Alt resumes from the current count with the held end");
        assert_eq!(t.decay, decay);
        t.key(&cx, ToolKey::Escape, alt);
        assert_eq!(t.spiral_end, None);
        assert_eq!(t.spiral_alt_base, None);
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 100.0, 100.0));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 200.0, 100.0).with_mods(alt));
        assert_eq!(t.segments, 18, "a new drag holds a new inner end");
    }

    /// Alt adjusts the segment count in whole winds while keeping it within 2 to 1000.
    #[test]
    fn spiral_drag_alt_keeps_the_segments_within_2_to_1000() {
        let mut t = FamilyTool::new("spiral");
        let alt = Mods { alt: true, ..Mods::default() };
        let start = Point::new(0.0, 0.0);
        t.hold_spiral_end(start, Point::new(100.0, 0.0), alt);
        t.hold_spiral_end(start, Point::new(200.0, 0.0), alt);
        assert_eq!(t.segments, 14);
        t.hold_spiral_end(start, Point::new(0.01, 0.0), alt);
        assert_eq!(t.segments, 2);
        t.hold_spiral_end(start, Point::new(1e150, 0.0), alt);
        assert_eq!(t.segments, 1000);
    }

    #[test]
    fn spiral_drag_arrows_and_release_clear_the_right_held_state() {
        let (d, _) = doc_with_rect();
        let s = Selection::default();
        let p = paint();
        let mut cx = cx(&d, &s, &p);
        cx.smart_guides = false;
        let alt = Mods { alt: true, ..Mods::default() };
        let both = Mods { alt: true, cmd: true, ..Mods::default() };
        let mut t = FamilyTool::new("spiral");
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 100.0, 100.0));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 200.0, 100.0).with_mods(both));
        t.key(&cx, ToolKey::Up, both);
        assert_eq!(t.segments, 11);
        assert_eq!(t.spiral_alt_base, None, "Ctrl/Cmd still takes precedence after Up");
        t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 200.0, 100.0).with_mods(alt));
        assert_eq!(t.spiral_alt_base, Some(11), "Alt starts from the current count");
        t.key(&cx, ToolKey::Down, alt);
        assert_eq!(t.segments, 10);
        assert_eq!(t.spiral_alt_base, Some(10));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 200.0, 100.0).with_mods(alt));
        assert_eq!(t.spiral_end, None);
        assert_eq!(t.spiral_alt_base, None);
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 100.0, 100.0));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 100.0, 100.0));
        assert_eq!(t.spiral_end, None, "Up after Down clears the held end");
        assert_eq!(t.spiral_alt_base, None, "Up after Down clears the Alt base");
    }

    #[test]
    fn arc_drag_uses_stored_segment_options() {
        let mut t = FamilyTool::new("arc");
        for (key, value) in [
            ("lengthX", json!(80.0)),
            ("lengthY", json!(40.0)),
            ("reference", json!(4)),
            ("baseAlong", json!("y")),
            ("slope", json!(-25.0)),
            ("closed", json!(true)),
            ("fillArc", json!(true)),
        ] {
            t.set_option(key, &value);
        }
        let (command, params) = t.command(Point::new(10.0, 20.0), Point::new(50.0, 70.0), Mods::default());
        assert_eq!(command, "shape.arc");
        assert_eq!(
            params,
            json!({
                "x": 30.0,
                "y": 45.0,
                "lengthX": 40.0,
                "lengthY": 50.0,
                "reference": 4,
                "baseAlong": "y",
                "slope": -25.0,
                "closed": true,
                "fillArc": true,
            })
        );
    }

    /// While an arc is dragged out, C switches it between open and closed, F switches its base
    /// between the X and Y axes and ↑/↓ change its slope by 10 within −100…100; the arc drawn
    /// next keeps them.
    #[test]
    fn arc_drag_keys_change_its_type_base_and_slope() {
        let (d, _) = doc_with_rect();
        let s = Selection::default();
        let p = paint();
        let mut cx = cx(&d, &s, &p);
        cx.smart_guides = false;
        let mut t = FamilyTool::new("arc");
        let (c, f) = (ToolKey::Letter('c'), ToolKey::Letter('f'));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 10.0, 10.0));
        assert!(!t.claims_key(&cx, c), "no drag yet");
        t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 60.0, 40.0));
        assert!(t.claims_key(&cx, c) && t.claims_key(&cx, f) && !t.claims_key(&cx, ToolKey::Letter('e')));
        let preview = |a: Vec<Action>| match &a[..] {
            [Action::Preview(c, v)] if c == "shape.arc" => v.clone(),
            _ => panic!("{a:?}"),
        };
        let v = preview(t.key(&cx, c, Mods::default()));
        assert_eq!((v["closed"].clone(), v["baseAlong"].clone(), v["slope"].clone()), (json!(true), json!("x"), json!(50.0)));
        let v = preview(t.key(&cx, f, Mods::default()));
        assert_eq!((v["closed"].clone(), v["baseAlong"].clone()), (json!(true), json!("y")));
        assert_eq!((v["x"].clone(), v["y"].clone(), v["reference"].clone()), (json!(10.0), json!(10.0), json!(0)), "the reference point stays");
        assert_eq!(preview(t.key(&cx, ToolKey::Up, Mods::default()))["slope"], json!(60.0));
        for _ in 0..30 {
            t.key(&cx, ToolKey::Down, Mods::default());
        }
        assert_eq!(preview(t.key(&cx, ToolKey::Down, Mods::default()))["slope"], json!(-100.0));
        for _ in 0..30 {
            t.key(&cx, ToolKey::Up, Mods::default());
        }
        assert_eq!(preview(t.key(&cx, ToolKey::Up, Mods::default()))["slope"], json!(100.0));
        let v = preview(t.key(&cx, c, Mods::default()));
        assert_eq!(v["closed"], json!(false), "C again opens it");
        assert_eq!(t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 60.0, 40.0)), vec![Action::Commit]);
        assert!(!t.claims_key(&cx, c), "the drag is over");
        let o = t.options();
        assert_eq!((o["closed"].clone(), o["baseAlong"].clone(), o["slope"].clone()), (json!(false), json!("y"), json!(100.0)));
        // The other tools of the family leave C and F to their shortcuts.
        let mut g = FamilyTool::new("rectangularGrid");
        g.pointer(&cx, &PointerEvent::new(PointerKind::Down, 10.0, 10.0));
        g.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 60.0, 40.0));
        assert!(!g.claims_key(&cx, c) && !g.claims_key(&cx, f));
    }

    /// General › Constrain Angle: grids are drawn along the turned axes, Shift's square too; at 0°
    /// and whole turns as before.
    #[test]
    fn grids_follow_the_constrain_angle() {
        let t = FamilyTool::new("polarGrid");
        let (a, b) = (Point::new(100.0, 100.0), Point::new(160.0, 120.0));
        let flat = t.command(a, b, Mods::default());
        assert_eq!(t.command_at(a, b, Mods::default(), 360.0), flat);
        assert_eq!(t.command_at(a, b, Mods::default(), -720.0), flat);
        let (_, v) = t.command_at(a, b, Mods::default(), 90.0);
        assert_eq!(
            v,
            json!({"x": 120.0, "y": 80.0, "width": 20.0, "height": 60.0, "rotation": -90.0, "concentric": 5, "radial": 5, "concentricSkew": 0.0, "radialSkew": 0.0, "compound": false, "fill": false})
        );
        let shift = Mods { shift: true, ..Mods::default() };
        let (_, v) = FamilyTool::new("rectangularGrid").command_at(a, b, shift, 30.0);
        assert!((v["width"].as_f64().unwrap() - v["height"].as_f64().unwrap()).abs() < 1e-9, "a square along the turned axes: {v}");
        assert_eq!(v["rotation"], json!(-30.0));
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

    /// A grid's Default Size and Reference Point are options for a click's dialog; the drag
    /// draws the box it is dragged to and leaves them out of its command.
    #[test]
    fn grid_default_size_is_an_option_the_drag_leaves_out() {
        for id in ["rectangularGrid", "polarGrid"] {
            let mut t = FamilyTool::new(id);
            for (k, v) in [("width", json!(150.0)), ("height", json!(-3.0)), ("reference", json!(4))] {
                t.set_option(k, &v);
            }
            let o = t.options();
            assert_eq!((o["width"].clone(), o["height"].clone(), o["reference"].clone()), (json!(150.0), json!(100.0), json!(4)), "{id}");
            let (_, p) = t.command(Point::new(0.0, 0.0), Point::new(40.0, 30.0), Mods::default());
            assert!(p.get("reference").is_none(), "{id}: {p}");
            assert_eq!((p["width"].clone(), p["height"].clone()), (json!(40.0), json!(30.0)), "{id}");
        }
    }
}
