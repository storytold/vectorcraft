use super::*;
use serde_json::json;
use vectorcraft_doc::NodeKind;
use vectorcraft_doc::pattern::RepeatKind;
use vectorcraft_engine::Session;

// PLAN: these drive `Layout::of` and `advance` (the control's own steps) with the document points a press reports;
// a one-pass canvas harness for the real input path belongs to canvas-w32's fixture.

const ALL: Show = Show { nine: true, selecting: true, shaper: true, trace: true };

fn app() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.select_tool("selection");
    app
}

fn view(app: &VectorcraftApp) -> CanvasView {
    app.view().unwrap().canvas(app.ui_scaling(), Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0)))
}

fn layout(app: &mut VectorcraftApp, show: Show) -> Layout {
    let (view, info) = (view(app), app.view_info());
    app.session.with_tool_context(info, |cx| Layout::of(cx, &view, show))
}

fn widgets(l: &Layout, w: Widget) -> usize {
    l.items.iter().filter(|i| i.widget == w).count()
}

fn node(app: &VectorcraftApp, id: u64) -> vectorcraft_doc::Node {
    app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).cloned().unwrap()
}

/// Presses at screen point `from`, drags to `to` in two moves and releases.
fn drag_through(app: &mut VectorcraftApp, widget: Widget, from: Pos2, to: Pos2) -> Option<Held> {
    let v = view(app);
    let mut held = None;
    let mid = from + (to - from) * 0.5;
    for g in [Gesture::Press(v.to_doc(from)), Gesture::Move(v.to_doc(mid)), Gesture::Move(v.to_doc(to)), Gesture::Release(v.to_doc(to))] {
        advance(app, &mut held, widget, g, Mods::default());
    }
    held
}

/// A press on `widget` at screen point `at`, dragged to `q` and released at `r`, is dragged to `r` before it ends: the
/// release keeps what a drag to `r` shows.
fn check_release_away(app: &mut VectorcraftApp, widget: Widget, at: Pos2, q: Pos2, r: Pos2) {
    let v = view(app);
    let (at, q, r) = (v.to_doc(at), v.to_doc(q), v.to_doc(r));
    let pressed = |app: &mut VectorcraftApp| {
        let mut held = None;
        plan(app, &mut held, widget, Gesture::Press(at), Mods::default());
        assert!(!plan(app, &mut held, widget, Gesture::Move(q), Mods::default()).is_empty(), "the drag to {q:?} changes the widget");
        held
    };
    let mut held = pressed(app);
    let mut want = plan(app, &mut held, widget, Gesture::Move(r), Mods::default());
    assert!(!want.is_empty(), "the drag to {r:?} changes the widget");
    want.push(Action::Commit);
    let mut held = pressed(app);
    assert_eq!(plan(app, &mut held, widget, Gesture::Release(r), Mods::default()), want, "released at {r:?}");
}

/// A drag of `widget` from screen point `at` to `q` is cancelled by a switch to another tool: what it changed goes
/// back, and the release after it commits nothing.
fn check_tool_switch_cancels(app: &mut VectorcraftApp, widget: Widget, at: Pos2, q: Pos2) {
    let v = view(app);
    let before = app.session.active().unwrap().doc.clone();
    let mut held = None;
    advance(app, &mut held, widget, Gesture::Press(v.to_doc(at)), Mods::default());
    advance(app, &mut held, widget, Gesture::Move(v.to_doc(q)), Mods::default());
    assert_ne!(*app.session.active().unwrap().doc, *before, "the drag previews");
    app.select_tool("directSelection");
    let visible = shown(app);
    let l = layout(app, visible);
    end_hidden(app, &mut held, &l);
    assert!(held.is_none(), "a tool switch cancels the drag");
    assert_eq!(*app.session.active().unwrap().doc, *before, "its value goes back");
    assert!(plan(app, &mut held, widget, Gesture::Release(v.to_doc(q)), Mods::default()).is_empty(), "the release commits nothing");
}

fn mirror_app() -> (VectorcraftApp, u64) {
    let mut app = app();
    app.run("shape.rectangle", json!({"x": 100, "y": 100, "width": 60, "height": 80})).unwrap();
    let id = app.run("object.repeat.mirror", json!({})).unwrap()["id"].as_u64().unwrap();
    (app, id)
}

fn grid_of(app: &VectorcraftApp, id: u64) -> (u32, u32, f64) {
    let NodeKind::Repeat(spec) = &node(app, id).kind else { panic!("a repeat") };
    let RepeatKind::Grid { rows, cols, h_spacing, .. } = spec.kind else { panic!("a grid") };
    (rows, cols, h_spacing)
}

fn radial_of(app: &VectorcraftApp, id: u64) -> (u32, f64) {
    let NodeKind::Repeat(spec) = &node(app, id).kind else { panic!("a repeat") };
    let RepeatKind::Radial { instances, radius, .. } = spec.kind else { panic!("a radial") };
    (instances, radius)
}

#[test]
fn the_shaper_widget_points_to_the_other_mode() {
    let mut app = app();
    for x in [0, 50] {
        app.run("shape.rectangle", json!({"x": x, "y": 0, "width": 100, "height": 100})).unwrap();
    }
    app.run("shaper.scribble", json!({"points": [[25, 20], [75, 35], [30, 50], [125, 65]]})).unwrap();
    let mark = layout(&mut app, ALL).shaper.unwrap();
    assert!(!mark.construction);
    let v = view(&app);
    let mut drag = None;
    advance(&mut app, &mut drag, Widget::Shaper, Gesture::Click(v.to_doc(mark.at)), Mods::default());
    assert!(layout(&mut app, ALL).shaper.unwrap().construction, "a press switches to Construction mode");
    advance(&mut app, &mut drag, Widget::Shaper, Gesture::Click(v.to_doc(mark.at)), Mods::default());
    assert!(!layout(&mut app, ALL).shaper.unwrap().construction, "and back");
}

#[test]
fn mirror_axis_tracks_selection_options_and_view_without_editing_art() {
    let (mut app, _) = mirror_app();
    let revision = app.session.active().unwrap().revision;
    let l = layout(&mut app, ALL);
    let (ends, centre) = l.mirror.unwrap();
    assert!(ends.iter().all(|e| e.distance(centre) > 10.0));
    app.view_mut().unwrap().zoom *= 2.0;
    let zoomed = layout(&mut app, ALL).mirror.unwrap();
    assert_ne!(zoomed.1, centre, "the axis follows the view");
    assert!(layout(&mut app, Show { selecting: false, ..ALL }).mirror.is_none(), "not with Edges hidden");
    assert_eq!(app.session.active().unwrap().revision, revision, "showing the axis edits nothing");
    app.run("select.none", json!({})).unwrap();
    let revision = app.session.active().unwrap().revision;
    assert!(layout(&mut app, ALL).mirror.is_none());
    assert_eq!(app.session.active().unwrap().revision, revision, "hiding the axis edits nothing");
}

#[test]
fn mirror_axis_drag_uses_canvas_coordinates_in_a_zoomed_rotated_view() {
    let (mut app, id) = mirror_app();
    app.view_mut().unwrap().zoom = 1.5;
    app.view_mut().unwrap().rotation = 30.0;
    let before = node(&app, id);
    let l = layout(&mut app, ALL);
    let (_, centre) = l.mirror.unwrap();
    let to = centre + Vec2::new(30.0, 10.0);
    drag_through(&mut app, Widget::Mirror, centre, to);
    let v = view(&app);
    let moved = v.to_doc(to) - v.to_doc(centre);
    let st = app.session.active().unwrap();
    let axis = mirror::MirrorAxis::of(&st.doc, &st.selection, 1.0).unwrap();
    let NodeKind::Repeat(old) = &before.kind else { panic!() };
    let RepeatKind::Mirror { center: was, .. } = old.kind else { panic!() };
    assert!((axis.center - was - moved).hypot() < 1e-6, "{:?} vs {moved:?}", axis.center - was);
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(node(&app, id), before, "one undo step");
}

#[test]
fn trace_split_divider_drag_in_a_zoomed_rotated_view_is_one_undo_step() {
    let mut app = app();
    let data = vectorcraft_format::base64_encode(&crate::tests_sysclip::blue_png());
    app.run("file.place", json!({"name": "blue.png", "dataBase64": data, "link": false, "at": [200, 150]})).unwrap();
    app.run("imageTrace.make", json!({"preset": "Default"})).unwrap();
    app.run("imageTrace.setView", json!({"view": "split"})).unwrap();
    // Zoomed in on the 4 × 2 pixel image, where its bounding-box handles leave the divider room.
    let ends = {
        let st = app.session.active().unwrap();
        vectorcraft_tools::trace_split::TraceDivider::of(&st.doc, &st.selection, 1.0).unwrap().ends
    };
    let v = app.view_mut().unwrap();
    (v.zoom, v.rotation, v.center) = (30.0, 30.0, ends[0].midpoint(ends[1]));
    let l = layout(&mut app, ALL);
    let [a, b] = l.trace.unwrap();
    let (from, along) = (a + (b - a) * 0.25, b - a);
    let to = from + Vec2::new(-along.y, along.x).normalized() * 20.0;
    let before = app.session.active().unwrap().doc.clone();
    drag_through(&mut app, Widget::Trace, from, to);
    assert!(!std::sync::Arc::ptr_eq(&before, &app.session.active().unwrap().doc), "the split moved");
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(*app.session.active().unwrap().doc, *before, "one undo step puts it back");
}

#[test]
fn grid_repeat_handles_set_rows_and_spacing_in_one_undo_step_each() {
    let mut app = app();
    app.run("shape.rectangle", json!({"x": 40, "y": 40, "width": 40, "height": 40})).unwrap();
    let id = app.run("object.repeat.grid", json!({"hSpacing": 10, "vSpacing": 10})).unwrap()["id"].as_u64().unwrap();
    let (rows, ..) = grid_of(&app, id);
    let l = layout(&mut app, ALL);
    let rows_handle = l.grid[0].at;
    let step = 50.0 * view(&app).zoom as f32;
    drag_through(&mut app, Widget::Grid, rows_handle, rows_handle + Vec2::new(0.0, 2.0 * step));
    assert_eq!(grid_of(&app, id).0, rows + 2);
    let l = layout(&mut app, ALL);
    let slider = l.grid.iter().find(|g| g.bar.is_some()).unwrap().at;
    let spacing = grid_of(&app, id).2;
    drag_through(&mut app, Widget::Grid, slider, slider + Vec2::new(6.0, 0.0));
    assert!(grid_of(&app, id).2 > spacing);
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!((grid_of(&app, id).0, grid_of(&app, id).2), (rows + 2, spacing), "the spacing was its own step");
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(grid_of(&app, id).0, rows, "and the rows theirs");
}

#[test]
fn radial_repeat_handles_set_instances_and_radius_in_one_undo_step_each() {
    let mut app = app();
    app.run("shape.rectangle", json!({"x": 180, "y": 40, "width": 40, "height": 40})).unwrap();
    let id = app.run("object.repeat.radial", json!({"instances": 6, "radius": 60})).unwrap()["id"].as_u64().unwrap();
    let l = layout(&mut app, ALL);
    let instances = l.radial.as_ref().unwrap().handles[0].0;
    drag_through(&mut app, Widget::Radial, instances, instances + Vec2::new(0.0, 2.0 * repeat_radial::INSTANCE_STEP as f32));
    assert_eq!(radial_of(&app, id), (8, 60.0));
    let l = layout(&mut app, ALL);
    let (centre, _) = l.radial.as_ref().unwrap().circle.unwrap();
    let radius = l.radial.as_ref().unwrap().handles[1].0;
    let out = (radius - centre).normalized();
    let outward = 20.0 * view(&app).zoom as f32;
    drag_through(&mut app, Widget::Radial, radius, radius + out * outward);
    assert_eq!(radial_of(&app, id), (8, 80.0));
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(radial_of(&app, id), (8, 60.0));
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(radial_of(&app, id), (6, 60.0));
}

#[test]
fn radial_repeat_splitter_hides_instances_in_one_undo_step() {
    let mut app = app();
    app.run("shape.rectangle", json!({"x": 180, "y": 40, "width": 40, "height": 40})).unwrap();
    let id = app.run("object.repeat.radial", json!({"instances": 6, "radius": 60})).unwrap()["id"].as_u64().unwrap();
    let l = layout(&mut app, ALL);
    let m = l.radial.unwrap();
    let (centre, _) = m.circle.unwrap();
    let splitter = m.handles[2].0;
    let (v, mut drag) = (view(&app), None);
    advance(&mut app, &mut drag, Widget::Radial, Gesture::Press(v.to_doc(splitter)), Mods::default());
    // Back round the circle in 30 degree moves: past two of the six instances.
    let from = splitter - centre;
    let mut last = v.to_doc(splitter);
    for k in 1..=4 {
        let a = (-30.0_f32 * k as f32).to_radians();
        let turned = Vec2::new(from.x * a.cos() - from.y * a.sin(), from.x * a.sin() + from.y * a.cos());
        last = v.to_doc(centre + turned);
        advance(&mut app, &mut drag, Widget::Radial, Gesture::Move(last), Mods::default());
    }
    advance(&mut app, &mut drag, Widget::Radial, Gesture::Release(last), Mods::default());
    assert_eq!(radial_of(&app, id).0, 4);
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(radial_of(&app, id).0, 6, "the split was one step");
}

#[test]
fn the_edited_symbols_nine_slice_guides_are_drawn_and_dragged_on_the_canvas() {
    let mut app = app();
    app.run("shape.rectangle", json!({"x": 100, "y": 100, "width": 100, "height": 100})).unwrap();
    app.run("symbol.new", json!({"name": "Box", "nineSlice": true})).unwrap();
    app.run("symbol.edit", json!({})).unwrap();
    app.select_tool("directSelection");
    let l = layout(&mut app, Show { nine: true, selecting: false, shaper: false, trace: false });
    assert_eq!(l.nine.len(), 4, "the guides show with any tool");
    let lines = |app: &VectorcraftApp| app.session.active().unwrap().doc.edited_nine_slice().unwrap().lines();
    let left = lines(&app)[0].1;
    let from = l.nine[0][0] + (l.nine[0][1] - l.nine[0][0]) * 0.5;
    let v = view(&app);
    drag_through(&mut app, Widget::Nine, from, from - Vec2::new(20.0 * v.zoom as f32, 0.0));
    let moved = lines(&app)[0].1;
    assert!((moved.x - (left.x - 20.0)).abs() < 1e-3, "{left:?} to {moved:?}");
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(lines(&app)[0].1, left);
}

#[test]
fn a_press_cancel_mid_drag_gives_one_cancel_and_no_undo_step() {
    let (mut app, id) = mirror_app();
    let before = node(&app, id);
    let l = layout(&mut app, ALL);
    let (_, centre) = l.mirror.unwrap();
    let v = view(&app);
    let mut drag = None;
    let mut steps = vec![];
    let to = v.to_doc(centre + Vec2::new(40.0, 0.0));
    for g in [Gesture::Press(v.to_doc(centre)), Gesture::Move(to), Gesture::Cancel, Gesture::Release(to)] {
        let actions = plan(&mut app, &mut drag, Widget::Mirror, g, Mods::default());
        steps.push(actions.clone());
        app.session.apply_actions(actions).unwrap();
    }
    assert_eq!(steps[2], vec![Action::Cancel]);
    assert!(steps[3].is_empty(), "the release after a cancel does nothing");
    assert!(drag.is_none());
    assert_eq!(node(&app, id), before);
}

#[test]
fn a_mirror_axis_drag_reads_its_release_and_a_tool_switch_cancels_it() {
    let (mut app, _) = mirror_app();
    let (_, centre) = layout(&mut app, ALL).mirror.unwrap();
    check_release_away(&mut app, Widget::Mirror, centre, centre + Vec2::new(20.0, 0.0), centre + Vec2::new(40.0, 0.0));
    check_tool_switch_cancels(&mut app, Widget::Mirror, centre, centre + Vec2::new(20.0, 0.0));
}

#[test]
fn a_grid_repeat_handle_drag_reads_its_release_and_a_tool_switch_cancels_it() {
    let mut app = app();
    app.run("shape.rectangle", json!({"x": 40, "y": 40, "width": 40, "height": 40})).unwrap();
    app.run("object.repeat.grid", json!({"hSpacing": 10, "vSpacing": 10})).unwrap();
    let rows_handle = layout(&mut app, ALL).grid[0].at;
    let step = 50.0 * view(&app).zoom as f32;
    let (q, r) = (rows_handle + Vec2::new(0.0, 2.0 * step), rows_handle + Vec2::new(0.0, 4.0 * step));
    check_release_away(&mut app, Widget::Grid, rows_handle, q, r);
    check_tool_switch_cancels(&mut app, Widget::Grid, rows_handle, q);
}

#[test]
fn a_radial_repeat_handle_drag_reads_its_release_and_a_tool_switch_cancels_it() {
    let mut app = app_with_radial();
    let instances = layout(&mut app, ALL).radial.unwrap().handles[0].0;
    let step = repeat_radial::INSTANCE_STEP as f32;
    let (q, r) = (instances + Vec2::new(0.0, 2.0 * step), instances + Vec2::new(0.0, 4.0 * step));
    check_release_away(&mut app, Widget::Radial, instances, q, r);
    check_tool_switch_cancels(&mut app, Widget::Radial, instances, q);
}

/// The guides are the canvas's, not the Selection tool's: they stay with any tool, and so does their drag.
#[test]
fn a_nine_slice_guide_drag_reads_its_release() {
    let mut app = app();
    app.run("shape.rectangle", json!({"x": 100, "y": 100, "width": 100, "height": 100})).unwrap();
    app.run("symbol.new", json!({"name": "Box", "nineSlice": true})).unwrap();
    app.run("symbol.edit", json!({})).unwrap();
    let l = layout(&mut app, ALL);
    let from = l.nine[0][0] + (l.nine[0][1] - l.nine[0][0]) * 0.5;
    let unit = view(&app).zoom as f32;
    check_release_away(&mut app, Widget::Nine, from, from - Vec2::new(10.0 * unit, 0.0), from - Vec2::new(20.0 * unit, 0.0));
}

#[test]
fn no_part_is_declared_where_a_widget_is_hidden() {
    let (mut app, _) = mirror_app();
    assert!(widgets(&layout(&mut app, ALL), Widget::Mirror) > 0);
    assert_eq!(widgets(&layout(&mut app, Show { selecting: false, ..ALL }), Widget::Mirror), 0, "Edges hidden");
    app.ui.view.bounding_box = false;
    assert_eq!(widgets(&layout(&mut app, ALL), Widget::Mirror), 0, "Bounding Box hidden");
    let mut other = app_with_radial();
    let l = layout(&mut other, ALL);
    assert_eq!((widgets(&l, Widget::Mirror), widgets(&l, Widget::Grid)), (0, 0), "another repeat kind");
    assert!(widgets(&l, Widget::Radial) > 0);
}

fn app_with_radial() -> VectorcraftApp {
    let mut app = app();
    app.run("shape.rectangle", json!({"x": 180, "y": 40, "width": 40, "height": 40})).unwrap();
    app.run("object.repeat.radial", json!({"instances": 6, "radius": 60})).unwrap();
    app
}
