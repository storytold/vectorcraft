//! The track cube's rotations, grips and gestures.
use super::*;
use egui::pos2;
use vectorcraft_effects::ROTATION_PRESETS;

const SAMPLES: [[f64; 3]; 6] =
    [[0.0, 0.0, 0.0], [-21.88, -23.4, 9.06], [27.0, -39.0, 53.0], [170.0, 10.0, -95.5], [-120.0, 75.0, 33.0], [45.0, -35.26, 30.0]];

fn close(a: [f64; 3], b: [f64; 3]) -> bool {
    a.iter().zip(b).all(|(a, b)| tidy(a - b).abs() < 0.011)
}

fn same_rotation(a: M3, b: M3) -> bool {
    a.iter().flatten().zip(b.iter().flatten()).all(|(a, b)| (a - b).abs() < 1e-3)
}

/// The cube turns as the 3D effects turn the art: each object axis lands where the effects' own
/// view rotation puts it.
#[test]
fn the_cube_rotates_as_the_effects_do() {
    for angles in SAMPLES {
        let m = rotation(angles);
        let view = vectorcraft_three_d::Revolve { rotation_x: angles[0], rotation_y: angles[1], rotation_z: angles[2], ..Default::default() };
        for (axis, p) in view.view_axes().into_iter().enumerate() {
            assert!((m[0][axis] - p.x).abs() < 1e-9 && (m[1][axis] - p.y).abs() < 1e-9, "{angles:?} axis {axis}");
        }
    }
}

/// Reading the angles back gives the same angles, presets included, Back's 180° and the gimbal
/// positions Left and Right (Y at ±90°) among them.
#[test]
fn angles_read_back_as_they_were_set() {
    for angles in SAMPLES.into_iter().chain(ROTATION_PRESETS.iter().map(|(_, a)| *a)) {
        let back = angles_of(rotation(angles), angles);
        assert!(close(back, angles), "{angles:?} read back as {back:?}");
        assert!(same_rotation(rotation(back), rotation(angles)));
    }
    assert_eq!(angles_of(rotation([0.0, 180.0, 0.0]), [0.0, 180.0, 0.0]), [0.0, 180.0, 0.0], "Back stays Back, not -180°");
}

#[test]
fn angles_stay_in_the_fields_range_to_a_hundredth() {
    assert_eq!(tidy(180.0), 180.0);
    assert_eq!(tidy(-180.0), 180.0);
    assert_eq!(tidy(190.004), -170.0);
    assert_eq!(tidy(-0.001), 0.0);
    assert!(tidy(-0.001).is_sign_positive());
}

fn drag(grip: Grip, angles: [f64; 3], from: Pos2, to: Pos2, shift: bool) -> [f64; 3] {
    let mut g =
        CubeDrag { grip, matrix: rotation(angles), origin: from, last: from, lock: None, fields: Default::default(), pass: 0, owner: Id::NULL };
    // In five steps, as a pointer moves.
    let mut near = angles;
    for k in 1..=5 {
        let p = from + (to - from) * (k as f32 / 5.0);
        g.matrix = drag_turn(&mut g, p, pos2(0.0, 0.0), shift);
        g.last = p;
        near = angles_of(g.matrix, near);
    }
    near
}

/// The ring turns in the screen plane, which the effects apply last: only Z changes, by the angle
/// swept, clockwise on the screen.
#[test]
fn the_ring_turns_z_only() {
    for angles in SAMPLES {
        let after = drag(Grip::Ring, angles, pos2(0.0, -53.0), pos2(53.0, 0.0), false);
        assert!(close(after, [angles[0], angles[1], angles[2] + 90.0]), "{angles:?} -> {after:?}");
    }
}

/// An edge turns about its own object axis. The effects turn X first, so an X edge changes X alone.
#[test]
fn an_x_edge_turns_x_only_and_other_edges_keep_their_axis() {
    let start = [-21.88, -23.4, 9.06];
    let after = drag(Grip::Edge(0), start, pos2(0.0, 0.0), pos2(0.0, -40.0), false);
    assert!((after[0] - start[0]).abs() > 5.0, "X turned: {after:?}");
    assert!((after[1] - start[1]).abs() < 0.02 && (after[2] - start[2]).abs() < 0.02, "Y and Z kept: {after:?}");
    for axis in 0..3 {
        let after = drag(Grip::Edge(axis), start, pos2(0.0, 0.0), pos2(30.0, -30.0), false);
        let (m0, m1) = (rotation(start), rotation(after));
        let column = |m: M3| [m[0][axis], m[1][axis], m[2][axis]];
        let (a, b) = (column(m0), column(m1));
        assert!(a.iter().zip(b).all(|(a, b)| (a - b).abs() < 1e-3), "axis {axis} stays put: {a:?} {b:?}");
        assert!(!same_rotation(m0, m1), "axis {axis}: the cube turned");
    }
}

/// A face drag across the screen turns about the screen's vertical axis, the front toward the
/// drag; Shift keeps a slanted drag to that axis.
#[test]
fn a_face_drag_turns_the_front_toward_the_pointer_and_shift_keeps_one_axis() {
    let front = [0.0, 0.0, 0.0];
    let right = drag(Grip::Face, front, pos2(0.0, 0.0), pos2(40.0, 0.0), false);
    assert!(close(right, [0.0, 20.0, 0.0]), "{right:?}");
    // The front (+Z) turns toward +X.
    assert!(rotation(right)[0][2] > 0.3);
    let down = drag(Grip::Face, front, pos2(0.0, 0.0), pos2(0.0, 40.0), false);
    assert!(close(down, [-20.0, 0.0, 0.0]), "{down:?}");
    assert!(rotation(down)[1][2] > 0.3, "the front turns down");
    let slanted = drag(Grip::Face, front, pos2(0.0, 0.0), pos2(40.0, 12.0), true);
    assert!(close(slanted, [0.0, 20.0, 0.0]), "Shift across: {slanted:?}");
    let steep = drag(Grip::Face, front, pos2(0.0, 0.0), pos2(10.0, -40.0), true);
    assert!(close(steep, [20.0, 0.0, 0.0]), "Shift up: {steep:?}");
    let free = drag(Grip::Face, front, pos2(0.0, 0.0), pos2(40.0, 12.0), false);
    assert!(free[0].abs() > 1.0, "without Shift a slanted drag also tilts: {free:?}");
}

/// A gesture ends when the cube stops being drawn during it (its dialog closed, or the Rotation tab
/// was left), so Escape is not held back for a cube that is no longer there.
#[test]
fn a_drag_ends_when_the_cube_is_no_longer_drawn() {
    let ctx = egui::Context::default();
    let run = |draw: &mut dyn FnMut(&mut egui::Ui)| ctx.run_ui(Default::default(), draw).textures_delta.clear();
    run(&mut |ui| {
        let pass = ui.ctx().cumulative_pass_nr();
        let drag = CubeDrag {
            grip: Grip::Face,
            matrix: rotation([0.0; 3]),
            origin: Pos2::ZERO,
            last: Pos2::ZERO,
            lock: None,
            fields: Default::default(),
            pass,
            owner: Id::NULL,
        };
        ui.data_mut(|m| m.insert_temp(cube_drag_id(), drag));
        assert!(cube_dragging(ui.ctx()));
    });
    assert!(cube_dragging(&ctx), "the next pass still sees the gesture");
    for _ in 0..2 {
        run(&mut |_| {});
    }
    assert!(!cube_dragging(&ctx), "two passes without the cube end the gesture");
}

/// With the 3D and Materials panel and a 3D dialog both showing a cube, a gesture on one cube is
/// left alone by the other: Escape there does not put the first cube's settings into it.
#[test]
fn a_cube_leaves_a_gesture_on_the_other_cube_alone() {
    let ctx = egui::Context::default();
    let mut d = Dialog::new("effect", json!({ "rotationX": 10.0, "rotationY": 0.0, "rotationZ": 0.0 }));
    let escape = egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
    let input = egui::RawInput { events: vec![escape], ..Default::default() };
    let mut out = ctx.run_ui(input, |ui| {
        let pass = ui.ctx().cumulative_pass_nr();
        let other = CubeDrag {
            grip: Grip::Face,
            matrix: rotation([50.0, 0.0, 0.0]),
            origin: Pos2::ZERO,
            last: Pos2::ZERO,
            lock: None,
            fields: json!({ "rotationX": 50.0, "rotationY": 0.0, "rotationZ": 0.0 }).as_object().cloned().unwrap_or_default(),
            pass,
            owner: cube_id(crate::panels::three_d::ID),
        };
        assert_ne!(cube_id(crate::panels::three_d::ID), cube_id(&d.kind), "each cube has its own focus");
        ui.data_mut(|m| m.insert_temp(cube_drag_id(), other));
        track_cube(ui, &mut d);
    });
    out.textures_delta.clear();
    assert_eq!(d.f64("rotationX", 0.0), 10.0, "the dialog keeps its own rotation");
    // The button is up in that pass, and the other cube, drawn later, still has the release to handle.
    assert!(ctx.data(|m| m.get_temp::<CubeDrag>(cube_drag_id())).is_some(), "the other cube's gesture is still there");
}

/// The centre is on the front face, the ring's band on the ring, and an edge's middle on that edge.
#[test]
fn grips_follow_what_is_under_the_pointer() {
    let m = rotation([-21.88, -23.4, 9.06]);
    let center = pos2(100.0, 100.0);
    assert_eq!(grip_at(m, center, center), Grip::Face);
    assert_eq!(grip_at(m, center, center + vec2(0.0, -(RING_IN + RING_OUT) / 2.0)), Grip::Ring);
    for (i, j, axis) in edges().filter(|&(i, _, axis)| edge_visible(m, i, axis)) {
        let middle = corner(m, center, i) + (corner(m, center, j) - corner(m, center, i)) * 0.5;
        assert_eq!(grip_at(m, center, middle), Grip::Edge(axis), "edge {i}-{j}");
    }
    // Facing front: four faces are edge-on and hidden, so only the front's edges show.
    let m = rotation([0.0, 0.0, 0.0]);
    assert_eq!(edges().filter(|&(i, _, axis)| edge_visible(m, i, axis)).count(), 4);
}

/// Map Art lists each document's own surfaces: after a switch to another document whose object has
/// the same id, effect settings and revision, the surfaces are those of the other document's object.
#[test]
fn map_art_surfaces_follow_the_active_document() {
    let mut app = crate::VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    for d in ["M50 50L150 50L150 150L50 150Z", "M50 50L80 50L80 200L50 200Z"] {
        app.run("file.new", json!({"width":300,"height":300})).unwrap();
        app.run("path.create", json!({"d": d})).unwrap();
    }
    app.run("effect.dialog", json!({"effect":CLASSIC_EXTRUDE,"item":null})).unwrap();
    let d = app.ui.dialog.take().unwrap();
    let ctx = egui::Context::default();
    let key = |app: &crate::VectorcraftApp| {
        let st = app.session.active().unwrap();
        (st.selection.objects.first().copied(), st.revision)
    };
    let second = surfaces(&app, &ctx, &d).unwrap();
    let at_second = key(&app);
    assert!(app.session.set_active(0));
    assert_eq!(key(&app), at_second, "both documents' objects have the same id and revision");
    let first = surfaces(&app, &ctx, &d).unwrap();
    let st = app.session.active().unwrap();
    let id = st.selection.objects[0];
    let own = vectorcraft_effects::map_surfaces_with(
        Some(&st.doc),
        st.doc.node(id).unwrap(),
        (None, None),
        CLASSIC_EXTRUDE,
        &super::super::form::params(&d),
    );
    assert_ne!(first, second, "the two objects' solids differ in size");
    assert_eq!(Some(first.as_ref()), own.as_ref(), "the first document's surfaces");
}
