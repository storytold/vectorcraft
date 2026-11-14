//! Free Transform tool (E).
//!
//! Shows the bounding box with handles. In the default mode handles scale (Shift proportional,
//! Alt from centre), outside the corners rotates, inside moves, and Cmd-dragging a side handle
//! shears along that side. A scale snaps as the Selection tool's does. The Perspective Distort and Free Distort modes (tool option `mode`, or
//! Cmd / Cmd+Alt+Shift while dragging a corner like classic Illustrator) move the corners of the
//! box and preview `object.distort`; in Free Distort, Shift or the Constrain option keeps a dragged
//! corner on the diagonal through the opposite corner. After a rotation the box stays turned with
//! the selection, as the Selection tool's does: its handles scale, shear and distort along the selection's own sides.

use serde_json::{Value, json};
use vectorcraft_doc::OrientedBox;
use vectorcraft_geom::{Affine, Point, Rect, Vec2};

use super::{BLUE, polygon, rect_corners};
use crate::bbox::{Handle, hit_handle, in_rotate_zone, move_delta, rotate_for_drag, scale_for_drag};
use crate::guides::{PointSnap, Targets};
use crate::select::{handle_cursor, matrix_json, screen_handle, selection_box};
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
    /// The selection's bounding box at the press, turned with the selection.
    bx: OrientedBox,
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

/// The part of drag `d` that lies along the diagonal from corner `h` of `r` to the opposite
/// corner. A Free Distort corner dragged with Shift or Constrain moves only along that line, so its
/// offset from the opposite corner keeps the box's width-to-height ratio. A box with no width and no height gives no movement.
pub fn along_diagonal(r: Rect, h: Handle, d: Vec2) -> Vec2 {
    let u = h.pos(r) - h.opposite().pos(r);
    let len2 = u.hypot2();
    if len2 <= 1e-18 || !len2.is_finite() {
        return Vec2::ZERO;
    }
    u * (d.dot(u) / len2)
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

    /// What a press at `p` on box `bx` does, read in the box's own frame.
    fn classify(&self, cx: &ToolContext, bx: &OrientedBox, p: Point, m: Mods) -> Option<Op> {
        let (tol, r, lp) = (cx.tol(5.0), bx.rect, bx.to_local(p));
        if let Some(h) = hit_handle(r, lp, tol) {
            return Some(self.handle_op(h, m));
        }
        if self.mode == DistortMode::Free && in_rotate_zone(r, lp, tol, cx.tol(18.0)).is_some() {
            return Some(Op::Rotate);
        }
        r.contains(lp).then_some(Op::Move)
    }

    fn preview(&mut self, cx: &ToolContext, d: Drag, p: Point, m: Mods) -> Action {
        let shift = m.shift || self.constrain;
        let bx = d.bx;
        let delta = p - d.start;
        // The drag along the box's own sides.
        let local = bx.to_local(p) - bx.to_local(d.start);
        self.guides.clear();
        let xf = match d.op {
            Op::Corners(h, mode) => {
                // Shift or Constrain keeps a Free Distort corner on its diagonal.
                let constrained = shift && mode == DistortMode::Distort && h.is_corner();
                let local = if constrained { along_diagonal(bx.rect, h, local) } else { local };
                let q = distort_quad(bx.rect, h, local, mode).map(|c| bx.to_doc() * c);
                self.quad = Some(q);
                let moved = if constrained { bx.to_doc() * (h.pos(bx.rect) + local) - bx.to_doc() * h.pos(bx.rect) } else { delta };
                self.measure = cx.measurement_labels.then(|| (p, cx.offset_label(moved.x, moved.y)));
                let corners: Vec<Value> = q.iter().map(|c| json!([c.x, c.y])).collect();
                let r = bx.rect;
                let mut params = json!({ "corners": corners, "from": [r.x0, r.y0, r.x1, r.y1] });
                if bx.angle != 0.0 {
                    params["angle"] = json!(bx.angle);
                }
                return Action::Preview("object.distort".into(), params);
            }
            Op::Move => {
                let v = move_delta(d.start, p, shift, cx.constrain_angle);
                self.measure = cx.measurement_labels.then(|| (p, cx.offset_label(v.x, v.y)));
                Affine::translate(v)
            }
            Op::Scale(h) => {
                // Scaled in the box's own frame, then turned back onto the page.
                let mut a = scale_for_drag(bx.rect, h, bx.to_local(p), shift, m.alt);
                if let Some(t) = &self.targets {
                    (a, self.guides) = t.snap_scale(cx, &bx, h, a, shift, m.alt);
                }
                let nr = a.transform_rect_bbox(bx.rect);
                self.measure = cx.transform_tools_guides.then(|| (p, cx.size_label(nr.width(), nr.height())));
                bx.conjugate(a)
            }
            Op::Shear(h) => bx.conjugate(shear_for_side(bx.rect, h, local, m.alt)),
            Op::Rotate => {
                let (a, deg) = rotate_for_drag(bx.center(), d.start, p, shift);
                self.measure = cx.transform_tools_guides.then(|| (p, format!("{:.1}°", -deg)));
                a
            }
        };
        self.quad = Some(bx.corners().map(|c| xf * c));
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
                let Some(bx) = selection_box(cx) else { return vec![] };
                if let Some(op) = self.classify(cx, &bx, p, ev.mods) {
                    self.targets = op.handle().is_some().then(|| PointSnap::scaling(cx, || Targets::for_scale(cx)));
                    self.drag = Some(Drag { op, bx, start: p, began: false });
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
        let quad = match self.quad {
            Some(q) => q,
            None => match selection_box(cx) {
                Some(bx) => bx.corners(),
                None => return o,
            },
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
        let Some(bx) = self.drag.map(|d| d.bx).or_else(|| selection_box(cx)) else { return Cursor::Arrow };
        // The box's angle as it shows on screen.
        let shown = bx.angle - cx.view_angle();
        match self.drag.map(|d| d.op).or_else(|| self.classify(cx, &bx, p, m)) {
            Some(Op::Rotate) => Cursor::rotate_about_in_view(bx.center(), p, cx.view_angle()),
            Some(Op::Move) => Cursor::Move,
            Some(Op::Shear(h)) => match screen_handle(h, shown) {
                Handle::Top | Handle::Bottom | Handle::TopLeft | Handle::BottomRight => Cursor::ShearH,
                _ => Cursor::ShearV,
            },
            Some(Op::Scale(h)) => handle_cursor(h, shown),
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

    fn check_option(&self, key: &str, value: &Value) -> Result<(), String> {
        use crate::option_check as c;
        match key {
            "mode" => c::one_of(key, value, &["free", "perspective", "distort", "freeDistort"]),
            "constrain" => c::boolean(key, value),
            _ => Err(c::unknown(key)),
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

    /// After a 30° turn the box, its handles and their cursors turn with the selection, and a side
    /// handle scales along the selection's own width.
    #[test]
    fn the_box_and_its_handles_turn_with_the_selection() {
        let (mut d, id) = doc_with_rect();
        let c = Vec2::new(150.0, 150.0);
        let turn = Affine::translate(c) * Affine::rotate(-30f64.to_radians()) * Affine::translate(-c);
        d.node_mut(id).unwrap().transform(turn, false);
        let mut s = Selection::default();
        s.add(id);
        let p = paint();
        let cx = cx(&d, &s, &p);
        let bx = selection_box(&cx).unwrap();
        assert!((bx.angle - 30.0).abs() < 1e-9, "{bx:?}");
        let mut t = FreeTransformTool::default();
        let shown: Vec<Point> = t.overlays(&cx).iter().filter_map(|o| if let Overlay::Anchor { p, .. } = o { Some(*p) } else { None }).collect();
        for corner in bx.corners() {
            assert!(shown.iter().any(|q| q.distance(corner) < 1e-9), "{corner:?} not in {shown:?}");
        }
        let top = bx.to_doc() * Handle::Top.pos(bx.rect);
        assert_eq!(t.cursor(&cx, top, Mods::default()), Cursor::ResizeNwSe, "the top handle, turned 30°");
        // The Right handle dragged 100 pt out along the selection's own x axis doubles its width.
        let right = bx.to_doc() * Handle::Right.pos(bx.rect);
        let to = bx.to_doc() * (Handle::Right.pos(bx.rect) + Vec2::new(100.0, 0.0));
        t.pointer(&cx, &ev(PointerKind::Down, right.x, right.y));
        let a = t.pointer(&cx, &ev(PointerKind::Drag, to.x, to.y));
        assert_eq!(a[0], Action::Begin("Scale".into()));
        let Some(Action::Preview(_, v)) = a.last() else { panic!("{a:?}") };
        let m: Vec<f64> = v["matrix"].as_array().unwrap().iter().map(|x| x.as_f64().unwrap()).collect();
        let m = Affine::new([m[0], m[1], m[2], m[3], m[4], m[5]]);
        let local = bx.to_doc().inverse() * m * bx.to_doc();
        let want = Affine::translate((bx.rect.x0, 0.0)) * Affine::scale_non_uniform(2.0, 1.0) * Affine::translate((-bx.rect.x0, 0.0));
        for (got, want) in local.as_coeffs().iter().zip(want.as_coeffs()) {
            assert!((got - want).abs() < 1e-9, "{local:?}");
        }
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

    /// The corners an `object.distort` preview sends.
    fn sent_corners(a: &[Action]) -> Vec<Point> {
        let Some(Action::Preview(c, v)) = a.last() else { panic!("{a:?}") };
        assert_eq!(c, "object.distort");
        v["corners"].as_array().unwrap().iter().map(|p| Point::new(p[0].as_f64().unwrap(), p[1].as_f64().unwrap())).collect()
    }

    fn assert_corners(got: &[Point], want: [(f64, f64); 4]) {
        for (g, w) in got.iter().zip(want) {
            assert!(g.distance(Point::new(w.0, w.1)) < 1e-9, "{got:?} != {want:?}");
        }
    }

    /// A 200 × 100 rectangle at (100, 100)–(300, 200), selected.
    fn wide_rect() -> (vectorcraft_doc::Document, Selection, vectorcraft_doc::NodeId) {
        let (mut d, id) = doc_with_rect();
        let wide = Affine::translate((100.0, 100.0)) * Affine::scale_non_uniform(2.0, 1.0) * Affine::translate((-100.0, -100.0));
        d.node_mut(id).unwrap().transform(wide, false);
        let mut s = Selection::default();
        s.add(id);
        (d, s, id)
    }

    /// In Free Distort, Shift or the Constrain option keeps the dragged corner on the diagonal
    /// through the opposite corner; the other three corners stay put.
    #[test]
    fn free_distort_corner_keeps_to_its_diagonal_with_shift_or_constrain() {
        let (d, s, _) = wide_rect();
        let p = paint();
        let cx = cx(&d, &s, &p);
        let shift = Mods { shift: true, ..Default::default() };
        // The bottom-right corner dragged by (40, 60): along the diagonal (200, 100) that is
        // (40·200 + 60·100) / (200² + 100²) = 0.28 of it, (56, 28).
        let fixed = [(100.0, 100.0), (300.0, 100.0), (356.0, 228.0), (100.0, 200.0)];
        let drag = |t: &mut FreeTransformTool, m: Mods| {
            t.pointer(&cx, &ev(PointerKind::Down, 300.0, 200.0).with_mods(m));
            let a = t.pointer(&cx, &ev(PointerKind::Drag, 340.0, 260.0).with_mods(m));
            t.pointer(&cx, &ev(PointerKind::Up, 340.0, 260.0).with_mods(m));
            sent_corners(&a)
        };
        let mut t = FreeTransformTool::default();
        t.set_option("mode", &json!("distort"));
        assert_corners(&drag(&mut t, Mods::default()), [(100.0, 100.0), (300.0, 100.0), (340.0, 260.0), (100.0, 200.0)]);
        assert_corners(&drag(&mut t, shift), fixed);
        t.set_option("constrain", &json!(true));
        assert_corners(&drag(&mut t, Mods::default()), fixed);
        // Cmd+Shift on a corner in the Free Transform mode is Free Distort, constrained too.
        let mut t = FreeTransformTool::default();
        assert_corners(&drag(&mut t, Mods { cmd: true, shift: true, ..Default::default() }), fixed);
        // Cmd+Alt+Shift stays Perspective Distort, with Constrain on as well.
        t.set_option("constrain", &json!(true));
        let perspective = Mods { cmd: true, alt: true, shift: true, ..Default::default() };
        assert_corners(&drag(&mut t, perspective), [(100.0, 100.0), (300.0, 40.0), (300.0, 260.0), (100.0, 200.0)]);
        // The measurement label shows how far the corner moved.
        let cx = ToolContext { measurement_labels: true, ..cx };
        let mut t = FreeTransformTool::default();
        t.set_option("mode", &json!("distort"));
        t.pointer(&cx, &ev(PointerKind::Down, 300.0, 200.0).with_mods(shift));
        t.pointer(&cx, &ev(PointerKind::Drag, 340.0, 260.0).with_mods(shift));
        let want = cx.offset_label(56.0, 28.0);
        assert!(t.overlays(&cx).iter().any(|o| matches!(o, Overlay::Measure { text, .. } if text == &want)), "{want}");
    }

    /// Shift pressed in the middle of a Free Distort corner drag constrains it from then on, in the
    /// same undo step; letting go frees the corner again.
    #[test]
    fn shift_pressed_during_a_free_distort_drag_constrains_it() {
        let (d, s, _) = wide_rect();
        let p = paint();
        let cx = cx(&d, &s, &p);
        let mut t = FreeTransformTool::default();
        t.set_option("mode", &json!("distort"));
        t.pointer(&cx, &ev(PointerKind::Down, 300.0, 200.0));
        let a = t.pointer(&cx, &ev(PointerKind::Drag, 310.0, 210.0));
        assert_eq!(a[0], Action::Begin("Distort".into()));
        let a = t.pointer(&cx, &ev(PointerKind::Drag, 340.0, 260.0).with_mods(Mods { shift: true, ..Default::default() }));
        assert_eq!(a.len(), 1, "no new undo step: {a:?}");
        assert_corners(&sent_corners(&a), [(100.0, 100.0), (300.0, 100.0), (356.0, 228.0), (100.0, 200.0)]);
        let a = t.pointer(&cx, &ev(PointerKind::Drag, 340.0, 260.0));
        assert_corners(&sent_corners(&a), [(100.0, 100.0), (300.0, 100.0), (340.0, 260.0), (100.0, 200.0)]);
        assert_eq!(t.pointer(&cx, &ev(PointerKind::Up, 340.0, 260.0)), vec![Action::Commit]);
    }

    /// A turned box is constrained along its own diagonal.
    #[test]
    fn a_turned_box_is_constrained_along_its_own_diagonal() {
        let (mut d, s, id) = wide_rect();
        let c = Vec2::new(200.0, 150.0);
        let turn = Affine::translate(c) * Affine::rotate(-30f64.to_radians()) * Affine::translate(-c);
        d.node_mut(id).unwrap().transform(turn, false);
        let p = paint();
        let cx = cx(&d, &s, &p);
        let bx = selection_box(&cx).unwrap();
        assert!((bx.rect.width() - 200.0).abs() < 1e-9 && (bx.rect.height() - 100.0).abs() < 1e-9, "{bx:?}");
        let corner = Handle::BottomRight.pos(bx.rect);
        let (from, to) = (bx.to_doc() * corner, bx.to_doc() * (corner + Vec2::new(40.0, 60.0)));
        let mut t = FreeTransformTool::default();
        t.set_option("mode", &json!("distort"));
        t.set_option("constrain", &json!(true));
        t.pointer(&cx, &ev(PointerKind::Down, from.x, from.y));
        let got = sent_corners(&t.pointer(&cx, &ev(PointerKind::Drag, to.x, to.y)));
        let want = bx.to_doc() * (corner + Vec2::new(56.0, 28.0));
        assert!(got[2].distance(want) < 1e-9, "{got:?} {want:?}");
        for (i, k) in [(0, Handle::TopLeft), (1, Handle::TopRight), (3, Handle::BottomLeft)] {
            assert!(got[i].distance(bx.to_doc() * k.pos(bx.rect)) < 1e-9, "{got:?}");
        }
    }

    /// A box with no size has no diagonal to move along; a line keeps to its own length.
    #[test]
    fn along_diagonal_of_a_degenerate_box() {
        assert_eq!(along_diagonal(Rect::new(5.0, 5.0, 5.0, 5.0), Handle::BottomRight, Vec2::new(3.0, 4.0)), Vec2::ZERO);
        assert_eq!(along_diagonal(Rect::new(5.0, 0.0, 5.0, 10.0), Handle::BottomRight, Vec2::new(3.0, 4.0)), Vec2::new(0.0, 4.0));
        let d = along_diagonal(Rect::new(0.0, 0.0, 10.0, 10.0), Handle::TopLeft, Vec2::new(1.0, 0.0));
        assert!(d.x.is_finite() && (d - Vec2::new(0.5, 0.5)).hypot() < 1e-12, "{d:?}");
    }

    #[test]
    fn distort_drag_shows_move_measurement_labels() {
        let (d, id) = doc_with_rect();
        let mut s = Selection::default();
        s.add(id);
        let p = paint();
        let cx = |on: bool| ToolContext { measurement_labels: on, ..cx(&d, &s, &p) };
        let mut t = FreeTransformTool::default();
        t.set_option("mode", &json!("distort"));
        t.pointer(&cx(true), &ev(PointerKind::Down, 200.0, 200.0));
        t.pointer(&cx(true), &ev(PointerKind::Drag, 220.0, 230.0));
        let want = cx(true).offset_label(20.0, 30.0);
        assert!(t.overlays(&cx(true)).iter().any(|o| matches!(o, Overlay::Measure { text, .. } if text == &want)));
        t.pointer(&cx(true), &ev(PointerKind::Up, 220.0, 230.0));
        t.pointer(&cx(false), &ev(PointerKind::Down, 200.0, 200.0));
        t.pointer(&cx(false), &ev(PointerKind::Drag, 220.0, 230.0));
        assert!(t.overlays(&cx(false)).iter().all(|o| !matches!(o, Overlay::Measure { .. })));
    }
}
