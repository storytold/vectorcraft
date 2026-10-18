//! Free Transform tool (E).
//!
//! Shows the bounding box with handles. In the default mode handles scale (Shift proportional,
//! Alt from centre), outside the corners rotates, inside moves, and Cmd-dragging a side handle
//! shears along that side. A scale snaps as the Selection tool's does. The Perspective Distort and Free Distort modes (tool option `mode`, or
//! Cmd / Cmd+Alt+Shift while dragging a corner like classic Illustrator) move the corners of the
//! box and preview `object.distort`.

use serde_json::{Value, json};
use vectorcraft_geom::{Affine, Point, Rect, Vec2};

use super::{BLUE, polygon, rect_corners};
use crate::bbox::{Handle, hit_handle, in_rotate_zone, move_delta, rotate_for_drag, scale_for_drag};
use crate::guides::{PointSnap, Targets};
use crate::select::{matrix_json, selection_bounds};
use crate::{Action, Cursor, Mods, Overlay, PointerEvent, PointerKind, Tool, ToolContext, ToolKey};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DistortMode {
    /// Scale / rotate / shear / move.
    #[default]
    Free,
    /// Corners move symmetrically along their side.
    Perspective,
    /// Corners and sides move independently.
    Distort,
}

impl DistortMode {
    fn name(self) -> &'static str {
        match self {
            DistortMode::Free => "free",
            DistortMode::Perspective => "perspective",
            DistortMode::Distort => "distort",
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum Op {
    Move,
    Scale(Handle),
    Shear(Handle),
    Rotate,
    Corners(Handle, DistortMode),
}

impl Op {
    /// The bounding-box handle being dragged, if any.
    fn handle(self) -> Option<Handle> {
        match self {
            Op::Scale(h) | Op::Shear(h) | Op::Corners(h, _) => Some(h),
            Op::Move | Op::Rotate => None,
        }
    }

    /// The undo step's name.
    fn label(self) -> &'static str {
        match self {
            Op::Move => "Move",
            Op::Scale(_) => "Scale",
            Op::Shear(_) => "Shear",
            Op::Rotate => "Rotate",
            Op::Corners(..) => "Distort",
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Drag {
    op: Op,
    rect: Rect,
    start: Point,
    began: bool,
}

#[derive(Default)]
pub struct FreeTransformTool {
    pub mode: DistortMode,
    /// The on-canvas "Constrain" toggle (acts like holding Shift).
    pub constrain: bool,
    drag: Option<Drag>,
    /// The current (distorted) quad while dragging corners.
    quad: Option<[Point; 4]>,
    measure: Option<(Point, String)>,
    /// What a scale snaps to ([`PointSnap::snap_scale`]): pixels, the grid, the other objects with
    /// Smart Guides, or their anchors with Snap to Point.
    targets: Option<PointSnap>,
    /// The lines of the current snap.
    guides: Vec<Overlay>,
}

/// Shear for dragging side `h` of `r` by `d` (about the opposite side, or the centre with Alt).
pub fn shear_for_side(r: Rect, h: Handle, d: Vec2, from_center: bool) -> Affine {
    let o = if from_center { r.center() } else { h.opposite().pos(r) };
    let hp = h.pos(r);
    let m = match h {
        Handle::Top | Handle::Bottom => {
            let lever = hp.y - o.y;
            let k = if lever.abs() > 1e-9 { d.x / lever } else { 0.0 };
            Affine::new([1.0, 0.0, k, 1.0, 0.0, 0.0])
        }
        _ => {
            let lever = hp.x - o.x;
            let k = if lever.abs() > 1e-9 { d.y / lever } else { 0.0 };
            Affine::new([1.0, k, 0.0, 1.0, 0.0, 0.0])
        }
    };
    Affine::translate(o.to_vec2()) * m * Affine::translate(-o.to_vec2())
}

/// Corner index (TL, TR, BR, BL) for a corner handle.
fn corner_index(h: Handle) -> Option<usize> {
    match h {
        Handle::TopLeft => Some(0),
        Handle::TopRight => Some(1),
        Handle::BottomRight => Some(2),
        Handle::BottomLeft => Some(3),
        _ => None,
    }
}

/// The distorted quad for dragging handle `h` of `r` by `d`.
pub fn distort_quad(r: Rect, h: Handle, d: Vec2, mode: DistortMode) -> [Point; 4] {
    let mut q = rect_corners(r);
    match (corner_index(h), mode) {
        (Some(i), DistortMode::Perspective) => {
            // Move along the dominant axis; the partner corner on that side mirrors the move.
            if d.x.abs() >= d.y.abs() {
                let partner = [1, 0, 3, 2][i];
                q[i].x += d.x;
                q[partner].x -= d.x;
            } else {
                let partner = [3, 2, 1, 0][i];
                q[i].y += d.y;
                q[partner].y -= d.y;
            }
        }
        (Some(i), _) => q[i] += d,
        (None, _) => {
            let (a, b) = match h {
                Handle::Top => (0, 1),
                Handle::Right => (1, 2),
                Handle::Bottom => (2, 3),
                _ => (3, 0),
            };
            q[a] += d;
            q[b] += d;
        }
    }
    q
}

impl FreeTransformTool {
    /// What dragging handle `h` with modifiers `m` does: Cmd on a corner distorts it freely,
    /// Cmd+Alt+Shift in perspective, Cmd on a side shears; otherwise the tool's mode.
    fn handle_op(&self, h: Handle, m: Mods) -> Op {
        let mode = if h.is_corner() && m.cmd && m.alt && m.shift {
            DistortMode::Perspective
        } else if h.is_corner() && m.cmd {
            DistortMode::Distort
        } else {
            self.mode
        };
        match mode {
            DistortMode::Free if !h.is_corner() && m.cmd => Op::Shear(h),
            DistortMode::Free => Op::Scale(h),
            DistortMode::Perspective if !h.is_corner() => Op::Scale(h),
            mode => Op::Corners(h, mode),
        }
    }

    fn classify(&self, cx: &ToolContext, r: Rect, p: Point, m: Mods) -> Option<Op> {
        let tol = cx.tol(5.0);
        if let Some(h) = hit_handle(r, p, tol) {
            return Some(self.handle_op(h, m));
        }
        if self.mode == DistortMode::Free && in_rotate_zone(r, p, tol, cx.tol(18.0)).is_some() {
            return Some(Op::Rotate);
        }
        r.contains(p).then_some(Op::Move)
    }

    fn preview(&mut self, cx: &ToolContext, d: Drag, p: Point, m: Mods) -> Action {
        let shift = m.shift || self.constrain;
        let delta = p - d.start;
        self.guides.clear();
        let xf = match d.op {
            Op::Corners(h, mode) => {
                let q = distort_quad(d.rect, h, delta, mode);
                self.quad = Some(q);
                self.measure = None;
                let corners: Vec<Value> = q.iter().map(|c| json!([c.x, c.y])).collect();
                return Action::Preview("object.distort".into(), json!({ "corners": corners, "from": [d.rect.x0, d.rect.y0, d.rect.x1, d.rect.y1] }));
            }
            Op::Move => {
                let v = move_delta(d.start, p, shift);
                self.measure = cx.measurement_labels.then(|| (p, cx.offset_label(v.x, v.y)));
                Affine::translate(v)
            }
            Op::Scale(h) => {
                let mut a = scale_for_drag(d.rect, h, p, shift, m.alt);
                if let Some(t) = &self.targets {
                    (a, self.guides) = t.snap_scale(cx, &vectorcraft_doc::OrientedBox::aligned(d.rect), h, a, shift, m.alt);
                }
                let nr = a.transform_rect_bbox(d.rect);
                self.measure = cx.transform_tools_guides.then(|| (p, cx.size_label(nr.width(), nr.height())));
                a
            }
            Op::Shear(h) => shear_for_side(d.rect, h, delta, m.alt),
            Op::Rotate => {
                let (a, deg) = rotate_for_drag(d.rect.center(), d.start, p, shift);
                self.measure = cx.transform_tools_guides.then(|| (p, format!("{:.1}°", -deg)));
                a
            }
        };
        self.quad = Some(rect_corners(d.rect).map(|c| xf * c));
        Action::Preview("object.transform".into(), json!({ "matrix": matrix_json(xf), "copy": false }))
    }
}

impl Tool for FreeTransformTool {
    fn id(&self) -> &'static str {
        "freeTransform"
    }

    fn busy(&self) -> bool {
        self.drag.is_some_and(|d| d.began)
    }

    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        let p = ev.pos;
        match ev.kind {
            PointerKind::Down => {
                self.drag = None;
                let Some(r) = selection_bounds(cx) else { return vec![] };
                if let Some(op) = self.classify(cx, r, p, ev.mods) {
                    self.targets = op.handle().is_some().then(|| PointSnap::new(cx, || Targets::collect(cx.doc, &cx.selection.objects, None)));
                    self.drag = Some(Drag { op, rect: r, start: p, began: false });
                }
                vec![]
            }
            PointerKind::Drag => {
                let Some(mut d) = self.drag else { return vec![] };
                let mut out = vec![];
                // A handle drag follows the modifiers held now: the gesture is to start dragging
                // and then hold Cmd (a Cmd press on the canvas is the temporary Selection tool).
                if let Some(h) = d.op.handle() {
                    let op = self.handle_op(h, ev.mods);
                    if d.began && op.label() != d.op.label() {
                        // Another operation: it gets an undo step of its own.
                        out.push(Action::Cancel);
                        out.push(Action::Begin(op.label().into()));
                    }
                    d.op = op;
                }
                if !d.began {
                    if p.distance(d.start) < cx.tol(2.0) {
                        self.drag = Some(d);
                        return out;
                    }
                    d.began = true;
                    out.push(Action::Begin(d.op.label().into()));
                }
                self.drag = Some(d);
                out.push(self.preview(cx, d, p, ev.mods));
                out
            }
            PointerKind::Up => {
                let d = self.drag.take();
                self.quad = None;
                self.measure = None;
                self.guides.clear();
                self.targets = None;
                if d.is_some_and(|d| d.began) { vec![Action::Commit] } else { vec![] }
            }
            _ => vec![],
        }
    }

    fn key(&mut self, _cx: &ToolContext, key: ToolKey, _mods: Mods) -> Vec<Action> {
        if key == ToolKey::Escape && self.busy() {
            self.drag = None;
            self.quad = None;
            self.measure = None;
            self.guides.clear();
            self.targets = None;
            return vec![Action::Cancel];
        }
        vec![]
    }

    fn overlays(&self, cx: &ToolContext) -> Vec<Overlay> {
        let mut o = vec![];
        let quad = match (self.quad, selection_bounds(cx)) {
            (Some(q), _) => q,
            (None, Some(r)) => rect_corners(r),
            _ => return o,
        };
        o.push(Overlay::Path { path: polygon(&quad, true), color: BLUE, width: 1.0, dashed: false });
        for i in 0..4 {
            let (a, b) = (quad[i], quad[(i + 1) % 4]);
            o.push(Overlay::Anchor { p: a, color: BLUE, filled: false, size: 6.0 });
            if self.mode != DistortMode::Perspective {
                o.push(Overlay::Anchor { p: a.midpoint(b), color: BLUE, filled: false, size: 6.0 });
            }
        }
        o.extend(self.guides.iter().cloned());
        if let Some((p, t)) = &self.measure {
            o.push(Overlay::Measure { p: *p + Vec2::new(cx.tol(12.0), cx.tol(12.0)), text: t.clone() });
        }
        o
    }

    fn cursor(&self, cx: &ToolContext, p: Point, m: Mods) -> Cursor {
        let Some(r) = selection_bounds(cx) else { return Cursor::Arrow };
        match self.drag.map(|d| d.op).or_else(|| self.classify(cx, r, p, m)) {
            Some(Op::Rotate) => Cursor::rotate_about_in_view(r.center(), p, cx.view_angle()),
            Some(Op::Move) => Cursor::Move,
            Some(Op::Shear(Handle::Top | Handle::Bottom)) => Cursor::ShearH,
            Some(Op::Shear(_)) => Cursor::ShearV,
            Some(Op::Scale(h)) => match h {
                Handle::Top | Handle::Bottom => Cursor::ResizeV,
                Handle::Left | Handle::Right => Cursor::ResizeH,
                Handle::TopLeft | Handle::BottomRight => Cursor::ResizeNwSe,
                Handle::TopRight | Handle::BottomLeft => Cursor::ResizeNeSw,
            }
            .turned(cx.view_angle()),
            Some(Op::Corners(..)) => Cursor::Crosshair,
            None => Cursor::Arrow,
        }
    }

    fn options(&self) -> Value {
        json!({ "mode": self.mode.name(), "constrain": self.constrain })
    }

    fn set_option(&mut self, key: &str, value: &Value) {
        match key {
            "mode" => {
                self.mode = match value.as_str() {
                    Some("perspective") => DistortMode::Perspective,
                    Some("distort") | Some("freeDistort") => DistortMode::Distort,
                    _ => DistortMode::Free,
                }
            }
            "constrain" => self.constrain = value.as_bool().unwrap_or(false),
            _ => {}
        }
    }

    fn deactivate(&mut self, _cx: &ToolContext) -> Vec<Action> {
        self.quad = None;
        self.guides.clear();
        self.targets = None;
        if self.drag.take().is_some_and(|d| d.began) { vec![Action::Cancel] } else { vec![] }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::*;
    use vectorcraft_doc::Selection;

    fn ev(kind: PointerKind, x: f64, y: f64) -> PointerEvent {
        PointerEvent::new(kind, x, y)
    }

    #[test]
    fn corner_scales_and_inside_moves() {
        let (d, id) = doc_with_rect();
        let mut s = Selection::default();
        s.add(id);
        let p = paint();
        let cx = cx(&d, &s, &p);
        let mut t = FreeTransformTool::default();
        t.pointer(&cx, &ev(PointerKind::Down, 200.0, 200.0));
        let a = t.pointer(&cx, &ev(PointerKind::Drag, 300.0, 300.0));
        assert_eq!(a[0], Action::Begin("Scale".into()));
        assert!(matches!(&a[1], Action::Preview(c, v) if c == "object.transform" && v["matrix"][0] == 2.0));
        assert_eq!(t.pointer(&cx, &ev(PointerKind::Up, 300.0, 300.0)), vec![Action::Commit]);
        t.pointer(&cx, &ev(PointerKind::Down, 150.0, 150.0));
        let a = t.pointer(&cx, &ev(PointerKind::Drag, 160.0, 170.0));
        assert_eq!(a[0], Action::Begin("Move".into()));
        assert!(matches!(&a[1], Action::Preview(_, v) if v["matrix"][4] == 10.0 && v["matrix"][5] == 20.0));
    }

    /// #2054: with Snap to Grid on, a handle drag lands the side it moves on the nearest gridline
    /// (every 9 pt by default) while the opposite side stays put.
    #[test]
    fn handle_drag_snaps_to_the_grid() {
        let (d, id) = doc_with_rect();
        let mut s = Selection::default();
        s.add(id);
        let p = paint();
        let cx = ToolContext { snap_to_grid: true, ..cx(&d, &s, &p) };
        let matrix = |a: &[Action]| {
            let Some(Action::Preview(_, v)) = a.last() else { panic!("{a:?}") };
            let m: Vec<f64> = v["matrix"].as_array().unwrap().iter().map(|x| x.as_f64().unwrap()).collect();
            Affine::new([m[0], m[1], m[2], m[3], m[4], m[5]])
        };
        let mut t = FreeTransformTool::default();
        t.pointer(&cx, &ev(PointerKind::Down, 200.0, 150.0));
        let r = matrix(&t.pointer(&cx, &ev(PointerKind::Drag, 213.0, 150.0))).transform_rect_bbox(Rect::new(100.0, 100.0, 200.0, 200.0));
        assert!((r.x1 - 216.0).abs() < 1e-9 && (r.x0 - 100.0).abs() < 1e-9, "{r:?}");
        assert_eq!(t.pointer(&cx, &ev(PointerKind::Up, 213.0, 150.0)), vec![Action::Commit]);
    }

    /// In a view turned 90° the handles' cursors turn with the view.
    #[test]
    fn handle_cursors_turn_with_the_view() {
        let (d, id) = doc_with_rect();
        let mut s = Selection::default();
        s.add(id);
        let p = paint();
        let mut c = cx(&d, &s, &p);
        c.screen = turned_view(90.0);
        let t = FreeTransformTool::default();
        assert_eq!(t.cursor(&c, Point::new(150.0, 100.0), Mods::default()), Cursor::ResizeH, "top-middle");
        assert_eq!(t.cursor(&c, Point::new(200.0, 200.0), Mods::default()), Cursor::ResizeNeSw, "bottom-right");
        assert_eq!(t.cursor(&c, Point::new(207.0, 93.0), Mods::default()), Cursor::RotateToward(3), "out from the top right");
    }

    #[test]
    fn cmd_side_handle_shears() {
        let (d, id) = doc_with_rect();
        let mut s = Selection::default();
        s.add(id);
        let p = paint();
        let cx = cx(&d, &s, &p);
        let mut t = FreeTransformTool::default();
        let cmd = Mods { cmd: true, ..Default::default() };
        t.pointer(&cx, &ev(PointerKind::Down, 150.0, 100.0).with_mods(cmd));
        let a = t.pointer(&cx, &ev(PointerKind::Drag, 170.0, 100.0).with_mods(cmd));
        assert_eq!(a[0], Action::Begin("Shear".into()));
        // Bottom edge fixed, top edge moved 20 right.
        let Action::Preview(_, v) = &a[1] else { panic!() };
        let c: Vec<f64> = v["matrix"].as_array().unwrap().iter().map(|x| x.as_f64().unwrap()).collect();
        let m = Affine::new([c[0], c[1], c[2], c[3], c[4], c[5]]);
        assert!((m * Point::new(100.0, 200.0)).distance(Point::new(100.0, 200.0)) < 1e-9);
        assert!((m * Point::new(100.0, 100.0)).distance(Point::new(120.0, 100.0)) < 1e-9);
    }

    #[test]
    fn cmd_over_a_side_shows_the_shear_cursor_along_it() {
        let (d, id) = doc_with_rect();
        let mut s = Selection::default();
        s.add(id);
        let p = paint();
        let cx = cx(&d, &s, &p);
        let mut t = FreeTransformTool::default();
        let cmd = Mods { cmd: true, ..Default::default() };
        let (top, left) = (Point::new(150.0, 100.0), Point::new(100.0, 150.0));
        assert_eq!(t.cursor(&cx, top, cmd), Cursor::ShearH);
        assert_eq!(t.cursor(&cx, left, cmd), Cursor::ShearV);
        // Without Cmd the sides scale; corners never shear.
        assert_eq!(t.cursor(&cx, top, Mods::default()), Cursor::ResizeV);
        assert_eq!(t.cursor(&cx, Point::new(200.0, 200.0), cmd), Cursor::Crosshair);
        // The cursor stays while the shear is dragged.
        t.pointer(&cx, &ev(PointerKind::Down, 150.0, 100.0).with_mods(cmd));
        t.pointer(&cx, &ev(PointerKind::Drag, 170.0, 100.0).with_mods(cmd));
        assert_eq!(t.cursor(&cx, Point::new(170.0, 100.0), cmd), Cursor::ShearH);
    }

    #[test]
    fn modifiers_pressed_during_a_handle_drag_change_what_it_does() {
        let (d, id) = doc_with_rect();
        let mut s = Selection::default();
        s.add(id);
        let p = paint();
        let cx = cx(&d, &s, &p);
        let mut t = FreeTransformTool::default();
        let cmd = Mods { cmd: true, ..Default::default() };
        let perspective = Mods { cmd: true, alt: true, shift: true, ..Default::default() };
        // Start dragging the bottom-right corner, then hold Cmd: Free Distort, a step of its own.
        t.pointer(&cx, &ev(PointerKind::Down, 200.0, 200.0));
        let a = t.pointer(&cx, &ev(PointerKind::Drag, 210.0, 210.0));
        assert_eq!(a[0], Action::Begin("Scale".into()));
        let a = t.pointer(&cx, &ev(PointerKind::Drag, 220.0, 230.0).with_mods(cmd));
        let corners = json!({"corners": [[100.0, 100.0], [200.0, 100.0], [220.0, 230.0], [100.0, 200.0]], "from": [100.0, 100.0, 200.0, 200.0]});
        assert_eq!(a, vec![Action::Cancel, Action::Begin("Distort".into()), Action::Preview("object.distort".into(), corners)]);
        // Cmd+Alt+Shift: Perspective Distort, still the same step.
        let a = t.pointer(&cx, &ev(PointerKind::Drag, 220.0, 230.0).with_mods(perspective));
        assert!(matches!(&a[..], [Action::Preview(c, _)] if c == "object.distort"), "{a:?}");
        // Letting go of the keys scales again.
        let a = t.pointer(&cx, &ev(PointerKind::Drag, 220.0, 230.0));
        assert_eq!(a[..2], [Action::Cancel, Action::Begin("Scale".into())]);
        assert_eq!(t.pointer(&cx, &ev(PointerKind::Up, 220.0, 230.0)), vec![Action::Commit]);
        // Cmd held during a side handle's drag shears.
        t.pointer(&cx, &ev(PointerKind::Down, 150.0, 100.0));
        t.pointer(&cx, &ev(PointerKind::Drag, 155.0, 100.0));
        let a = t.pointer(&cx, &ev(PointerKind::Drag, 170.0, 100.0).with_mods(cmd));
        assert_eq!(a[..2], [Action::Cancel, Action::Begin("Shear".into())]);
    }

    #[test]
    fn distort_modes_emit_corners() {
        let (d, id) = doc_with_rect();
        let mut s = Selection::default();
        s.add(id);
        let p = paint();
        let cx = cx(&d, &s, &p);
        let mut t = FreeTransformTool::default();
        t.set_option("mode", &json!("distort"));
        assert_eq!(t.options()["mode"], "distort");
        t.pointer(&cx, &ev(PointerKind::Down, 200.0, 200.0));
        let a = t.pointer(&cx, &ev(PointerKind::Drag, 220.0, 230.0));
        assert_eq!(a[0], Action::Begin("Distort".into()));
        assert_eq!(
            a[1],
            Action::Preview(
                "object.distort".into(),
                json!({"corners": [[100.0, 100.0], [200.0, 100.0], [220.0, 230.0], [100.0, 200.0]], "from": [100.0, 100.0, 200.0, 200.0]})
            )
        );
        t.pointer(&cx, &ev(PointerKind::Up, 220.0, 230.0));
        let q = distort_quad(Rect::new(0.0, 0.0, 10.0, 10.0), Handle::TopLeft, Vec2::new(2.0, 0.0), DistortMode::Perspective);
        assert_eq!(q[0], Point::new(2.0, 0.0));
        assert_eq!(q[1], Point::new(8.0, 0.0));
        assert_eq!(q[2], Point::new(10.0, 10.0));
    }
}
