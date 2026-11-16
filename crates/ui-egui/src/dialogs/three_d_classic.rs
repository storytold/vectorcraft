//! The own controls of Extrude & Bevel (Classic), Revolve (Classic) and Rotate (Classic) in the
//! compact 3D dialog: the extrude and bevel options, or the revolve options, on the Object tab
//! (Rotate (Classic) has none), and the surface with its lighting sphere, lights, highlight, blend
//! steps, shading color and Preserve Spot Colors on the Lighting tab. Position (presets, rotation, perspective) is the Rotation tab every 3D
//! effect shares; the classic effects add the track cube to it ([`track_cube`]).

use egui::{Color32, Id, Pos2, Rect, Sense, Stroke, Vec2, vec2};
use serde_json::{Value, json};
use vectorcraft_doc::Unit;
use vectorcraft_effects::{CLASSIC_EXTRUDE, CLASSIC_REVOLVE, CLASSIC_ROTATE};

use super::revolve::{c3, choice, heading, label, light_list, light_record, number, reset_dot, selected_light, store_selected, toggle};
use crate::state::Dialog;
use crate::theme::Tokens;

/// Depth and cap, then the bevel: its shape, its height and whether it is added around the shape
/// or carved into it.
pub(super) fn object_controls(ui: &mut egui::Ui, d: &mut Dialog, unit: Unit) {
    heading(ui, tl!("Object"));
    number(ui, d, "depth", tl!("Depth"), (0.0, 2000.0), "", 1, Some(unit));
    choice(ui, d, "cap", c3("Cap"), &[(json!(true), c3("Solid")), (json!(false), c3("Hollow"))]);
    ui.add_space(10.0);
    heading(ui, c3("Bevel"));
    choice(ui, d, "bevel", tl!("Bevel Shape"), &options("bevel"));
    ui.add_enabled_ui(d.str("bevel") != "none", |ui| {
        number(ui, d, "bevelHeight", tl!("Height"), (1.0, 100.0), "", 1, Some(unit));
        toggle(ui, d, "bevelInside", tl!("Bevel Inside"), false);
    });
}

/// Revolve (Classic)'s angle, cap and offset, and the edge of the profile's bounds the axis is at.
pub(super) fn revolve_controls(ui: &mut egui::Ui, d: &mut Dialog, unit: Unit) {
    heading(ui, tl!("Object"));
    number(ui, d, "angle", tl!("Angle"), (0.0, 360.0), "°", 1, None);
    choice(ui, d, "cap", c3("Cap"), &[(json!(true), c3("Solid")), (json!(false), c3("Hollow"))]);
    number(ui, d, "offset", tl!("Offset"), (0.0, 1000.0), "", 1, Some(unit));
    let sides: Vec<(Value, &str)> = super::effect::own_choices(CLASSIC_REVOLVE, "edge", false)
        .unwrap_or_default()
        .into_iter()
        .map(|(label, value)| (json!(value), label))
        .collect();
    choice(ui, d, "edge", tl!("Axis side"), &sides);
}

/// The surface, then for Diffuse and Plastic the lighting sphere, the lights, Ambient Light, the
/// highlight (Plastic only, and not for Rotate (Classic), which has no Plastic surface), Blend Steps,
/// Shading Color and Preserve Spot Colors (unavailable while Shading Color is Custom); Draw Hidden
/// Faces last, for Extrude & Bevel (Classic) (Revolve (Classic) draws every face, and Rotate
/// (Classic)'s plane is seen from either side). The sphere shows in the dialog as well as in the panel.
pub(super) fn surface_controls(ui: &mut egui::Ui, d: &mut Dialog) {
    let id = d.str("__effect");
    let effect = if id == CLASSIC_ROTATE { CLASSIC_ROTATE } else { CLASSIC_EXTRUDE };
    choice(ui, d, "surface", tl!("Surface"), &options_of(effect, "surface"));
    let surface = d.str("surface");
    ui.add_enabled_ui(surface == "diffuse" || surface == "plastic", |ui| {
        super::revolve_lights::show(ui, d);
        light_list(ui, d);
        let mut selected = selected_light(d);
        let before = light_record(&selected);
        number(ui, &mut selected, "lightIntensity", tl!("Light Intensity"), (0.0, 100.0), "%", 1, None);
        number(ui, &mut selected, "lightRotation", tl!("Rotation"), (-180.0, 180.0), "°", 1, None);
        number(ui, &mut selected, "lightHeight", tl!("Height"), (0.0, 90.0), "°", 1, None);
        toggle(ui, &mut selected, "lightBehind", tl!("Light behind the object"), false);
        if light_record(&selected) != before {
            store_selected(d, &selected);
        }
        number(ui, d, "ambient", tl!("Ambient Light"), (0.0, 100.0), "%", 1, None);
        if id != CLASSIC_ROTATE {
            ui.add_enabled_ui(surface == "plastic", |ui| {
                number(ui, d, "highlightIntensity", tl!("Highlight Intensity"), (0.0, 100.0), "%", 1, None);
                number(ui, d, "highlightSize", tl!("Highlight Size"), (0.0, 100.0), "%", 1, None);
            });
        }
        number(ui, d, "blendSteps", tl!("Blend Steps"), (1.0, 256.0), "", 0, None);
        choice(ui, d, "shadingColor", tl!("Shading Color"), &options_of(effect, "shadingColor"));
        let custom = d.str("shadingColor") == "custom";
        if custom {
            shading_custom(ui, d);
        }
        ui.add_enabled_ui(!custom, |ui| toggle(ui, d, "preserveSpotColors", tl!("Preserve Spot Colors"), false));
    });
    if id == CLASSIC_EXTRUDE {
        toggle(ui, d, "drawHiddenFaces", tl!("Draw Hidden Faces"), false);
    }
}

/// The values of Extrude & Bevel (Classic)'s `key` as the dropdowns list them, (value, label).
fn options(key: &str) -> Vec<(Value, &'static str)> {
    options_of(CLASSIC_EXTRUDE, key)
}

/// The values of classic 3D effect `effect`'s `key` as the dropdowns list them, (value, label).
fn options_of(effect: &str, key: &str) -> Vec<(Value, &'static str)> {
    super::effect::own_choices(effect, key, false).unwrap_or_default().into_iter().map(|(label, value)| (json!(value), label)).collect()
}

/// The custom shading color: a color button, its hex value and its reset dot.
fn shading_custom(ui: &mut egui::Ui, d: &mut Dialog) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        label(ui, tl!("Color"));
        let color = d
            .fields
            .get("shadingCustom")
            .and_then(Value::as_str)
            .filter(|s| s.len() == 7 && s.starts_with('#'))
            .and_then(vectorcraft_color::Color::from_hex)
            .unwrap_or(vectorcraft_color::Color::BLACK);
        let mut rgb = color.to_rgb_uncalibrated().map(|c| (c * 255.0).round() as u8);
        if ui.color_edit_button_srgb(&mut rgb).on_hover_text(tl!("Shading Color")).changed() {
            d.fields.insert("shadingCustom".into(), json!(format!("#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2])));
        }
        ui.label(format!("#{:02X}{:02X}{:02X}", rgb[0], rgb[1], rgb[2]));
        ui.add_space((ui.available_width() - 24.0).max(0.0));
        if reset_dot(ui, "shadingCustom", rgb != [0; 3], "#000000") {
            d.fields.insert("shadingCustom".into(), json!("#000000"));
        }
    });
}

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
#[derive(Clone, Copy, Debug, PartialEq)]
enum Grip {
    /// A face: a free turn, or with Shift about the screen's horizontal or vertical axis.
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
#[derive(Clone)]
struct CubeDrag {
    grip: Grip,
    /// The rotation so far, kept unrounded so a long drag does not drift.
    matrix: M3,
    origin: Pos2,
    last: Pos2,
    /// With Shift, the screen axis the drag keeps to (0 X, 1 Y), chosen once it has moved 3 points.
    lock: Option<usize>,
    /// The settings before the gesture, which Escape puts back.
    fields: serde_json::Map<String, Value>,
    /// The last pass that drew the cube during the gesture.
    pass: u64,
    /// The cube the gesture started on. The 3D and Materials panel and a 3D dialog can each show
    /// a cube at the same time, and only this one follows the gesture.
    owner: Id,
}

impl CubeDrag {
    /// Was the cube drawn in this pass or the one before? When it was not (its dialog closed, or
    /// the Rotation tab or the panel was left during the drag), the gesture is over.
    fn live(&self, pass: u64) -> bool {
        self.pass.saturating_add(1) >= pass
    }
}

/// The track cube of the dialog or panel of kind `kind`. The 3D and Materials panel and a 3D
/// dialog can show a cube at the same time, each with its own keyboard focus.
pub(super) fn cube_id(kind: &str) -> Id {
    Id::unique(("three-d-track-cube", kind))
}

fn cube_drag_id() -> Id {
    Id::unique("three-d-track-cube-drag")
}

/// Is a track cube gesture in progress?
pub(super) fn cube_dragging(ctx: &egui::Context) -> bool {
    let pass = ctx.cumulative_pass_nr();
    ctx.data(|m| m.get_temp::<CubeDrag>(cube_drag_id())).is_some_and(|g| g.live(pass))
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

/// The track cube on the Rotation tab of Extrude & Bevel (Classic), Revolve (Classic) and Rotate
/// (Classic): a cube drawn at the current X, Y and Z rotation (the object's front blue, its top and
/// bottom light gray, its sides mid gray and its back dark gray) inside a ring.
///
/// Dragging a face turns the object freely; with Shift the turn keeps to the screen's vertical axis
/// (a drag across) or its horizontal axis (a drag up or down), whichever the drag follows first.
/// Dragging an edge turns it about the object axis the edge runs along (X red, Y green, Z blue; the
/// edge takes its axis's color while hovered or dragged), and dragging in the ring turns it in the
/// plane of the screen. Escape during a drag puts the angles back. With the keyboard focus, the
/// arrow keys turn it about the screen's axes and Page Up and Page Down in the screen plane, 1° a
/// press (15° with Shift). The angles go to `rotationX`, `rotationY` and `rotationZ` only when they
/// change, and the dialog's preview applies them; OK records them as one undo step. The cube is off
/// while a rotation setting is mixed (the 3D and Materials panel's selected effects differ).
pub(super) fn track_cube(ui: &mut egui::Ui, d: &mut Dialog) {
    let defaults = vectorcraft_effects::default_params(&d.str("__effect")).unwrap_or(json!({}));
    let mixed = ROTATION_KEYS.iter().any(|k| d.fields.get(*k).is_some_and(Value::is_null));
    let now = ROTATION_KEYS.map(|k| d.f64(k, defaults.get(k).and_then(Value::as_f64).unwrap_or(0.0)));
    let mut next = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let name = tl!("Track cube");
        label(ui, name);
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(CUBE_BOX), Sense::hover());
        let enabled = ui.is_enabled() && !mixed;
        let id = cube_id(&d.kind);
        let resp = ui.interact(rect, id, if enabled { Sense::click_and_drag() } else { Sense::hover() });
        let center = rect.center();
        let value = format!("X {:.2}°, Y {:.2}°, Z {:.2}°", now[0], now[1], now[2]);
        resp.widget_info(|| egui::WidgetInfo {
            current_text_value: Some(value.clone()),
            ..egui::WidgetInfo::labeled(egui::Role::Slider, enabled, name)
        });
        if enabled
            && resp.drag_started()
            && let Some(origin) = ui.input(|i| i.pointer.press_origin())
        {
            // A value being typed ends first; the cube then takes the keyboard focus below.
            ui.ctx().memory_mut(|m| m.stop_text_input());
            let matrix = rotation(now);
            let pass = ui.ctx().cumulative_pass_nr();
            let drag = CubeDrag {
                grip: grip_at(matrix, center, origin),
                matrix,
                origin,
                last: origin,
                lock: None,
                fields: d.fields.clone(),
                pass,
                owner: id,
            };
            ui.data_mut(|m| m.insert_temp(cube_drag_id(), drag));
        }
        if enabled && (resp.drag_started() || resp.clicked()) {
            resp.request_focus();
        }
        let pass = ui.ctx().cumulative_pass_nr();
        // A gesture on the other cube (the panel's or the dialog's) is left to that cube.
        let mut drag = ui.data(|m| m.get_temp::<CubeDrag>(cube_drag_id())).filter(|g| g.owner == id);
        if drag.as_ref().is_some_and(|g| !g.live(pass)) {
            ui.data_mut(|m| m.remove::<CubeDrag>(cube_drag_id()));
            drag = None;
        }
        if let Some(current) = &mut drag {
            current.pass = pass;
            if ui.input_mut(|i| i.consume_key(i.modifiers, egui::Key::Escape)) {
                d.fields = current.fields.clone();
                d.fields.remove(super::form::PREVIEWED);
                ui.data_mut(|m| m.remove::<CubeDrag>(cube_drag_id()));
                drag = None;
            } else {
                if let Some(p) = resp.interact_pointer_pos() {
                    let shift = ui.input(|i| i.modifiers.shift);
                    current.matrix = drag_turn(current, p, center, shift);
                    current.last = p;
                    next = Some(angles_of(current.matrix, now));
                }
                ui.data_mut(|m| m.insert_temp(cube_drag_id(), current.clone()));
            }
        }
        // Only this cube's gesture ends here; the other cube may still have the release to handle.
        if drag.is_some() && (resp.drag_stopped() || !ui.input(|i| i.pointer.primary_down())) {
            ui.data_mut(|m| m.remove::<CubeDrag>(cube_drag_id()));
        }
        if enabled && resp.has_focus() {
            use egui::{Key, Modifiers};
            ui.memory_mut(|m| {
                m.set_focus_lock_filter(id, egui::EventFilter { horizontal_arrows: true, vertical_arrows: true, ..Default::default() })
            });
            let mut m = rotation(now);
            let mut turned = false;
            for (key, axis, sign) in [
                (Key::ArrowRight, 1, 1.0),
                (Key::ArrowLeft, 1, -1.0),
                (Key::ArrowUp, 0, 1.0),
                (Key::ArrowDown, 0, -1.0),
                (Key::PageDown, 2, 1.0),
                (Key::PageUp, 2, -1.0),
            ] {
                // Shift first: a key without modifiers also matches the key with Shift.
                for (modifiers, step) in [(Modifiers::SHIFT, 15.0), (Modifiers::NONE, 1.0)] {
                    while ui.input_mut(|i| i.consume_key(modifiers, key)) {
                        m = multiply(turn(unit(axis), sign * step), m);
                        turned = true;
                    }
                }
            }
            if turned {
                next = Some(angles_of(m, now));
            }
        }
        let shown = next.unwrap_or(now);
        let hovered = drag.as_ref().map(|g| g.grip).or_else(|| resp.hover_pos().filter(|_| enabled).map(|p| grip_at(rotation(shown), center, p)));
        if enabled && (resp.hovered() || drag.is_some()) {
            ui.ctx().set_cursor_icon(match (hovered, drag.is_some()) {
                (Some(Grip::Edge(_)), _) => egui::CursorIcon::ResizeHorizontal,
                (_, true) => egui::CursorIcon::Grabbing,
                _ => egui::CursorIcon::Grab,
            });
        }
        paint_cube(ui, rect, rotation(shown), hovered, enabled, resp.has_focus());
        resp.on_hover_text(tl!("Drag a face, an edge or the ring to rotate. Shift turns about one screen axis."));
    });
    if let Some(angles) = next
        && angles.iter().zip(now).any(|(a, b)| (a - b).abs() > 1e-9)
    {
        for (key, angle) in ROTATION_KEYS.into_iter().zip(angles) {
            d.fields.insert(key.into(), json!(angle));
        }
    }
}

/// Draws the track cube at rotation `m` in `rect`, with the grip under the pointer or in hand lit.
fn paint_cube(ui: &egui::Ui, rect: Rect, m: M3, active: Option<Grip>, enabled: bool, focused: bool) {
    let t = Tokens::get(ui.ctx());
    let fade = |c: Color32| if enabled { c } else { c.gamma_multiply(0.4) };
    let painter = ui.painter_at(rect);
    let center = rect.center();
    painter.rect_filled(rect, 2.0, t.input);
    let ring = if active == Some(Grip::Ring) { 0.6 } else { 0.3 };
    painter.circle_stroke(center, (RING_IN + RING_OUT) / 2.0, Stroke::new(RING_OUT - RING_IN, fade(t.axis_z.gamma_multiply(ring))));
    // Front blue; top and bottom light, sides mid and back dark gray. Only faces turned to the
    // viewer are drawn, and on a cube those never overlap.
    for (axis, side) in (0..3).flat_map(|axis| [(axis, true), (axis, false)]) {
        if !face_visible(m, axis, side) {
            continue;
        }
        let fill = match (axis, side) {
            (2, true) => t.axis_z,
            (2, false) => Color32::from_gray(90),
            (1, _) => Color32::from_gray(205),
            _ => Color32::from_gray(155),
        };
        let points: Vec<Pos2> = face_corners(axis, side).iter().map(|&i| corner(m, center, i)).collect();
        painter.add(egui::Shape::convex_polygon(points, fade(fill), Stroke::NONE));
    }
    let colors = [t.axis_x, t.axis_y, t.axis_z];
    for (i, j, axis) in edges() {
        if !edge_visible(m, i, axis) {
            continue;
        }
        let lit = active == Some(Grip::Edge(axis));
        let stroke = if lit { Stroke::new(2.5, fade(colors[axis])) } else { Stroke::new(1.0, fade(t.text_dim)) };
        painter.line_segment([corner(m, center, i), corner(m, center, j)], stroke);
    }
    let border = if focused { t.accent } else { t.input_border };
    painter.rect_stroke(rect, 2.0, Stroke::new(1.0, border), egui::StrokeKind::Inside);
}

#[cfg(test)]
#[path = "tests_track_cube.rs"]
mod tests;
