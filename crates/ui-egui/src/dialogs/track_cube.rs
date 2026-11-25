//! The track cube on the Rotation tab of Extrude & Bevel (Classic), Revolve (Classic) and Rotate
//! (Classic): a cube drawn at the current X, Y and Z rotation inside a ring, dragged or keyed to turn the object.

// PLAN: assumed the app depends on craft-behavior and craft-ui-patterns (hub-w2 owns Cargo.toml).
use craft::behavior::{CursorIcon, CursorPriority, Mixed, PartKey, PointerButton, PressPhase, Sense};
use craft::ui::contract::{
    Access, Bind, BindMut, Common, Component, Cx, DisabledLook, Edit as _, EditResponse, Focus as _, Input, MeasureCx, Name, PaintUi, PartsCx,
    Pointer as _, Props as _, Remember as _, Role, Show as _,
};
use craft::ui::patterns::form_dialogs::Param;
use craft::ui::runtime::focus::{ActivateKeys, KeepKeys, KeyUse};
use craft::ui::style::StyleCx;
use egui::{Color32, Pos2, Rect, Stroke, Vec2, vec2};
use serde_json::json;

use super::revolve_lights::RowStyle;

/// The settings the track cube turns, as every 3D effect names them.
const ROTATION_KEYS: [&str; 3] = ["rotationX", "rotationY", "rotationZ"];
/// The track cube's box side, its cube's half edge, and its ring's inner and outer radius (points).
const CUBE_BOX: f32 = 124.0;
const CUBE_HALF: f32 = 25.0;
const RING_IN: f32 = 47.0;
const RING_OUT: f32 = 60.0;
/// How far an edge reaches for the pointer (points).
const EDGE_REACH: f32 = 5.0;
/// Degrees one point of pointer movement turns the cube.
const DEGREES_PER_POINT: f64 = 0.5;
/// The cube's one part: its box. The grip a press takes is chosen from the pointer, as edges, faces and the ring overlap.
const CUBE: PartKey = PartKey::new("cube");

/// A 3×3 rotation, rows first.
type M3 = [[f64; 3]; 3];

/// The rotation that X, Y and Z angles in degrees make, as the 3D effects apply them: X first, then
/// Y, then Z, in view space (x right, y down, z toward the viewer).
fn rotation(angles: [f64; 3]) -> M3 {
    let [(sx, cx), (sy, cy), (sz, cz)] = angles.map(|a| a.to_radians().sin_cos());
    [[cz * cy, cz * sy * sx - sz * cx, cz * sy * cx + sz * sx], [sz * cy, sz * sy * sx + cz * cx, sz * sy * cx - cz * sx], [-sy, cy * sx, cy * cx]]
}

fn multiply(a: M3, b: M3) -> M3 {
    std::array::from_fn(|i| std::array::from_fn(|j| (0..3).map(|k| a[i][k] * b[k][j]).sum()))
}

/// The unit vector along axis `axis` (0 X, 1 Y, 2 Z).
fn unit(axis: usize) -> [f64; 3] {
    std::array::from_fn(|i| if i == axis { 1.0 } else { 0.0 })
}

/// A turn of `degrees` about the unit vector `axis`.
fn turn(axis: [f64; 3], degrees: f64) -> M3 {
    let (s, c) = degrees.to_radians().sin_cos();
    let t = 1.0 - c;
    let [x, y, z] = axis;
    [
        [t * x * x + c, t * x * y - s * z, t * x * z + s * y],
        [t * x * y + s * z, t * y * y + c, t * y * z - s * x],
        [t * x * z - s * y, t * y * z + s * x, t * z * z + c],
    ]
}

/// An angle as the rotation fields hold it: above -180° up to 180°, to a hundredth of a degree.
fn tidy(degrees: f64) -> f64 {
    let wrapped = 180.0 - (180.0 - degrees).rem_euclid(360.0);
    (wrapped * 100.0).round() / 100.0 + 0.0
}

/// The X, Y and Z angles of rotation `m`. Of the two sets of angles that make it, the one nearest
/// `near` (so a turn moves the fields as little as it can); with Y at ±90°, where X and Z turn about
/// the same axis, X keeps its value from `near`.
fn angles_of(m: M3, near: [f64; 3]) -> [f64; 3] {
    let y = (-m[2][0]).clamp(-1.0, 1.0).asin().to_degrees();
    let candidates = if m[2][1].hypot(m[2][2]) < 1e-9 {
        let x = near[0];
        let z = if y > 0.0 { x - m[0][1].atan2(m[1][1]).to_degrees() } else { (-m[0][1]).atan2(m[1][1]).to_degrees() - x };
        [Some([x, y, z]), None]
    } else {
        let x = m[2][1].atan2(m[2][2]).to_degrees();
        let z = m[1][0].atan2(m[0][0]).to_degrees();
        [Some([x, y, z]), Some([x + 180.0, 180.0 - y, z + 180.0])]
    };
    let distance = |a: &[f64; 3]| a.iter().zip(near).map(|(a, b)| tidy(a - b).abs()).sum::<f64>();
    candidates.into_iter().flatten().min_by(|a, b| distance(a).total_cmp(&distance(b))).unwrap_or(near).map(tidy)
}

/// Where corner `i` of the cube (bit 0 for +X, bit 1 for +Y, bit 2 for +Z) is drawn.
fn corner(m: M3, center: Pos2, i: usize) -> Pos2 {
    let p: [f64; 3] = std::array::from_fn(|b| if i >> b & 1 == 1 { 1.0 } else { -1.0 });
    let v: [f64; 3] = std::array::from_fn(|r| (0..3).map(|k| m[r][k] * p[k]).sum());
    center + vec2(v[0] as f32, v[1] as f32) * CUBE_HALF
}

/// The corners of the face square to `axis` on its positive or negative side, in order round it.
fn face_corners(axis: usize, positive: bool) -> [usize; 4] {
    let (b, c) = (1 << ((axis + 1) % 3), 1 << ((axis + 2) % 3));
    let side = if positive { 1 << axis } else { 0 };
    [0, b, b | c, c].map(|k| side | k)
}

/// Does the face square to `axis` on that side face the viewer?
fn face_visible(m: M3, axis: usize, positive: bool) -> bool {
    m[2][axis] * if positive { 1.0 } else { -1.0 } > 1e-6
}

/// The 12 edges: (from corner, to corner, the object axis it runs along).
fn edges() -> impl Iterator<Item = (usize, usize, usize)> {
    (0..3).flat_map(|axis| (0..8).filter(move |i| i >> axis & 1 == 0).map(move |i| (i, i | 1 << axis, axis)))
}

/// Is the edge from corner `i` along `axis` on a face the viewer sees?
fn edge_visible(m: M3, i: usize, axis: usize) -> bool {
    [(axis + 1) % 3, (axis + 2) % 3].into_iter().any(|other| face_visible(m, other, i >> other & 1 == 1))
}

/// What a press on the track cube takes hold of.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum Grip {
    /// A face: a free turn, or with Shift about the screen's horizontal or vertical axis.
    #[default]
    Face,
    /// An edge: a turn about the object axis the edge runs along.
    Edge(usize),
    /// The ring round the cube: a turn in the plane of the screen.
    Ring,
}

/// The z of the cross product of two screen vectors: positive when `b` is clockwise from `a`.
fn cross(a: Vec2, b: Vec2) -> f32 {
    a.x * b.y - a.y * b.x
}

fn segment_distance(p: Pos2, a: Pos2, b: Pos2) -> f32 {
    let line = b - a;
    let t = ((p - a).dot(line) / line.length_sq().max(1e-6)).clamp(0.0, 1.0);
    p.distance(a + line * t)
}

/// What the pointer at `p` takes hold of on a cube drawn at `m` about `center`: the nearest edge
/// within reach, else a face, else the ring, else (in the box's corners) a face's free turn.
fn grip_at(m: M3, center: Pos2, p: Pos2) -> Grip {
    let edge = edges()
        .filter(|&(i, _, axis)| edge_visible(m, i, axis))
        .map(|(i, j, axis)| (segment_distance(p, corner(m, center, i), corner(m, center, j)), axis))
        .filter(|(distance, _)| *distance <= EDGE_REACH)
        .min_by(|a, b| a.0.total_cmp(&b.0));
    if let Some((_, axis)) = edge {
        return Grip::Edge(axis);
    }
    let on_face = (0..3).flat_map(|axis| [(axis, true), (axis, false)]).filter(|&(axis, side)| face_visible(m, axis, side)).any(|(axis, side)| {
        let points = face_corners(axis, side).map(|i| corner(m, center, i));
        let sides: [f32; 4] = std::array::from_fn(|k| cross(points[(k + 1) % 4] - points[k], p - points[k]));
        sides.iter().all(|s| *s >= 0.0) || sides.iter().all(|s| *s <= 0.0)
    });
    let distance = p.distance(center);
    if !on_face && (RING_IN - 3.0..=RING_OUT + 3.0).contains(&distance) { Grip::Ring } else { Grip::Face }
}

/// The turn in degrees, clockwise on the screen, from `from` to `to` about `center`.
fn sweep(from: Pos2, to: Pos2, center: Pos2) -> f64 {
    let (a, b) = (from - center, to - center);
    if a.length() < 2.0 || b.length() < 2.0 {
        return 0.0;
    }
    f64::from(cross(a, b)).atan2(f64::from(a.dot(b))).to_degrees()
}

/// A track cube gesture in progress.
#[derive(Clone, Copy, Default)]
struct CubeDrag {
    grip: Grip,
    /// The rotation so far, kept unrounded so a long drag does not drift.
    matrix: M3,
    origin: Pos2,
    last: Pos2,
    /// With Shift, the screen axis the drag keeps to (0 X, 1 Y), chosen once it has moved 3 points.
    lock: Option<usize>,
}

/// The rotation after the pointer moves from `drag.last` to `p` on a cube about `center`.
fn drag_turn(drag: &mut CubeDrag, p: Pos2, center: Pos2, shift: bool) -> M3 {
    let m = drag.matrix;
    let delta = p - drag.last;
    let (mut dx, mut dy) = (f64::from(delta.x), f64::from(delta.y));
    match drag.grip {
        Grip::Face => {
            if shift {
                let total = p - drag.origin;
                if drag.lock.is_none() && total.length() >= 3.0 {
                    drag.lock = Some(usize::from(total.x.abs() >= total.y.abs()));
                }
                match drag.lock {
                    // Across the screen turns about its vertical (Y) axis, up and down about X.
                    Some(1) => dy = 0.0,
                    Some(_) => dx = 0.0,
                    None => return m,
                }
            }
            let length = dx.hypot(dy);
            if length < 1e-9 {
                return m;
            }
            // The front follows the pointer: right turns it toward +X, down toward +Y.
            multiply(turn([-dy / length, dx / length, 0.0], length * DEGREES_PER_POINT), m)
        }
        Grip::Ring => multiply(turn(unit(2), sweep(drag.last, p, center)), m),
        Grip::Edge(axis) => {
            let w = [m[0][axis], m[1][axis], m[2][axis]];
            let across = w[0].hypot(w[1]);
            // An axis pointing at the viewer turns like the ring, on the side it points to.
            let degrees =
                if w[2].abs() > 0.9 { sweep(drag.last, p, center) * w[2].signum() } else { (dx * w[1] - dy * w[0]) / across * DEGREES_PER_POINT };
            multiply(m, turn(unit(axis), degrees))
        }
    }
}

/// The angles a cube value holds; a mixed one reads 0, as the cube is off while any is mixed.
fn degrees(value: &[Mixed<f64>; 3]) -> [f64; 3] {
    value.each_ref().map(|m| if let Mixed::Same(v) = m { *v } else { 0.0 })
}

/// The cube's value for `angles`.
fn same(angles: [f64; 3]) -> [Mixed<f64>; 3] {
    angles.map(Mixed::Same)
}

/// The cube turned by the pointer at `p`, as the value the gesture proposes.
fn dragged(drag: &mut CubeDrag, p: Pos2, center: Pos2, shift: bool, near: [f64; 3]) -> [Mixed<f64>; 3] {
    drag.matrix = drag_turn(drag, p, center, shift);
    drag.last = p;
    same(angles_of(drag.matrix, near))
}

/// The track cube: a cube drawn at the current X, Y and Z rotation (the object's front blue, its top and bottom light gray,
/// its sides mid gray and its back dark gray) inside a ring. The value is the three angles; it is off while any is mixed
/// (the 3D and Materials panel's selected effects differ).
///
/// - Pointer: dragging a face turns the object freely; with Shift the turn keeps to the screen's vertical axis (a drag
///   across) or its horizontal axis (a drag up or down), whichever the drag follows first. Dragging an edge turns it about
///   the object axis the edge runs along (the edge takes its axis's color while hovered or dragged), and dragging in the
///   ring turns it in the plane of the screen. A click that does not drag changes nothing. Escape puts the angles back.
/// - Keyboard: one Tab stop; the arrows turn the cube about the screen's axes and Page Up and Page Down in the screen
///   plane, 1° a press (15° with Shift), every press of a pass in turn.
/// - AccessKit: a `Slider` named by the label with the three angles as its value text.
#[derive(craft::ui::contract::Component, craft::ui::contract::Props)]
#[component(content_painter)]
pub(crate) struct TrackCube<'a> {
    #[common]
    common: Common,
    angles: Bind<'a, [Mixed<f64>; 3]>,
    /// The grip the pointer is over or holds.
    active: Option<Grip>,
}

impl<'a> TrackCube<'a> {
    /// A cube named `label` showing `angles`.
    pub(crate) fn new(label: impl Into<craft::ui::runtime::env::Label>, angles: Bind<'a, [Mixed<f64>; 3]>) -> Self {
        TrackCube { common: Common::new(), angles, active: None }.label(label)
    }
}

impl Component for TrackCube<'_> {
    const DISABLED: DisabledLook = DisabledLook::Faded;
    const SINGLE_VALUE: bool = true;
    const SHOWS_LABEL: bool = false;
    const ROLE: Role = Role::GROUP;
    const KEYS: KeyUse = KeyUse::of(Input::Pointer).with_keep(KeepKeys::BlockArrows).with_activate(ActivateKeys::Neither).with_moves();
    const OVERLAYS: bool = false;
    type Style = RowStyle;
    type Output = EditResponse<[Mixed<f64>; 3]>;

    fn style(cx: &StyleCx<'_>, _: TrackCubeStyleKey) -> RowStyle {
        RowStyle::of(cx)
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::splat(CUBE_BOX)
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        cx.part(CUBE, rect)
            .sense(Sense::DRAG | Sense::CLICK)
            .tooltip(tl!("Drag a face, an edge or the ring to rotate. Shift turns about one screen axis."))
            .focusable();
    }

    fn bind(&mut self) -> Option<BindMut<'_, [Mixed<f64>; 3]>> {
        Some(self.angles.reborrow())
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let now = cx.value().map(degrees).unwrap_or_default();
        let center = cx.rect.center();
        let mut held = *cx.press_slot::<CubeDrag>();
        let result = cx.part(CUBE);
        let (press, hover) = (result.press, result.hover_pos.filter(|_| result.hovered));
        match press.phase {
            PressPhase::Down { start, .. } if start.button == PointerButton::Primary => {
                cx.focus_part(CUBE);
                let matrix = rotation(now);
                held = Some(CubeDrag { grip: grip_at(matrix, center, start.origin), matrix, origin: start.origin, last: start.origin, lock: None });
            }
            PressPhase::DragStart { pos, .. } | PressPhase::Drag { pos, .. } => {
                if let Some(drag) = held.as_mut() {
                    if matches!(press.phase, PressPhase::DragStart { .. }) {
                        cx.begin();
                    }
                    cx.propose(dragged(drag, pos, center, press.mods.shift, now));
                }
            }
            PressPhase::Release { pos, .. } => {
                // A click that never dragged ran no gesture and changes nothing.
                if let Some(drag) = held.as_mut()
                    && cx.gesture().is_some()
                {
                    cx.propose(dragged(drag, pos, center, press.mods.shift, now));
                    cx.commit();
                }
                held = None;
            }
            PressPhase::Cancel { .. } => held = None,
            _ => {}
        }
        *cx.press_slot::<CubeDrag>() = held;
        self.active = held.map(|drag| drag.grip).or_else(|| hover.map(|p| grip_at(rotation(now), center, p)));
        match (held, self.active) {
            (Some(drag), _) => {
                let icon = if matches!(drag.grip, Grip::Edge(_)) { CursorIcon::ResizeHorizontal } else { CursorIcon::Grabbing };
                cx.cursor(icon, CursorPriority::Drag);
            }
            (None, Some(grip)) if hover.is_some() => {
                let icon = if matches!(grip, Grip::Edge(_)) { CursorIcon::ResizeHorizontal } else { CursorIcon::Grab };
                cx.cursor(icon, CursorPriority::Part);
            }
            _ => {}
        }
        if cx.gesture().is_none() && cx.active_part() == Some(CUBE.into()) {
            let mut m = rotation(now);
            let mut turned = false;
            while let Some((key, mods)) = cx.intents().take_move() {
                use craft::behavior::NavMove;
                let (axis, sign) = match key {
                    NavMove::Forward => (1, 1.0),
                    NavMove::Back => (1, -1.0),
                    NavMove::Up => (0, 1.0),
                    NavMove::Down => (0, -1.0),
                    NavMove::PageDown => (2, 1.0),
                    NavMove::PageUp => (2, -1.0),
                    _ => continue,
                };
                if mods.lead || mods.fine {
                    continue;
                }
                m = multiply(turn(unit(axis), sign * if mods.extend { 15.0 } else { 1.0 }), m);
                turned = true;
            }
            let next = angles_of(m, now);
            if turned && next.iter().zip(now).any(|(a, b)| (a - b).abs() > 1e-9) {
                cx.set(same(next));
            }
        }
    }

    fn access(&self, access: &mut Access, cx: &Cx<'_, Self>) {
        let [x, y, z] = cx.value().map(degrees).unwrap_or_default();
        access.node(CUBE, Role::SLIDER).name = Name::Text(tl!("Track cube").to_owned());
        access.node(CUBE, Role::SLIDER).value(format!("X {x:.2}°, Y {y:.2}°, Z {z:.2}°"));
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let m = rotation(cx.value().map(degrees).unwrap_or_default());
        let content = ui.content_painter(cx);
        paint_cube(&content.painter, cx.style, cx.rect, m, self.active, cx.is_focused());
    }

    fn finish(self, cx: Cx<'_, Self>) -> EditResponse<[Mixed<f64>; 3]> {
        cx.edit_response()
    }
}

/// The track cube's row on the Rotation page of the 3D (Classic) effects. It reads and writes `rotationX`, `rotationY` and
/// `rotationZ` (a missing one reads the effect's default) and only writes an angle that changes; the dialog's preview applies
/// them and OK records them as one undo step. Its control's events go to the rows, so a drag previews and Escape restores.
pub(crate) fn row() -> Param {
    Param::custom("__trackCube", tl!("Track cube"), |ui, fields, cx| {
        let defaults = vectorcraft_effects::default_params(fields.text("__effect").unwrap_or_default()).unwrap_or(json!({}));
        let now = ROTATION_KEYS.map(|key| {
            if fields.is_mixed(key) {
                Mixed::Mixed
            } else {
                Mixed::Same(fields.number(key).unwrap_or_else(|| defaults.get(key).and_then(serde_json::Value::as_f64).unwrap_or(0.0)))
            }
        });
        let mixed = now.iter().any(|m| matches!(m, Mixed::Mixed));
        let out = TrackCube::new(tl!("Track cube"), Bind::given(now)).enabled(!mixed).show(ui);
        cx.gesture(&out.events);
        let was = degrees(&now);
        // What each event leaves in the cube, a cancel's restored start included.
        for next in out.events.iter().filter_map(|event| event.landed()) {
            for ((key, angle), old) in ROTATION_KEYS.into_iter().zip(degrees(next)).zip(was) {
                if (angle - old).abs() > 1e-9 {
                    fields.set(key, angle);
                }
            }
        }
        out.response
    })
}

/// Draws the track cube at rotation `m` in `rect`, with the grip under the pointer or in hand lit.
fn paint_cube(painter: &egui::Painter, s: &RowStyle, rect: Rect, m: M3, active: Option<Grip>, focused: bool) {
    let center = rect.center();
    let colors = [s.axis_x.egui(), s.axis_y.egui(), s.axis_z.egui()];
    painter.rect_filled(rect, 2.0, s.well.egui());
    let ring = if active == Some(Grip::Ring) { 0.6 } else { 0.3 };
    painter.circle_stroke(center, (RING_IN + RING_OUT) / 2.0, Stroke::new(RING_OUT - RING_IN, colors[2].gamma_multiply(ring)));
    // Front blue; top and bottom light, sides mid and back dark gray. Only faces turned to the
    // viewer are drawn, and on a cube those never overlap.
    for (axis, side) in (0..3).flat_map(|axis| [(axis, true), (axis, false)]) {
        if !face_visible(m, axis, side) {
            continue;
        }
        // PLAN: the gray steps are literals; the data tokens of area plan §2 w3 were not visible to this writer.
        let fill = match (axis, side) {
            (2, true) => colors[2],
            (2, false) => Color32::from_gray(90),
            (1, _) => Color32::from_gray(205),
            _ => Color32::from_gray(155),
        };
        let points: Vec<Pos2> = face_corners(axis, side).iter().map(|&i| corner(m, center, i)).collect();
        painter.add(egui::Shape::convex_polygon(points, fill, Stroke::NONE));
    }
    for (i, j, axis) in edges() {
        if !edge_visible(m, i, axis) {
            continue;
        }
        let lit = active == Some(Grip::Edge(axis));
        let stroke = if lit { Stroke::new(2.5, colors[axis]) } else { Stroke::new(1.0, s.muted.egui()) };
        painter.line_segment([corner(m, center, i), corner(m, center, j)], stroke);
    }
    let border = if focused { s.accent } else { s.edge }.egui();
    painter.rect_stroke(rect, 2.0, Stroke::new(1.0, border), egui::StrokeKind::Inside);
}

#[cfg(test)]
#[path = "tests_track_cube.rs"]
mod tests;
