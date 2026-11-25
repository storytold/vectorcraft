//! The bounding box, Live Corners and the shape widgets, through the real pointer path and the inputs of the
//! controls that draw them.

use super::*;
use serde_json::{Value, json};
use vectorcraft_engine::Session;
use vectorcraft_tools::{PointerEvent, PointerKind};

const NONE: egui::Modifiers = egui::Modifiers::NONE;

fn new_app(prefs: &[(&str, Value)]) -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    for (key, value) in prefs {
        app.session.execute("prefs.set", &json!({"key": key, "value": value})).unwrap();
    }
    app.ui.view.smart_guides = false;
    app.ui.view.snap_to_point = false;
    app
}

fn make(app: &mut VectorcraftApp, command: &str, params: Value) -> u64 {
    app.session.execute(command, &params).unwrap()["id"].as_u64().unwrap()
}

fn pointer(app: &mut VectorcraftApp, kind: PointerKind, (x, y): (f64, f64)) {
    let view = app.view_info();
    app.session.pointer(&PointerEvent::new(kind, x, y), view).unwrap();
}

fn drag(app: &mut VectorcraftApp, from: (f64, f64), to: (f64, f64)) {
    pointer(app, PointerKind::Down, from);
    pointer(app, PointerKind::Drag, to);
}

fn layer(app: &VectorcraftApp, id: u64) -> DataColor {
    data_color(app.session.active().unwrap().doc.layer_color(vectorcraft_doc::NodeId(id)))
}

/// The tool whose widgets show with `held` and the pointer off the canvas: while Cmd is held, the selection tool it
/// lends.
fn shown(app: &VectorcraftApp, held: egui::Modifiers) -> &'static str {
    app.session.hover_tool_id(&super::super::mods(held, false))
}

/// The bounding box on show with `held`.
fn boxed(app: &mut VectorcraftApp, held: egui::Modifiers) -> Option<ShownBox> {
    let tool = shown(app, held);
    bounding_box(app, tool)
}

/// The shape widgets on show with `held`.
fn marks_with(app: &mut VectorcraftApp, held: egui::Modifiers) -> Vec<Mark> {
    let tool = shown(app, held);
    let shown = bounding_box(app, tool);
    let zoom = app.view_info().zoom;
    ShapeWidgets::of(app, tool, true, shown.as_ref(), zoom, 0.0).marks
}

fn marks(app: &mut VectorcraftApp) -> Vec<Mark> {
    marks_with(app, NONE)
}

fn count(marks: &[Mark], form: Form, radius: f32) -> usize {
    marks.iter().filter(|m| m.form == form && m.radius == radius).count()
}

fn corner_count(app: &VectorcraftApp) -> usize {
    let zoom = app.view_info().zoom;
    corners(app, app.session.tool_id(), zoom).map_or(0, |groups| groups.iter().flat_map(|(_, targets)| targets).map(|t| t.corners.len()).sum())
}

#[test]
fn real_time_drawing_off_moves_the_bounding_box_with_a_move_drag() {
    for (real_time, while_moving) in [(true, false), (false, false), (true, true), (false, true)] {
        let what = format!("real time {real_time}, box while moving {while_moving}");
        let mut app = new_app(&[("realTimeDrawing", json!(real_time)), ("boundingBoxWhileMovingAndDrawing", json!(while_moving))]);
        make(&mut app, "shape.rectangle", json!({"x": 100, "y": 100, "width": 100, "height": 60}));
        app.select_tool("selection");
        assert!(boxed(&mut app, NONE).is_some(), "{what}: at rest");
        drag(&mut app, (150.0, 130.0), (250.0, 230.0));
        let during = boxed(&mut app, NONE);
        if real_time && !while_moving {
            assert!(during.is_none(), "{what}: hidden while the art moves");
        } else {
            let rect = during.unwrap().obox.rect;
            assert_eq!((rect.x0, rect.y0), (200.0, 200.0), "{what}: where the drag carries the selection");
        }
        pointer(&mut app, PointerKind::Up, (250.0, 230.0));
        assert_eq!(boxed(&mut app, NONE).unwrap().obox.rect.x0, 200.0, "{what}: at rest after the release");
    }
}

#[test]
fn corner_widgets_hide_above_the_preference_angle() {
    let mut app = new_app(&[]);
    make(&mut app, "shape.rectangle", json!({"x": 100, "y": 100, "width": 100, "height": 100}));
    app.select_tool("selection");
    assert_eq!(corner_count(&app), 4);
    app.session.execute("prefs.set", &json!({"key": "hideCornerWidgetAbove", "value": 80})).unwrap();
    assert!(corners(&app, app.session.tool_id(), app.view_info().zoom).is_none());
}

#[test]
fn a_resize_drag_keeps_the_bounding_box_at_the_new_bounds() {
    for real_time in [true, false] {
        let mut app = new_app(&[("realTimeDrawing", json!(real_time))]);
        make(&mut app, "shape.ellipse", json!({"x": 100, "y": 100, "width": 100, "height": 60}));
        app.select_tool("selection");
        let before = boxed(&mut app, NONE).unwrap();
        drag(&mut app, (200.0, 160.0), (263.0, 211.0));
        assert!(app.session.tool_resizing(), "real time {real_time}: the handle was hit");
        let during = boxed(&mut app, NONE).unwrap();
        pointer(&mut app, PointerKind::Up, (263.0, 211.0));
        let after = boxed(&mut app, NONE).unwrap();
        assert_ne!(after.obox.rect, before.obox.rect, "real time {real_time}: the drag resized the ellipse");
        assert_eq!(during.obox.rect, after.obox.rect, "real time {real_time}: the box followed the drag");
    }
}

#[test]
fn a_move_drag_keeps_the_bounding_box_where_the_drag_carries_the_selection() {
    let mut app = new_app(&[("boundingBoxWhileMovingAndDrawing", json!(true))]);
    make(&mut app, "shape.rectangle", json!({"x": 100, "y": 100, "width": 100, "height": 100}));
    app.select_tool("selection");
    drag(&mut app, (150.0, 150.0), (170.0, 160.0));
    let rect = boxed(&mut app, NONE).unwrap().obox.rect;
    assert_eq!((rect.x0, rect.y0), (120.0, 110.0));
    app.ui.view.bounding_box = false;
    assert!(boxed(&mut app, NONE).is_none(), "View › Hide Bounding Box");
    app.ui.view.bounding_box = true;
    app.session.execute("prefs.set", &json!({"key": "boundingBoxWhileMovingAndDrawing", "value": false})).unwrap();
    assert!(boxed(&mut app, NONE).is_none(), "off: hidden while the selection moves");
}

#[test]
fn a_shape_being_drawn_shows_its_bounding_box() {
    for real_time in [true, false] {
        for tool in ["rectangle", "ellipse", "polygon", "star"] {
            let what = format!("{tool}, real time {real_time}");
            let mut app = new_app(&[("realTimeDrawing", json!(real_time)), ("boundingBoxWhileMovingAndDrawing", json!(true))]);
            app.select_tool(tool);
            drag(&mut app, (150.0, 120.0), (180.0, 140.0));
            assert!(app.session.draws_new_shape_box(), "{what}");
            let first = boxed(&mut app, NONE).unwrap_or_else(|| panic!("{what}: no box")).obox;
            pointer(&mut app, PointerKind::Drag, (210.0, 170.0));
            let during = boxed(&mut app, NONE).unwrap().obox;
            assert_ne!(first.rect, during.rect, "{what}: the box follows the drag");
            app.session.execute("prefs.set", &json!({"key": "boundingBoxWhileMovingAndDrawing", "value": false})).unwrap();
            assert!(boxed(&mut app, NONE).is_none(), "{what}: off, no box while drawing");
        }
    }
}

#[test]
fn a_failed_preview_of_a_shape_being_drawn_shows_no_bounding_box() {
    for real_time in [true, false] {
        let mut app = new_app(&[("realTimeDrawing", json!(real_time)), ("boundingBoxWhileMovingAndDrawing", json!(true))]);
        make(&mut app, "shape.rectangle", json!({"x": 20, "y": 20, "width": 30, "height": 30}));
        app.select_tool("rectangle");
        drag(&mut app, (150.0, 120.0), (180.0, 140.0));
        assert!(boxed(&mut app, NONE).is_some(), "real time {real_time}: the box of the shape being drawn");
        assert!(app.session.preview("shape.rectangle", &json!({"x": "left"})).is_err());
        assert!(app.session.draws_new_shape_box() && app.session.interaction_unapplied());
        assert!(boxed(&mut app, NONE).is_none(), "real time {real_time}: not the old selection's box");
        pointer(&mut app, PointerKind::Drag, (210.0, 170.0));
        assert!(boxed(&mut app, NONE).is_some(), "real time {real_time}: the next preview shows it again");
    }
}

#[test]
fn a_shape_being_drawn_has_its_box_in_its_own_layers_color() {
    for real_time in [true, false] {
        let mut app = new_app(&[("realTimeDrawing", json!(real_time)), ("boundingBoxWhileMovingAndDrawing", json!(true))]);
        let old = make(&mut app, "shape.rectangle", json!({"x": 20, "y": 20, "width": 30, "height": 30}));
        let old_color = layer(&app, old);
        let new_layer = make(&mut app, "layer.new", json!({}));
        let new_color = layer(&app, new_layer);
        assert_ne!(old_color, new_color);
        app.select_tool("rectangle");
        drag(&mut app, (150.0, 120.0), (180.0, 140.0));
        assert_eq!(boxed(&mut app, NONE).unwrap().color, new_color, "real time {real_time}");
    }
}

#[test]
fn a_shape_being_drawn_has_no_type_widget() {
    let mut app = new_app(&[("realTimeDrawing", json!(false)), ("boundingBoxWhileMovingAndDrawing", json!(true))]);
    let id = make(&mut app, "text.create", json!({"x": 40, "y": 40, "text": "Type"}));
    app.session.execute("select.set", &json!({"ids": [id]})).unwrap();
    app.select_tool("selection");
    assert_eq!(count(&marks(&mut app), Form::Disc, typewidget::RADIUS_PX), 1, "the selected type's widget");
    app.select_tool("rectangle");
    drag(&mut app, (150.0, 120.0), (210.0, 170.0));
    assert!(boxed(&mut app, NONE).is_some(), "the box of the shape being drawn");
    assert_eq!(count(&marks(&mut app), Form::Disc, typewidget::RADIUS_PX), 0, "no type widget beside it");
}

#[test]
fn freehand_and_pen_drawing_show_no_bounding_box() {
    for tool in ["pencil", "pen", "paintbrush"] {
        let mut app = new_app(&[("boundingBoxWhileMovingAndDrawing", json!(true))]);
        make(&mut app, "shape.rectangle", json!({"x": 20, "y": 20, "width": 30, "height": 30}));
        app.select_tool(tool);
        pointer(&mut app, PointerKind::Down, (150.0, 120.0));
        for at in [(170.0, 130.0), (190.0, 150.0), (210.0, 170.0)] {
            pointer(&mut app, PointerKind::Drag, at);
        }
        assert!(!app.session.draws_new_shape_box(), "{tool}");
        assert!(boxed(&mut app, NONE).is_none(), "{tool}: no box while it draws");
        pointer(&mut app, PointerKind::Up, (210.0, 170.0));
    }
}

#[test]
fn a_move_rotate_or_corner_radius_drag_hides_the_bounding_box() {
    let mut app = new_app(&[]);
    make(&mut app, "shape.rectangle", json!({"x": 100, "y": 100, "width": 100, "height": 100}));
    app.select_tool("selection");
    let offset = 10.0 / app.view_info().zoom;
    // A move, a rotation from just outside a corner and a corner-radius drag, each hiding the box until the release.
    for what in ["a move", "a rotation", "a corner-radius drag"] {
        let (from, to) = match what {
            "a move" => ((150.0, 150.0), (160.0, 160.0)),
            "a rotation" => ((210.0 + offset, 210.0 + offset), (260.0, 160.0)),
            // A corner widget of the rectangle as the move and the rotation left it.
            _ => {
                let st = app.session.active().unwrap();
                let corner = vectorcraft_tools::corners::CornerWidgets::of(&st.doc, &st.selection, app.view_info().zoom, false)
                    .unwrap()
                    .visible()
                    .next()
                    .unwrap();
                ((corner.x, corner.y), (corner.x + 10.0, corner.y + 10.0))
            }
        };
        assert!(boxed(&mut app, NONE).is_some(), "{what}: at rest");
        drag(&mut app, from, to);
        assert!(app.session.tool_transforming(), "{what}");
        assert!(boxed(&mut app, NONE).is_none(), "{what}: hidden during the drag");
        pointer(&mut app, PointerKind::Up, to);
    }
}

#[test]
fn direct_selection_shows_the_bounding_box_while_cmd_is_held() {
    let mut app = new_app(&[]);
    make(&mut app, "shape.rectangle", json!({"x": 100, "y": 100, "width": 100, "height": 50}));
    app.select_tool("directSelection");
    assert!(boxed(&mut app, NONE).is_none(), "Direct Selection alone shows no box");
    assert!(boxed(&mut app, egui::Modifiers::COMMAND).is_some(), "Cmd held shows the Selection tool's box");
    assert!(boxed(&mut app, NONE).is_none(), "gone once Cmd is released");
    assert!(app.ui.view.bounding_box, "View › Show Bounding Box is unchanged");
}

#[test]
fn a_star_shows_corner_widgets_with_direct_selection() {
    let mut app = new_app(&[]);
    make(&mut app, "shape.star", json!({"cx": 200, "cy": 150, "radius1": 80, "radius2": 40}));
    app.select_tool("selection");
    assert_eq!(corner_count(&app), 0);
    app.select_tool("directSelection");
    assert_eq!(corner_count(&app), 10);
    app.run("view.cornerWidget", json!({})).unwrap();
    assert_eq!(corner_count(&app), 0);
}

#[test]
fn the_type_widget_shows_which_kind_the_type_is() {
    let mut app = new_app(&[]);
    let id = make(&mut app, "text.create", json!({"x": 100, "y": 100, "text": "Type"}));
    app.session.execute("select.set", &json!({"ids": [id]})).unwrap();
    app.select_tool("selection");
    let widget = |app: &mut VectorcraftApp| -> Vec<Filling> {
        marks(app).iter().filter(|m| m.form == Form::Disc && m.radius == typewidget::RADIUS_PX).map(|m| m.filling).collect()
    };
    assert_eq!(widget(&mut app), vec![Filling::Hollow]);
    app.session.execute("type.convertToAreaType", &json!({})).unwrap();
    assert_eq!(widget(&mut app), vec![Filling::Layer]);
    app.ui.view.bounding_box = false;
    assert!(widget(&mut app).is_empty(), "no bounding box, no widget");
}

#[test]
fn a_selected_ellipse_shows_its_pie_widgets() {
    let mut app = new_app(&[]);
    make(&mut app, "shape.ellipse", json!({"x": 100, "y": 100, "width": 200, "height": 100}));
    app.session.execute("object.setLiveShape", &json!({"pieStart": 0, "pieEnd": 90})).unwrap();
    app.select_tool("selection");
    let pies = marks(&mut app);
    assert_eq!(count(&pies, Form::Disc, pie::RADIUS_PX), 2);
    assert!(pies.iter().filter(|m| m.radius == pie::RADIUS_PX).all(|m| m.join.is_some()), "each joined to its outline point");
    app.select_tool("directSelection");
    assert_eq!(count(&marks(&mut app), Form::Disc, pie::RADIUS_PX), 0);
    app.select_tool("selection");
    app.run("view.boundingBox", json!({})).unwrap();
    assert_eq!(count(&marks(&mut app), Form::Disc, pie::RADIUS_PX), 0, "no bounding box, no widgets");
}

/// With a shape tool, Cmd held lends the selection tool chosen last; when that is Direct Selection, which does not
/// press them, a selected live ellipse's pie widgets hide while the key is down.
#[test]
fn pie_widgets_hide_while_cmd_lends_direct_selection() {
    let mut app = new_app(&[]);
    make(&mut app, "shape.ellipse", json!({"x": 100, "y": 100, "width": 200, "height": 100}));
    app.session.execute("object.setLiveShape", &json!({"pieStart": 0, "pieEnd": 90})).unwrap();
    app.select_tool("directSelection");
    app.select_tool("ellipse");
    let pies = |app: &mut VectorcraftApp, held| count(&marks_with(app, held), Form::Disc, pie::RADIUS_PX);
    assert_eq!(pies(&mut app, NONE), 2, "the Ellipse tool shows them");
    assert_eq!(pies(&mut app, egui::Modifiers::COMMAND), 0, "hidden while Cmd lends Direct Selection");
    assert_eq!(pies(&mut app, NONE), 2, "back when the key is let go");
}

/// With the Selection tool, Cmd held lends Direct Selection away from the selection, and a selected blend's widgets,
/// which only the Selection and Blend tools press, hide while the key is down.
#[test]
fn blend_widgets_hide_while_cmd_lends_direct_selection() {
    let mut app = new_app(&[]);
    let a = make(&mut app, "shape.rectangle", json!({"x": 90, "y": 190, "width": 20, "height": 20}));
    let b = make(&mut app, "shape.rectangle", json!({"x": 290, "y": 190, "width": 20, "height": 20}));
    app.session.execute("select.set", &json!({"ids": [a, b]})).unwrap();
    let blend = make(&mut app, "object.blend.make", json!({}));
    app.select_tool("selection");
    app.session.execute("select.set", &json!({"ids": [blend]})).unwrap();
    let widgets = |app: &mut VectorcraftApp, held| count(&marks_with(app, held), Form::Disc, blendwidget::RADIUS_PX);
    let plain = widgets(&mut app, NONE);
    assert!(plain >= 1, "the Appearance widget: {plain}");
    assert_eq!(widgets(&mut app, egui::Modifiers::COMMAND), 0, "hidden while Cmd lends Direct Selection");
    assert_eq!(widgets(&mut app, NONE), plain, "back when the key is let go");
}

#[test]
fn a_selected_polygon_shows_its_side_widget() {
    let mut app = new_app(&[]);
    make(&mut app, "shape.polygon", json!({"cx": 200, "cy": 150, "radius": 60, "sides": 5}));
    let sides = |app: &mut VectorcraftApp| count(&marks(app), Form::Diamond, sidewidget::RADIUS_PX);
    for (tool, shown) in [("selection", 1), ("polygon", 1), ("directSelection", 0), ("lineSegment", 0)] {
        app.select_tool(tool);
        assert_eq!(sides(&mut app), shown, "{tool}");
    }
    app.select_tool("selection");
    app.run("view.boundingBox", json!({})).unwrap();
    assert_eq!(sides(&mut app), 0, "no bounding box, no widget");
    app.run("view.boundingBox", json!({})).unwrap();
    drag(&mut app, (200.0, 150.0), (260.0, 150.0));
    assert_eq!(sides(&mut app), 0, "hidden while the polygon moves");
    pointer(&mut app, PointerKind::Up, (260.0, 150.0));
    assert_eq!(sides(&mut app), 1, "back after the release");
    make(&mut app, "shape.ellipse", json!({"x": 10, "y": 10, "width": 50, "height": 50}));
    assert_eq!(sides(&mut app), 0, "an ellipse has none");
}

#[test]
fn hidden_shape_widgets_leave_the_bounding_box() {
    let mut app = new_app(&[]);
    let rect = make(&mut app, "shape.rectangle", json!({"x": 20, "y": 20, "width": 80, "height": 80}));
    let polygon = make(&mut app, "shape.polygon", json!({"cx": 200, "cy": 150, "radius": 50, "sides": 5}));
    let ellipse = make(&mut app, "shape.ellipse", json!({"x": 280, "y": 100, "width": 100, "height": 60}));
    // The corner widgets, the side and pie widgets, and whether the box shows, with `id` selected.
    let drawn = |app: &mut VectorcraftApp, id: u64| {
        app.session.execute("select.set", &json!({"ids": [id]})).unwrap();
        let m = marks(app);
        (corner_count(app), count(&m, Form::Diamond, sidewidget::RADIUS_PX), count(&m, Form::Disc, pie::RADIUS_PX), boxed(app, NONE).is_some())
    };
    for tool in ["selection", "polygon", "rectangle"] {
        app.select_tool(tool);
        assert_eq!(drawn(&mut app, rect), (4, 0, 0, true), "{tool}");
        assert_eq!(drawn(&mut app, polygon), (5, 1, 0, true), "{tool}");
        assert_eq!(drawn(&mut app, ellipse), (0, 0, 2, true), "{tool}");
        assert_eq!(app.run("view.shapeWidgets", json!({})).unwrap(), json!(false));
        assert_eq!(drawn(&mut app, rect), (0, 0, 0, true), "{tool}");
        assert_eq!(drawn(&mut app, polygon), (0, 0, 0, true), "{tool}");
        assert_eq!(drawn(&mut app, ellipse), (0, 0, 0, true), "{tool}");
        assert_eq!(app.run("view.shapeWidgets", json!({})).unwrap(), json!(true));
    }
    // Direct Selection keeps its corner widgets: they follow View › Hide Corner Widget alone.
    app.run("view.shapeWidgets", json!({"on": false})).unwrap();
    app.select_tool("directSelection");
    assert_eq!(drawn(&mut app, rect).0, 4);
    app.run("view.cornerWidget", json!({})).unwrap();
    assert_eq!(drawn(&mut app, rect).0, 0);
}

/// A group of `n` rectangles on a line, selected as the group, and the group's id.
fn objects_on_path(app: &mut VectorcraftApp, n: u32) -> (u64, Vec<u64>) {
    let line = make(app, "shape.line", json!({"x1": 20, "y1": 200, "x2": 380, "y2": 200}));
    let members: Vec<u64> = (0..n).map(|i| make(app, "shape.rectangle", json!({"x": 40 + 40 * i, "y": 40, "width": 20, "height": 20}))).collect();
    app.run("select.set", json!({"ids": members})).unwrap();
    let group = app.session.execute("object.objectsOnPath.attach", &json!({"path": line})).unwrap()["id"].as_u64().unwrap();
    (group, members)
}

/// How many widgets of `form` and `radius` show for each of the tools, then for one object of the group.
fn on_tools(app: &mut VectorcraftApp, member: u64, form: Form, radius: f32) -> [usize; 4] {
    let mut out = [0; 4];
    for (i, tool) in ["selection", "objectsOnPath", "directSelection"].into_iter().enumerate() {
        app.select_tool(tool);
        out[i] = count(&marks(app), form, radius);
    }
    app.select_tool("selection");
    app.run("select.set", json!({"ids": [member]})).unwrap();
    out[3] = count(&marks(app), form, radius);
    out
}

#[test]
fn a_selected_objects_on_path_group_shows_its_rotate_all_widget() {
    let mut app = new_app(&[]);
    let (group, members) = objects_on_path(&mut app, 2);
    let r = objects_on_path::ROTATE_ALL_RADIUS_PX;
    assert_eq!(on_tools(&mut app, members[0], Form::Disc, r), [1, 1, 0, 0], "an object in the group, not the group");
    // It is a control, so it stays with the edges hidden.
    app.run("select.set", json!({"ids": [group]})).unwrap();
    let zoom = app.view_info().zoom;
    app.select_tool("selection");
    assert_eq!(count(&ShapeWidgets::of(&app, "selection", false, None, zoom, 0.0).marks, Form::Disc, r), 1);
}

#[test]
fn a_selected_objects_on_path_group_shows_its_space_widget() {
    let mut app = new_app(&[]);
    let (group, members) = objects_on_path(&mut app, 2);
    let half = objects_on_path::SPACE_HALF_PX;
    assert_eq!(on_tools(&mut app, members[0], Form::Square, half), [1, 1, 0, 0]);
    app.run("edit.clear", json!({"ids": [members[1]]})).unwrap();
    app.run("select.set", json!({"ids": [group]})).unwrap();
    assert_eq!(count(&marks(&mut app), Form::Square, half), 0, "one object has no space to set");
}

#[test]
fn a_selected_objects_on_path_group_shows_its_move_all_widget_when_it_has_room() {
    let mut app = new_app(&[]);
    let (group, members) = objects_on_path(&mut app, 2);
    let half = objects_on_path::MOVE_ALL_HALF_PX;
    app.select_tool("selection");
    app.run("select.set", json!({"ids": [group]})).unwrap();
    assert_eq!(count(&marks(&mut app), Form::Diamond, half), 0, "Auto: the objects reach both ends");
    app.session.execute("object.objectsOnPath.options", &json!({"ids": [group], "spacing": 60})).unwrap();
    assert_eq!(on_tools(&mut app, members[0], Form::Diamond, half), [1, 1, 0, 0]);
    app.session.execute("object.objectsOnPath.options", &json!({"ids": [group], "spacing": "auto"})).unwrap();
    app.run("edit.clear", json!({"ids": [members[1]]})).unwrap();
    app.run("select.set", json!({"ids": [group]})).unwrap();
    assert_eq!(count(&marks(&mut app), Form::Diamond, half), 1, "one object can move along the path whatever its Space");
}

#[test]
fn a_selected_objects_on_path_group_shows_a_select_move_widget_per_object() {
    let mut app = new_app(&[]);
    let (_, members) = objects_on_path(&mut app, 3);
    let r = objects_on_path::MEMBER_RADIUS_PX;
    let by_filling = |app: &mut VectorcraftApp| {
        let m = marks(app);
        let of = |f: Filling| m.iter().filter(|m| m.radius == r && m.filling == f).count();
        (of(Filling::Ring), of(Filling::Chosen))
    };
    app.select_tool("selection");
    assert_eq!(by_filling(&mut app), (3, 0));
    app.run("object.objectsOnPath.selectMember", json!({"member": members[1]})).unwrap();
    assert_eq!(by_filling(&mut app), (2, 1), "the chosen one is marked");
    app.select_tool("objectsOnPath");
    assert_eq!(by_filling(&mut app), (2, 1));
    app.select_tool("directSelection");
    assert_eq!(by_filling(&mut app), (0, 0));
    app.select_tool("selection");
    app.run("select.set", json!({"ids": [members[0]]})).unwrap();
    assert_eq!(by_filling(&mut app), (0, 0), "an object in the group, not the group");
}
