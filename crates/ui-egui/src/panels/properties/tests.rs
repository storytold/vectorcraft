use egui::{Event, PointerButton, Pos2, Rect, vec2};
use vectorcraft_engine::Session;

use super::*;

/// One frame of the panel with `events` → the texts painted, with their rects.
fn frame(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<Event>) -> Vec<(String, Rect)> {
    panel_frame(app, ctx, events, show)
}

/// One frame of `draw` with `events` → the texts painted, with their rects.
fn panel_frame(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<Event>, draw: fn(&mut VectorcraftApp, &mut Ui)) -> Vec<(String, Rect)> {
    let raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(300.0, 700.0))), events, ..Default::default() };
    let mut out = ctx.run_ui(raw, |ui| draw(app, ui));
    out.textures_delta.clear();
    out.shapes
        .iter()
        .filter_map(|s| {
            if let egui::Shape::Text(t) = &s.shape { Some((t.galley.text().to_string(), Rect::from_min_size(t.pos, t.galley.size()))) } else { None }
        })
        .collect()
}

/// Clicks the text `label` where the last frame painted it, over two frames (press, release).
fn click(app: &mut VectorcraftApp, ctx: &egui::Context, texts: &[(String, Rect)], label: &str) {
    let at = texts.iter().find(|(t, _)| t == label).map(|(_, r)| r.center()).unwrap_or_else(|| panic!("no {label:?} in {texts:?}"));
    let b = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() };
    frame(app, ctx, vec![Event::PointerMoved(at), b(true)]);
    frame(app, ctx, vec![b(false)]);
}

/// One selected clip group has Edit Clipping Path and Edit Contents at the top of the panel,
/// under its Clip Group heading and before every section, and they select its clipping path or
/// its clipped art; one that clips nothing has Edit Clipping Path alone. A plain group, or a clip
/// group selected with another object, has neither.
#[test]
fn one_clip_group_has_edit_clipping_path_and_edit_contents_at_the_top() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    let art = app.run("shape.rectangle", json!({"x": 100, "y": 80, "width": 200, "height": 140})).unwrap()["id"].clone();
    let mask = app.run("shape.ellipse", json!({"x": 150, "y": 100, "width": 100, "height": 100})).unwrap()["id"].clone();
    app.run("select.set", json!({"ids": [art, mask]})).unwrap();
    let group = app.run("object.clippingMask.make", json!({})).unwrap()["id"].clone();
    app.run("select.set", json!({"ids": [group]})).unwrap();
    let ctx = egui::Context::default();
    let selected = |app: &VectorcraftApp| app.session.active().unwrap().selection.objects.iter().map(|n| json!(n.0)).collect::<Vec<_>>();
    let labels = |texts: &[(String, Rect)]| texts.iter().map(|(t, _)| t.clone()).collect::<Vec<_>>();
    let texts = frame(&mut app, &ctx, vec![]);
    let shown = labels(&texts);
    let at = |label: &str| shown.iter().position(|t| t == label).unwrap_or_else(|| panic!("{label} in {shown:?}"));
    assert_eq!(at("Clip Group"), 0, "{shown:?}");
    assert!(at("Edit Clipping Path") == 1 && at("Edit Contents") == 2, "{shown:?}");
    click(&mut app, &ctx, &texts, "Edit Contents");
    assert_eq!(selected(&app), std::slice::from_ref(&art), "Edit Contents selects the clipped art");
    app.run("select.set", json!({"ids": [group]})).unwrap();
    let texts = frame(&mut app, &ctx, vec![]);
    click(&mut app, &ctx, &texts, "Edit Clipping Path");
    assert_eq!(selected(&app), std::slice::from_ref(&mask), "Edit Clipping Path selects the clipping path");
    let neither = |app: &mut VectorcraftApp| {
        let shown = labels(&frame(app, &ctx, vec![]));
        !shown.iter().any(|t| t == "Edit Clipping Path" || t == "Edit Contents")
    };
    // The clip group with another object, whichever is selected first.
    let other = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 40, "height": 40})).unwrap()["id"].clone();
    app.run("select.set", json!({"ids": [group, other]})).unwrap();
    assert!(neither(&mut app), "a clip group and another object");
    app.run("select.set", json!({"ids": [other, group]})).unwrap();
    assert!(neither(&mut app), "another object and a clip group");
    // A plain group.
    let b = app.run("shape.rectangle", json!({"x": 60, "y": 10, "width": 40, "height": 40})).unwrap()["id"].clone();
    app.run("select.set", json!({"ids": [other, b]})).unwrap();
    let plain = app.run("object.group", json!({})).unwrap()["id"].clone();
    app.run("select.set", json!({"ids": [plain]})).unwrap();
    assert!(labels(&frame(&mut app, &ctx, vec![])).iter().any(|t| t == "Ungroup"));
    assert!(neither(&mut app), "a plain group");
    // A clip group whose clipped art is deleted: Edit Clipping Path alone.
    app.run("select.set", json!({"ids": [art]})).unwrap();
    app.run("edit.clear", json!({})).unwrap();
    app.run("select.set", json!({"ids": [group]})).unwrap();
    let texts = frame(&mut app, &ctx, vec![]);
    let shown = labels(&texts);
    assert_eq!(shown.iter().position(|t| t == "Edit Clipping Path"), Some(1), "{shown:?}");
    assert!(!shown.iter().any(|t| t == "Edit Contents"), "nothing to edit: {shown:?}");
    click(&mut app, &ctx, &texts, "Edit Clipping Path");
    assert_eq!(selected(&app), std::slice::from_ref(&mask), "Edit Clipping Path selects the clipping path");
}

/// Start Global Edit in Quick Actions starts it on the one selected path, the button and the
/// Select menu item then read Stop Global Edit, and Escape (the app's key handling) stops it.
/// Selected type has no Start Global Edit button.
#[test]
fn start_global_edit_quick_action_and_escape() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    let a = app.run("shape.rectangle", json!({"x": 20, "y": 20, "width": 40, "height": 40})).unwrap()["id"].clone();
    app.run("shape.rectangle", json!({"x": 200, "y": 20, "width": 80, "height": 80})).unwrap();
    app.run("select.set", json!({"ids": [a]})).unwrap();
    let ctx = egui::Context::default();
    // Tall enough for Quick Actions at the foot of the panel.
    let tall = |app: &mut VectorcraftApp, events: Vec<Event>| {
        let raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(300.0, 1600.0))), events, ..Default::default() };
        let mut out = ctx.run_ui(raw, |ui| show(app, ui));
        out.textures_delta.clear();
        out.shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Text(t) => Some((t.galley.text().to_string(), Rect::from_min_size(t.pos, t.galley.size()))),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    let on = |app: &VectorcraftApp| app.session.active().unwrap().global_edit.is_some();
    let label = |app: &VectorcraftApp| crate::menus::dynamic_label(app, "select.globalEdit", "Start Global Edit");
    assert_eq!(label(&app), "Start Global Edit");
    let texts = tall(&mut app, vec![]);
    let at = texts.iter().find(|(t, _)| t == "Start Global Edit").map(|(_, r)| r.center()).unwrap_or_else(|| panic!("{texts:?}"));
    let b = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() };
    tall(&mut app, vec![Event::PointerMoved(at), b(true)]);
    tall(&mut app, vec![b(false)]);
    assert!(on(&app), "the Quick Action starts Global Edit");
    assert_eq!(app.session.active().unwrap().global_edit.as_ref().unwrap().members.len(), 2);
    assert!(tall(&mut app, vec![]).iter().any(|(t, _)| t == "Stop Global Edit"));
    assert_eq!(label(&app), "Stop Global Edit");
    let escape = Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() };
    let raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(300.0, 1600.0))), events: vec![escape], ..Default::default() };
    let mut out = ctx.run_ui(raw, |ui| crate::shortcuts::handle(&mut app, ui.ctx()));
    out.textures_delta.clear();
    assert!(!on(&app), "Escape stops Global Edit");
    assert_eq!(label(&app), "Start Global Edit");
    assert_eq!(app.session.active().unwrap().selection.objects.iter().map(|n| json!(n.0)).collect::<Vec<_>>(), [a]);
    let t = app.run("text.create", json!({"x": 50, "y": 200, "text": "Hi"})).unwrap()["id"].clone();
    app.run("select.set", json!({"ids": [t]})).unwrap();
    assert!(!crate::menus::enabled(&app, "select.globalEdit"));
    assert!(!tall(&mut app, vec![]).iter().any(|(t, _)| t == "Start Global Edit"), "type has no Global Edit");
}

/// The options' Result(s) count is found once per document revision, options and selected
/// object, not every frame; another selected object or other options find it again.
#[test]
fn the_global_edit_result_count_is_cached() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    let a = app.run("shape.rectangle", json!({"x": 20, "y": 20, "width": 40, "height": 40})).unwrap()["id"].clone();
    app.run("shape.rectangle", json!({"x": 200, "y": 20, "width": 80, "height": 80})).unwrap();
    let e = app.run("shape.ellipse", json!({"x": 20, "y": 150, "width": 40, "height": 40})).unwrap()["id"].clone();
    app.run("select.set", json!({"ids": [a]})).unwrap();
    let ctx = egui::Context::default();
    assert_eq!(global_edit_count(&app, &ctx), 2);
    // The next frame reads the cache: a count planted under the same key is what it shows.
    let (key, _) = pstate::<GlobalEditCount>(&ctx, "props-global-edit-count").unwrap();
    set_pstate::<GlobalEditCount>(&ctx, "props-global-edit-count", Some((key, 7)));
    assert_eq!(global_edit_count(&app, &ctx), 7);
    app.run("select.set", json!({"ids": [e]})).unwrap();
    assert_eq!(global_edit_count(&app, &ctx), 1, "another selected object");
    app.run("select.set", json!({"ids": [a]})).unwrap();
    assert_eq!(global_edit_count(&app, &ctx), 2);
    app.run("select.globalEditOptions", json!({"size": true})).unwrap();
    assert_eq!(global_edit_count(&app, &ctx), 1, "other options");
}

// Regression test for #1910: the Properties panel's flip buttons are named as the reference
// app's Properties panel names them, not Flip Horizontal and Flip Vertical (the Transform
// panel menu's names).
#[test]
fn the_flip_buttons_keep_the_reference_properties_names() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
    let id = app.run("shape.rectangle", json!({"x": 50, "y": 50, "width": 120, "height": 80})).unwrap()["id"].clone();
    app.run("select.set", json!({"ids": [id]})).unwrap();
    let tree = crate::widgets::accessibility_tests::tree(|ui| transform_section(&mut app, ui));
    let names: Vec<&str> = tree.iter().filter_map(|n| n.label()).collect();
    for want in ["Flip Along Horizontal Axis", "Flip Along Vertical Axis"] {
        assert!(names.contains(&want), "{want} in {names:?}");
    }
    for old in ["Flip Horizontal", "Flip Vertical"] {
        assert!(!names.contains(&old), "{old} in {names:?}");
    }
}

/// #3451: the Properties panel Transform section shows ellipse pie angles and Invert Pie.
#[test]
fn transform_section_shows_ellipse_pie_properties() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
    app.run("shape.ellipse", json!({"x": 50, "y": 50, "width": 120, "height": 80})).unwrap();
    let ctx = egui::Context::default();
    let labels: Vec<String> = panel_frame(&mut app, &ctx, vec![], transform_section).into_iter().map(|(t, _)| t).collect();
    for t in ["Ellipse Properties:", "Pie Start Angle:", "0°", "Pie End Angle:", "360°", "Invert Pie"] {
        assert!(labels.iter().any(|s| s == t), "{t} in {labels:?}");
    }
    app.run("object.setLiveShape", json!({"pieStart": 30, "pieEnd": 120})).unwrap();
    let labels: Vec<String> = panel_frame(&mut app, &ctx, vec![], transform_section).into_iter().map(|(t, _)| t).collect();
    assert!(labels.iter().any(|s| s == "30°") && labels.iter().any(|s| s == "120°"), "{labels:?}");
}

/// The Snap Options and Scale Strokes & Effects checkboxes keep a visible box at rest in every
/// theme, as the Preferences ones do (#1080), and toggle when clicked.
#[test]
fn the_document_checkboxes_show_their_boxes_and_toggle() {
    for brightness in crate::theme::Brightness::ALL {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        crate::theme::apply(&ctx, brightness);
        frame(&mut app, &ctx, vec![]);
        let raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(300.0, 700.0))), ..Default::default() };
        let mut out = ctx.run_ui(raw, |ui| show(&mut app, ui));
        out.textures_delta.clear();
        let boxes = out
            .shapes
            .iter()
            .filter(|s| matches!(&s.shape, egui::Shape::Rect(r) if r.stroke.width > 0.0 && (r.rect.width() - 13.0).abs() < 0.5 && (r.rect.height() - 13.0).abs() < 0.5))
            .count();
        assert!(boxes >= 3, "{brightness:?}: {boxes} checkbox outlines");
    }
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let texts = frame(&mut app, &ctx, vec![]);
    let (grid, strokes) = (app.ui.view.snap_to_grid, app.session.prefs.scale_strokes);
    click(&mut app, &ctx, &texts, "Snap to Grid");
    assert_eq!(app.ui.view.snap_to_grid, !grid);
    let texts = frame(&mut app, &ctx, vec![]);
    click(&mut app, &ctx, &texts, "Scale Strokes & Effects");
    assert_eq!(app.session.prefs.scale_strokes, !strokes);
}

/// #921: with the Gradient tool, a Gradient section: the type, and for a freeform gradient
/// Draw as Points or Lines radio buttons.
#[test]
fn the_gradient_tool_shows_the_gradient_type_and_draw() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    let id = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 40})).unwrap()["id"].as_u64().unwrap();
    app.run("paint.setFill", json!({"gradient": {}})).unwrap();
    let ctx = egui::Context::default();
    let has = |texts: &[(String, Rect)], s: &str| texts.iter().any(|(t, _)| t == s);
    let texts = frame(&mut app, &ctx, vec![]);
    assert!(!has(&texts, "Gradient"), "only with the Gradient tool");
    app.select_tool("gradient");
    let texts = frame(&mut app, &ctx, vec![]);
    assert!(has(&texts, "Gradient") && has(&texts, "Type:") && !has(&texts, "Draw:"), "{texts:?}");
    app.run("paint.editGradient", json!({"kind": "freeform"})).unwrap();
    frame(&mut app, &ctx, vec![]);
    let texts = frame(&mut app, &ctx, vec![]);
    assert!(has(&texts, "Draw:") && has(&texts, "Points") && has(&texts, "Lines"), "{texts:?}");
    click(&mut app, &ctx, &texts, "Lines");
    let fill = app.session.doc().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().appearance.fill_paint();
    let vectorcraft_color::Paint::Gradient(g) = fill else { panic!("{fill:?}") };
    assert_eq!(super::super::gradient::shown_points(&g).mode, vectorcraft_color::FreeformMode::Lines);
}

/// #696: the number fields are all as wide: the Transform fields, the rotation, the corner
/// radius, Opacity and the stroke weight.
#[test]
fn the_number_fields_are_all_as_wide() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 40})).unwrap();
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    let texts = frame(&mut app, &ctx, vec![]);
    // The field each value is shown in: the narrowest widget around its text that isn't the
    // text alone.
    let field = |value: &str| {
        let at = texts.iter().find(|(t, _)| t == value).map(|(_, r)| r.center()).unwrap_or_else(|| panic!("{value}: {texts:?}"));
        ctx.viewport(|vp| {
            vp.prev_pass
                .widgets
                .layers()
                .flat_map(|(_, w)| w.iter())
                .filter(|w| w.rect.contains(at) && w.rect.width() > 30.0 && w.rect.height() < 40.0)
                .map(|w| w.rect.width())
                .fold(f32::INFINITY, f32::min)
        })
    };
    let widths = [field("35 pt"), field("0°"), field("0 pt"), field("100%")];
    assert!(widths.iter().all(|w| (w - widths[0]).abs() < 1.0), "X, rotation, corner radius, opacity: {widths:?}");
}

/// #1512: the link behind Corner Radius. On, one field sets every corner; off, a field per
/// corner shows each corner's own radius.
#[test]
fn the_corner_radius_link_shows_a_field_per_corner() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 40})).unwrap();
    app.run("object.setLiveShape", json!({"radius": 7, "corners": [2]})).unwrap();
    let ctx = egui::Context::default();
    let has = |texts: &[(String, Rect)], s: &str| texts.iter().any(|(t, _)| t == s);
    frame(&mut app, &ctx, vec![]);
    let texts = frame(&mut app, &ctx, vec![]);
    assert!(app.session.prefs.single_corner_radius, "linked at first");
    assert!(has(&texts, "Corner Radius:") && !has(&texts, "Top Left:") && !has(&texts, "7 pt"), "{texts:?}");
    app.run("prefs.set", json!({"key": "singleCornerRadius", "value": false})).unwrap();
    frame(&mut app, &ctx, vec![]);
    let texts = frame(&mut app, &ctx, vec![]);
    for l in ["Corner Radius:", "Top Left:", "Top Right:", "Bottom Right:", "Bottom Left:"] {
        assert!(has(&texts, l), "{l}: {texts:?}");
    }
    assert_eq!(texts.iter().filter(|(t, _)| t == "7 pt").count(), 1, "the bottom-right corner: {texts:?}");
    // The fields sit in line, in the order top left, top right, bottom right, bottom left.
    let at = |s: &str| texts.iter().find(|(t, _)| t == s).unwrap().1;
    assert!(at("Top Left:").top() < at("Top Right:").top() && at("Top Right:").top() < at("Bottom Right:").top());
    assert!((at("7 pt").top() - at("Bottom Right:").top()).abs() < 6.0, "{texts:?}");
}

/// The Transform section's More Options holds Scale Corners and Scale Strokes & Effects, and
/// stays open while they are toggled.
#[test]
fn transform_more_options_toggles_scale_corners_and_strokes() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    app.run("shape.rectangle", json!({"x": 10, "y": 20, "width": 30, "height": 40})).unwrap();
    let ctx = egui::Context::default();
    let has = |texts: &[(String, Rect)], s: &str| texts.iter().any(|(t, _)| t == s);
    frame(&mut app, &ctx, vec![]);
    let texts = frame(&mut app, &ctx, vec![]);
    assert!(!has(&texts, "Scale Corners"), "closed at first");
    // The button is an icon at the end of the angle row, under the H: field: sweep that row.
    let h = texts.iter().find(|(t, _)| t == "H:").unwrap().1;
    let mut open = None;
    'sweep: for dy in (12..60).step_by(6) {
        for x in (120..300).step_by(6) {
            let at = egui::pos2(x as f32, h.bottom() + dy as f32);
            let b = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() };
            frame(&mut app, &ctx, vec![Event::PointerMoved(at), b(true)]);
            frame(&mut app, &ctx, vec![b(false)]);
            let texts = frame(&mut app, &ctx, vec![]);
            if has(&texts, "Scale Corners") {
                open = Some(texts);
                break 'sweep;
            }
        }
    }
    let texts = open.expect("More Options opens the popup");
    let (sc, ss) = (app.session.prefs.scale_corners, app.session.prefs.scale_strokes);
    click(&mut app, &ctx, &texts, "Scale Corners");
    assert_eq!(app.session.prefs.scale_corners, !sc);
    let texts = frame(&mut app, &ctx, vec![]);
    assert!(has(&texts, "Scale Strokes & Effects"), "still open after a toggle");
    click(&mut app, &ctx, &texts, "Scale Strokes & Effects");
    assert_eq!(app.session.prefs.scale_strokes, !ss);
}

/// Both panels forward the Move Artwork toggle when their actual X field is edited.
#[test]
fn artboard_coordinate_fields_move_art_in_properties_and_transform() {
    for draw in [show as fn(&mut VectorcraftApp, &mut Ui), super::super::transform::show] {
        for move_art in [false, true] {
            let mut app = VectorcraftApp::new(Session::new(), Default::default());
            app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
            let id = app.run("shape.rectangle", json!({"x": 10, "y": 20, "width": 30, "height": 40})).unwrap()["id"].as_u64().unwrap();
            app.select_tool("artboard");
            app.session.set_tool_option("moveArt", &json!(move_art));
            let ctx = egui::Context::default();
            panel_frame(&mut app, &ctx, vec![], draw);
            let texts = panel_frame(&mut app, &ctx, vec![], draw);
            let label = texts.iter().find(|(t, _)| t == "X:").unwrap().1;
            let at = egui::pos2(label.right() + 40.0, label.center().y);
            let press = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() };
            panel_frame(&mut app, &ctx, vec![Event::PointerMoved(at), press(true)], draw);
            panel_frame(&mut app, &ctx, vec![press(false)], draw);
            panel_frame(
                &mut app,
                &ctx,
                vec![Event::Key { key: egui::Key::ArrowUp, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() }],
                draw,
            );
            let d = &app.session.doc().unwrap().doc;
            assert_eq!(d.artboards[0].rect.x0, 1.0, "the focused X field steps its position");
            assert_eq!(d.node(vectorcraft_doc::NodeId(id)).unwrap().geometric_bounds().unwrap().x0, if move_art { 11.0 } else { 10.0 });
            app.run("edit.undo", json!({})).unwrap();
            let d = &app.session.doc().unwrap().doc;
            assert_eq!(d.artboards[0].rect.x0, 0.0);
            assert_eq!(d.node(vectorcraft_doc::NodeId(id)).unwrap().geometric_bounds().unwrap().x0, 10.0);
        }
    }
}

/// Three landscape artboards, with Artboards 1 and 3 selected with the Artboard tool (Artboard
/// 1 active): 200 × 100, 200 × 100 and 300 × 150.
fn two_selected_artboards() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 100, "artboards": 3})).unwrap();
    app.run("artboard.setProps", json!({"index": 2, "width": 300, "height": 150})).unwrap();
    app.select_tool("artboard");
    super::super::artboards::select(&mut app, 0);
    app.session.set_tool_option("selected", &json!([0, 2]));
    app
}

/// Each artboard's (x0, y0, width, height).
fn boards(app: &VectorcraftApp) -> Vec<(f64, f64, f64, f64)> {
    app.session.doc().unwrap().doc.artboards.iter().map(|a| (a.rect.x0, a.rect.y0, a.rect.width(), a.rect.height())).collect()
}

/// #1482: with Artboards 1 and 3 selected, a W typed into Properties or the Transform panel
/// resizes both in one undo step, and Artboard 2 keeps its size. Properties keeps each top-left
/// corner; the Transform panel resizes each about its own center, its default reference point.
#[test]
fn artboard_width_resizes_every_selected_artboard_in_properties_and_transform() {
    for (draw, centered) in [(show as fn(&mut VectorcraftApp, &mut Ui), false), (super::super::transform::show, true)] {
        let mut app = two_selected_artboards();
        let before = boards(&app);
        let undo = app.session.doc().unwrap().history.undo.len();
        let ctx = egui::Context::default();
        panel_frame(&mut app, &ctx, vec![], draw);
        let texts = panel_frame(&mut app, &ctx, vec![], draw);
        let label = texts.iter().find(|(t, _)| t == "W:").unwrap().1;
        let at = egui::pos2(label.right() + 40.0, label.center().y);
        let press = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() };
        let key = |key, modifiers| Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers };
        panel_frame(&mut app, &ctx, vec![Event::PointerMoved(at), press(true)], draw);
        panel_frame(&mut app, &ctx, vec![press(false)], draw);
        panel_frame(&mut app, &ctx, vec![key(egui::Key::A, egui::Modifiers::COMMAND)], draw);
        panel_frame(&mut app, &ctx, vec![Event::Text("250".into())], draw);
        panel_frame(&mut app, &ctx, vec![key(egui::Key::Enter, egui::Modifiers::NONE)], draw);
        panel_frame(&mut app, &ctx, vec![], draw);
        let mut want = before.clone();
        for (i, shift) in [(0, 25.0), (2, -25.0)] {
            want[i].2 = 250.0;
            if centered {
                want[i].0 -= shift;
            }
        }
        assert_eq!(boards(&app), want, "centered: {centered}");
        assert_eq!(app.session.doc().unwrap().history.undo.len(), undo + 1, "one undo step");
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(boards(&app), before);
    }
}

/// #530: Edit Artboards shows the active artboard, with New Artboard, Delete Artboard and Exit.
/// #775: type's Paragraph section has Hyphenate, and its checkbox turns it on.
#[test]
fn type_paragraph_section_hyphenates() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    app.run("text.create", json!({"x": 10, "y": 50, "text": "Paragraph"})).unwrap();
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    let texts = frame(&mut app, &ctx, vec![]);
    click(&mut app, &ctx, &texts, "Hyphenate");
    let hyphenate = |app: &VectorcraftApp| crate::panels::character::text_style(app).unwrap().1.hyphenate;
    assert!(hyphenate(&app));
}

/// Variable fonts get the same Variable Font button in Properties as in the Character panel.
#[test]
fn properties_variable_font_button_for_variable_type() {
    use vectorcraft_text::test_fonts::{TWO_AXIS_FAMILY, two_axis_font};
    vectorcraft_text::FontDb::global().add_font(two_axis_font().unwrap());
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
    let plain = app.run("text.create", json!({"x": 20, "y": 50, "text": "Static"})).unwrap()["id"].clone();
    let id = app.run("text.create", json!({"x": 20, "y": 120, "text": "lIn", "font": TWO_AXIS_FAMILY})).unwrap()["id"].clone();
    let ctx = egui::Context::default();
    app.run("select.set", json!({"ids": [plain]})).unwrap();
    assert!(!frame(&mut app, &ctx, vec![]).iter().any(|(t, _)| t == "Variable Font"));
    app.run("select.set", json!({"ids": [id]})).unwrap();
    assert!(frame(&mut app, &ctx, vec![]).iter().any(|(t, _)| t == "Variable Font"));
}

#[test]
fn edit_artboards_adds_artboards_and_exits() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    let ctx = egui::Context::default();
    let texts = frame(&mut app, &ctx, vec![]);
    click(&mut app, &ctx, &texts, "Edit Artboards");
    assert_eq!(app.session.tool_id(), "artboard");
    let texts = frame(&mut app, &ctx, vec![]);
    assert!(texts.iter().any(|(t, _)| t == "Artboard 1") && texts.iter().any(|(t, _)| t == "200 pt"), "{texts:?}");
    click(&mut app, &ctx, &texts, "New Artboard");
    let boards = |app: &VectorcraftApp| app.session.active().unwrap().doc.artboards.len();
    assert_eq!(boards(&app), 2);
    let texts = frame(&mut app, &ctx, vec![]);
    assert!(texts.iter().any(|(t, _)| t == "Artboard 2"), "the new artboard is the active one: {texts:?}");
    click(&mut app, &ctx, &texts, "Delete Artboard");
    assert_eq!(boards(&app), 1);
    let texts = frame(&mut app, &ctx, vec![]);
    click(&mut app, &ctx, &texts, "Exit");
    assert_eq!(app.session.tool_id(), "selection");
}

/// Typing a name into Edit Artboards' name field and pressing Enter renames the active artboard
/// alone, trimmed as the Artboards panel's rename trims it, with one command and one undo step;
/// a click elsewhere in the panel renames it too. Pressing Escape or leaving the field empty
/// keeps the name and runs no command.
#[test]
fn edit_artboards_renames_the_active_artboard() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 100, "artboards": 3})).unwrap();
    app.select_tool("artboard");
    super::super::artboards::select(&mut app, 1);
    app.session.doc_mut().unwrap().mark_saved();
    let before = app.session.doc().unwrap().doc.artboards.clone();
    let undo = app.session.doc().unwrap().history.undo.len();
    let ctx = egui::Context::default();
    let key = |key, modifiers| vec![Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }];
    // Click the field showing Artboard 2's name, select all of it, type `typed`, then send each
    // frame's events of `end` → the number of commands run.
    let rename = |app: &mut VectorcraftApp, typed: Vec<Event>, end: Vec<Vec<Event>>| {
        frame(app, &ctx, vec![]);
        let texts = frame(app, &ctx, vec![]);
        let name = app.session.doc().unwrap().doc.artboards[1].name.clone();
        click(app, &ctx, &texts, &name);
        frame(app, &ctx, key(egui::Key::A, egui::Modifiers::COMMAND));
        let commands = app.run_count;
        frame(app, &ctx, typed);
        for events in end {
            frame(app, &ctx, events);
        }
        frame(app, &ctx, vec![]);
        app.run_count.wrapping_sub(commands)
    };
    let enter = || vec![key(egui::Key::Enter, egui::Modifiers::NONE)];
    let text = |s: &str| vec![Event::Text(s.into())];
    assert_eq!(rename(&mut app, text("Cover"), vec![key(egui::Key::Escape, egui::Modifiers::NONE)]), 0, "Escape");
    assert_eq!(rename(&mut app, key(egui::Key::Backspace, egui::Modifiers::NONE), enter()), 0, "an empty field");
    assert_eq!(app.session.doc().unwrap().doc.artboards, before);
    assert_eq!(rename(&mut app, text(" Cover "), enter()), 1);
    let mut renamed = before.clone();
    renamed[1].name = "Cover".into();
    let st = app.session.doc().unwrap();
    assert_eq!(st.doc.artboards, renamed, "Artboard 2 alone is renamed");
    assert_eq!(st.history.undo.len(), undo + 1, "one undo step");
    app.run("edit.undo", json!({})).unwrap();
    let st = app.session.doc().unwrap();
    assert_eq!(st.doc.artboards, before);
    assert!(!st.is_dirty());
    // The click elsewhere is a press and release on the panel's "Artboard" heading, outside
    // the field.
    let at = frame(&mut app, &ctx, vec![]).iter().find(|(t, _)| t == "Artboard").map(|(_, r)| r.center()).unwrap();
    let b = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() };
    assert_eq!(rename(&mut app, text("Back"), vec![vec![Event::PointerMoved(at), b(true)], vec![b(false)]]), 1, "a click elsewhere");
    renamed[1].name = "Back".into();
    assert_eq!(app.session.doc().unwrap().doc.artboards, renamed);
}

/// A click on Edit Artboards' Artboard Options icon opens Artboard Options for the active
/// artboard, as double-clicking the Artboard tool's button does (`tool.options {tool: "artboard"}`).
#[test]
fn edit_artboards_opens_the_active_artboards_options() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 100, "artboards": 3})).unwrap();
    app.select_tool("artboard");
    super::super::artboards::select(&mut app, 1);
    app.run("tool.options", json!({"tool": "artboard"})).unwrap();
    let want = app.ui.dialog.take().unwrap().fields;
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    let texts = frame(&mut app, &ctx, vec![]);
    // The icon is the rightmost clickable widget on the name's line.
    let name = texts.iter().find(|(t, _)| t == "Artboard 2").map(|(_, r)| *r).unwrap();
    let icon = ctx.viewport(|vp| {
        vp.prev_pass
            .widgets
            .layers()
            .flat_map(|(_, w)| w.iter())
            .filter(|w| w.sense.senses_click() && w.rect.y_range().contains(name.center().y))
            .map(|w| w.rect)
            .max_by(|a, b| a.left().total_cmp(&b.left()))
            .unwrap()
            .center()
    });
    let commands = app.run_count;
    let press = |pressed| Event::PointerButton { pos: icon, button: PointerButton::Primary, pressed, modifiers: Default::default() };
    frame(&mut app, &ctx, vec![Event::PointerMoved(icon), press(true)]);
    frame(&mut app, &ctx, vec![press(false)]);
    assert_eq!(app.run_count, commands.wrapping_add(1), "one command");
    let d = app.ui.dialog.take().expect("Artboard Options");
    assert_eq!((d.kind.as_str(), d.f64("index", 9.0), d.str("name").as_str()), (crate::dialogs::artboard_options::KIND, 1.0, "Artboard 2"));
    assert_eq!(d.fields, want, "the fields the tool's double-click shows");
    assert_eq!(app.session.tool_id(), "artboard");
}

/// Edit Artboards' Fill (Properties › Artboards › Fill): the list sets the active artboard's
/// background color to Black and back to None, one command and one undo step each, and shows the
/// choice. Color Picker… in the list and a click on the chip open the Color Picker for that
/// artboard.
#[test]
fn edit_artboards_fill_sets_the_background_color() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 100, "artboards": 2})).unwrap();
    app.select_tool("artboard");
    super::super::artboards::select(&mut app, 1);
    let colors = |app: &VectorcraftApp| app.session.doc().unwrap().doc.artboards.iter().map(|a| a.color).collect::<Vec<_>>();
    let undo = |app: &VectorcraftApp| app.session.doc().unwrap().history.undo.len();
    let ctx = egui::Context::default();
    // Open the list showing `shown` and choose `option` → the number of commands run.
    let choose = |app: &mut VectorcraftApp, shown: &str, option: &str| {
        frame(app, &ctx, vec![]);
        let texts = frame(app, &ctx, vec![]);
        click(app, &ctx, &texts, shown);
        let texts = frame(app, &ctx, vec![]);
        let commands = app.run_count;
        click(app, &ctx, &texts, option);
        app.run_count.wrapping_sub(commands)
    };
    let before = undo(&app);
    assert_eq!(choose(&mut app, "None", "Black"), 1, "one command");
    assert_eq!(colors(&app), [None, Some(Color::BLACK)]);
    assert_eq!(undo(&app), before + 1, "one undo step");
    assert_eq!(choose(&mut app, "Black", "None"), 1);
    assert_eq!(colors(&app), [None, None]);
    choose(&mut app, "None", "Color Picker…");
    let d = app.ui.dialog.take().expect("the Color Picker");
    assert_eq!((d.kind.as_str(), d.fields.get("artboard")), ("colorPicker", Some(&json!(1))));
    // The chip is the clickable widget left of the list on the Fill: row.
    let texts = frame(&mut app, &ctx, vec![]);
    let row = texts.iter().find(|(t, _)| t == "Fill:").map(|(_, r)| *r).unwrap();
    let chip = ctx.viewport(|vp| {
        vp.prev_pass
            .widgets
            .layers()
            .flat_map(|(_, w)| w.iter())
            .filter(|w| w.sense.senses_click() && w.rect.y_range().contains(row.center().y) && w.rect.left() > row.right())
            .map(|w| w.rect)
            .min_by(|a, b| a.left().total_cmp(&b.left()))
            .unwrap()
            .center()
    });
    let press = |pressed| Event::PointerButton { pos: chip, button: PointerButton::Primary, pressed, modifiers: Default::default() };
    frame(&mut app, &ctx, vec![Event::PointerMoved(chip), press(true)]);
    frame(&mut app, &ctx, vec![press(false)]);
    assert_eq!(app.ui.dialog.as_ref().map(|d| d.fields.get("artboard")), Some(Some(&json!(1))), "the chip opens it too");
}

fn artboard_deletion_app(count: usize) -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 100, "artboards": count})).unwrap();
    for i in 0..count {
        app.run("shape.rectangle", json!({"x": i * 250 + 10, "y": 10, "width": 30, "height": 20})).unwrap();
    }
    app.select_tool("artboard");
    super::super::artboards::select(&mut app, 0);
    app
}

fn artboard_deletion_click(
    app: &mut VectorcraftApp,
    ctx: &egui::Context,
    draw: fn(&mut VectorcraftApp, &mut Ui),
    label: Option<&str>,
    modifiers: egui::Modifiers,
) {
    let at = if let Some(label) = label {
        panel_frame(app, ctx, vec![], draw);
        let texts = panel_frame(app, ctx, vec![], draw);
        texts.iter().find(|(t, _)| t.trim() == label).map(|(_, r)| r.center()).unwrap_or_else(|| panic!("no {label:?} in {texts:?}"))
    } else {
        // The trash is the last 24-point footer button. Use the drawn panel bounds, as the Asset Export bin test uses its footer.
        let mut bounds = Rect::NOTHING;
        for _ in 0..2 {
            let raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(300.0, 700.0))), ..Default::default() };
            let mut out = ctx.run_ui(raw, |ui| {
                draw(app, ui);
                bounds = ui.min_rect();
            });
            out.textures_delta.clear();
        }
        assert!(bounds.width() >= 24.0 && bounds.height() >= 24.0);
        bounds.right_bottom() - vec2(12.0, 12.0)
    };
    let button = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers };
    panel_frame(app, ctx, vec![Event::ModifiersChanged(modifiers), Event::PointerMoved(at), button(true)], draw);
    panel_frame(app, ctx, vec![button(false)], draw);
    panel_frame(app, ctx, vec![Event::ModifiersChanged(egui::Modifiers::NONE)], draw);
}

fn select_artboard_rows(app: &mut VectorcraftApp, ctx: &egui::Context, indices: &[usize]) {
    for (at, i) in indices.iter().enumerate() {
        let modifiers = if at == 0 { egui::Modifiers::NONE } else { egui::Modifiers::COMMAND };
        artboard_deletion_click(app, ctx, super::super::artboards::show, Some(&format!("Artboard {}", i + 1)), modifiers);
    }
    let count = app.session.doc().unwrap().doc.artboards.len();
    assert_eq!(super::super::artboards::picked(app, count), indices, "the rows are selected before deletion");
}

fn assert_artboard_deletion(draw: fn(&mut VectorcraftApp, &mut Ui), label: Option<&str>) {
    for indices in [vec![0, 2], vec![2], vec![3], vec![2, 3], vec![0, 1, 2, 3]] {
        let mut app = artboard_deletion_app(4);
        let ctx = egui::Context::default();
        select_artboard_rows(&mut app, &ctx, &indices);
        app.session.doc_mut().unwrap().mark_saved();
        let before = app.session.doc().unwrap().doc.clone();
        let undo = app.session.doc().unwrap().history.undo.len();
        let all_selected = indices == [0, 1, 2, 3];
        let expected: Vec<_> = if all_selected {
            assert_eq!(app.view().unwrap().artboard, 3, "the active last board must not replace the first-board survivor oracle");
            vec![&before.artboards[0]]
        } else {
            before.artboards.iter().enumerate().filter(|(i, _)| !indices.contains(i)).map(|(_, a)| a).collect()
        };
        let expected = serde_json::to_value(expected).unwrap();
        let artwork = serde_json::to_value(&before.layers).unwrap();

        let commands = app.run_count;
        artboard_deletion_click(&mut app, &ctx, draw, label, egui::Modifiers::NONE);
        let st = app.session.doc().unwrap();
        assert_eq!(
            serde_json::to_value(&st.doc.artboards).unwrap(),
            expected,
            "selected deletion keeps the expected IDs; all-selected keeps original board 1"
        );
        assert_eq!(app.run_count, commands.wrapping_add(1), "one command from the actual deletion control");
        assert_eq!(serde_json::to_value(&st.doc.layers).unwrap(), artwork, "artwork IDs, geometry and appearance stay intact");
        assert_eq!(st.history.undo.len(), undo + 1, "one deletion is one undo step");
        assert!(st.is_dirty());
        let active = app.view().unwrap().artboard;
        if all_selected {
            assert_eq!(active, 0);
        }
        assert!(active < st.doc.artboards.len(), "the active artboard remains in range");
        assert_eq!(app.session.tool_options()["active"], json!(active));
        assert_eq!(super::super::artboards::picked(&app, st.doc.artboards.len()), [active]);
        assert_eq!(app.session.tool_options()["selected"], json!([active]), "raw tool selection contains only the remaining active board");

        app.run("edit.undo", json!({})).unwrap();
        let st = app.session.doc().unwrap();
        assert_eq!(serde_json::to_value(&st.doc.artboards).unwrap(), serde_json::to_value(&before.artboards).unwrap());
        assert_eq!(serde_json::to_value(&st.doc.layers).unwrap(), artwork);
        assert_eq!(st.history.undo.len(), undo);
        assert!(!st.is_dirty(), "one undo returns to the saved document");
        app.run("edit.redo", json!({})).unwrap();
        let st = app.session.doc().unwrap();
        assert_eq!(serde_json::to_value(&st.doc.artboards).unwrap(), expected, "redo removes the same artboard IDs");
        assert_eq!(serde_json::to_value(&st.doc.layers).unwrap(), artwork);
        assert_eq!(st.history.undo.len(), undo + 1);
        assert!(st.is_dirty());
    }
}

fn assert_artboard_deletion_guard(draw: fn(&mut VectorcraftApp, &mut Ui), label: Option<&str>) {
    let mut app = artboard_deletion_app(1);
    // Leave a redo entry to ensure a disabled action preserves both history branches.
    app.run("artboard.setProps", json!({"index": 0, "name": "Temporary"})).unwrap();
    app.run("edit.undo", json!({})).unwrap();
    let ctx = egui::Context::default();
    select_artboard_rows(&mut app, &ctx, &[0]);
    app.session.doc_mut().unwrap().mark_saved();
    let before = app.session.doc().unwrap().doc.clone();
    let st = app.session.doc().unwrap();
    let history = (st.history.undo.len(), st.history.redo.len());
    assert!(history.1 > 0);
    let active = app.view().unwrap().artboard;
    let options = app.session.tool_options();

    let commands = app.run_count;
    artboard_deletion_click(&mut app, &ctx, draw, label, egui::Modifiers::NONE);
    assert_eq!(app.run_count, commands, "the disabled last-board control dispatches no command");
    let st = app.session.doc().unwrap();
    assert_eq!(serde_json::to_value(&st.doc.artboards).unwrap(), serde_json::to_value(&before.artboards).unwrap());
    assert_eq!(serde_json::to_value(&st.doc.layers).unwrap(), serde_json::to_value(&before.layers).unwrap());
    assert_eq!((st.history.undo.len(), st.history.redo.len()), history);
    assert!(!st.is_dirty(), "deleting the last artboard is a no-op");
    assert_eq!(app.view().unwrap().artboard, active);
    assert_eq!(app.session.tool_options(), options);
}

#[test]
fn artboard_menu_deletes_selected_artboards_in_one_undo_step() {
    assert_artboard_deletion(super::super::artboards::menu, Some("Delete Artboards"));
}

#[test]
fn properties_deletes_selected_artboards_in_one_undo_step() {
    assert_artboard_deletion(show, Some("Delete Artboard"));
}

#[test]
fn artboard_menu_preserves_last_artboard() {
    assert_artboard_deletion_guard(super::super::artboards::menu, Some("Delete Artboards"));
}

#[test]
fn properties_preserves_last_artboard() {
    assert_artboard_deletion_guard(show, Some("Delete Artboard"));
}

#[test]
fn artboard_trash_deletes_selected_artboards_in_one_undo_step() {
    assert_artboard_deletion(super::super::artboards::show, None);
}

#[test]
fn artboard_trash_preserves_last_artboard() {
    assert_artboard_deletion_guard(super::super::artboards::show, None);
}

/// Click the actual panel footer's New button (24 points, with a 4-point gap before Trash).
fn artboard_history_new(app: &mut VectorcraftApp, ctx: &egui::Context) {
    let mut bounds = Rect::NOTHING;
    for _ in 0..2 {
        let raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(300.0, 700.0))), ..Default::default() };
        let mut out = ctx.run_ui(raw, |ui| {
            super::super::artboards::show(app, ui);
            bounds = ui.min_rect();
        });
        out.textures_delta.clear();
    }
    let count = app.session.doc().unwrap().doc.artboards.len();
    let commands = app.run_count;
    let at = bounds.right_bottom() - vec2(40.0, 12.0);
    let button = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
    panel_frame(app, ctx, vec![Event::PointerMoved(at), button(true)], super::super::artboards::show);
    panel_frame(app, ctx, vec![button(false)], super::super::artboards::show);
    assert_eq!(app.run_count, commands.wrapping_add(1), "the actual New button dispatches once");
    assert_eq!(app.session.doc().unwrap().doc.artboards.len(), count + 1);
    assert_eq!(app.session.tool_options()["active"], json!(count));
    assert_eq!(app.view().unwrap().artboard, count);
}

/// Keep the panel and its natural focus in the same context throughout Undo and deletion.
fn artboard_history_key(app: &mut VectorcraftApp, ctx: &egui::Context, key: egui::Key, modifiers: egui::Modifiers) {
    for pressed in [true, false] {
        let events = vec![Event::ModifiersChanged(modifiers), Event::Key { key, physical_key: None, pressed, repeat: false, modifiers }];
        let raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(300.0, 700.0))), events, ..Default::default() };
        let mut out = ctx.run_ui(raw, |ui| {
            crate::shortcuts::handle(app, ui.ctx());
            super::super::artboards::show(app, ui);
        });
        out.textures_delta.clear();
    }
}

fn artboard_history_delete_after_undo(key: Option<egui::Key>, duplicate: bool) {
    let mut app = artboard_deletion_app(2);
    app.session.set_tool_option("moveArt", &json!(false));
    app.session.set_tool_option("scaleArt", &json!(true));
    app.session.doc_mut().unwrap().mark_saved();
    let st = app.session.doc().unwrap();
    let before = st.doc.clone();
    let selection = st.selection.clone();
    assert!(!selection.is_empty(), "selected artwork catches a Clear fallthrough");
    let undo = st.history.undo.len();
    let ctx = egui::Context::default();
    if duplicate {
        let commands = app.run_count;
        artboard_deletion_click(&mut app, &ctx, super::super::artboards::menu, Some("Duplicate Artboards"), egui::Modifiers::NONE);
        assert_eq!(app.run_count, commands.wrapping_add(1));
        assert_eq!(app.session.doc().unwrap().doc.artboards.len(), 3);
        assert_eq!(app.session.tool_options()["active"], json!(2));
        assert_eq!(app.view().unwrap().artboard, 2);
    } else {
        artboard_history_new(&mut app, &ctx);
    }
    assert_eq!(app.session.doc().unwrap().history.undo.len(), undo + 1);
    artboard_history_key(&mut app, &ctx, egui::Key::Z, egui::Modifiers::COMMAND);
    let st = app.session.doc().unwrap();
    assert_eq!(st.doc, before, "the real Undo shortcut restores the original document");
    assert_eq!(st.selection, selection);
    assert_eq!((st.history.undo.len(), st.history.redo.len()), (undo, 1));
    assert!(!st.is_dirty());
    // Capture the state before another panel or pointer can repair it. Still exercise deletion
    // on RED: keyboard deletion used to do nothing; Properties already clamped its own target.
    let after_undo = (app.view().unwrap().artboard, app.session.tool_options());
    if let Some(key) = key {
        artboard_history_key(&mut app, &ctx, key, egui::Modifiers::NONE);
    } else {
        let texts = panel_frame(&mut app, &ctx, vec![], show);
        assert!(texts.iter().any(|(label, _)| label == &before.artboards[1].name));
        artboard_deletion_click(&mut app, &ctx, show, Some("Delete Artboard"), egui::Modifiers::NONE);
    }
    let st = app.session.doc().unwrap();
    assert_eq!(st.doc.artboards, vec![before.artboards[0].clone()], "deletion after Undo removes the shown second board");
    assert_eq!(st.doc.layers, before.layers, "ordinary artwork survives without geometry or style changes");
    assert_eq!(st.selection, selection, "deletion never falls through to Clear");
    assert_eq!(after_undo.0, 1, "Undo normalizes the navigator before another gesture");
    assert_eq!(after_undo.1, json!({"active": 1, "selected": [1], "moveArt": false, "scaleArt": true}));
    assert_eq!((st.history.undo.len(), st.history.redo.len()), (undo + 1, 0));
    assert_eq!(st.history.undo.last().unwrap().label, "Delete Artboard");
    assert!(st.is_dirty());
    assert_eq!(app.view().unwrap().artboard, 0);
    assert_eq!(app.session.tool_options(), json!({"active": 0, "selected": [0], "moveArt": false, "scaleArt": true}));
    let deleted = st.doc.clone();

    artboard_history_key(&mut app, &ctx, egui::Key::Z, egui::Modifiers::COMMAND);
    let st = app.session.doc().unwrap();
    assert_eq!(st.doc, before);
    assert_eq!(st.selection, selection);
    assert_eq!((st.history.undo.len(), st.history.redo.len()), (undo, 1));
    assert!(!st.is_dirty());
    // Valid current selection stays put; history does not restore an earlier artboard selection.
    assert_eq!(app.session.tool_options()["selected"], json!([0]));
    artboard_history_key(&mut app, &ctx, egui::Key::Z, egui::Modifiers::COMMAND | egui::Modifiers::SHIFT);
    let st = app.session.doc().unwrap();
    assert_eq!(st.doc, deleted);
    assert_eq!(st.selection, selection);
    assert_eq!((st.history.undo.len(), st.history.redo.len()), (undo + 1, 0));
    assert!(st.is_dirty());
}

#[test]
fn artboard_history_new_undo_delete_uses_remaining_board() {
    artboard_history_delete_after_undo(Some(egui::Key::Delete), false);
}

#[test]
fn artboard_history_new_undo_backspace_uses_remaining_board() {
    artboard_history_delete_after_undo(Some(egui::Key::Backspace), false);
}

#[test]
fn artboard_history_new_undo_properties_agrees_with_tool() {
    artboard_history_delete_after_undo(None, false);
}

#[test]
fn artboard_history_duplicate_undo_delete_uses_remaining_board() {
    artboard_history_delete_after_undo(Some(egui::Key::Delete), true);
}

#[test]
fn artboard_history_undo_prunes_only_removed_indices() {
    for order in [[0, 1, 2], [0, 2, 1]] {
        let mut app = artboard_deletion_app(2);
        app.session.set_tool_option("moveArt", &json!(false));
        app.session.set_tool_option("scaleArt", &json!(true));
        app.session.doc_mut().unwrap().mark_saved();
        let st = app.session.doc().unwrap();
        let before = st.doc.clone();
        let selection = st.selection.clone();
        let undo = st.history.undo.len();
        let ctx = egui::Context::default();
        artboard_history_new(&mut app, &ctx);
        let created = app.session.doc().unwrap().doc.clone();
        for (at, i) in order.into_iter().enumerate() {
            let modifiers = if at == 0 { egui::Modifiers::NONE } else { egui::Modifiers::COMMAND };
            artboard_deletion_click(&mut app, &ctx, super::super::artboards::show, Some(&format!("Artboard {}", i + 1)), modifiers);
        }
        let options = app.session.tool_options();
        let expected = if order[2] == 2 { json!([0, 1]) } else { json!([1, 0]) };
        assert_eq!(options["selected"], if order[2] == 2 { json!([2, 0, 1]) } else { json!([1, 0, 2]) });
        artboard_history_key(&mut app, &ctx, egui::Key::Z, egui::Modifiers::COMMAND);
        let mut normalized = options;
        normalized["active"] = expected[0].clone();
        normalized["selected"] = expected;
        assert_eq!(app.session.tool_options(), normalized, "only the removed index is pruned, preserving surviving order and options");
        assert_eq!(json!(app.view().unwrap().artboard), normalized["active"]);
        let st = app.session.doc().unwrap();
        assert_eq!(st.doc, before);
        assert_eq!(st.selection, selection);
        assert_eq!((st.history.undo.len(), st.history.redo.len()), (undo, 1));
        assert!(!st.is_dirty());
        artboard_history_key(&mut app, &ctx, egui::Key::Z, egui::Modifiers::COMMAND | egui::Modifiers::SHIFT);
        assert_eq!(app.session.doc().unwrap().doc, created);
        assert_eq!(app.session.doc().unwrap().selection, selection);
        assert_eq!(app.session.tool_options(), normalized, "Redo does not replace an already valid multiselection");
    }
}

#[test]
fn artboard_history_valid_multiselection_survives_undo_redo() {
    let mut app = artboard_deletion_app(3);
    app.session.set_tool_option("moveArt", &json!(false));
    app.session.set_tool_option("scaleArt", &json!(true));
    let ctx = egui::Context::default();
    select_artboard_rows(&mut app, &ctx, &[0, 1]);
    let options = app.session.tool_options();
    assert_eq!(options["selected"], json!([1, 0]));
    let view = app.view().unwrap().artboard;
    app.session.doc_mut().unwrap().mark_saved();
    let st = app.session.doc().unwrap();
    let before = st.doc.clone();
    let selection = st.selection.clone();
    let undo = st.history.undo.len();
    app.run("object.move", json!({"dx": 7, "dy": 9})).unwrap();
    let moved = app.session.doc().unwrap().doc.clone();
    assert_ne!(moved, before);
    artboard_history_key(&mut app, &ctx, egui::Key::Z, egui::Modifiers::COMMAND);
    app.sync_views();
    let st = app.session.doc().unwrap();
    assert_eq!(st.doc, before);
    assert_eq!(st.selection, selection);
    assert_eq!((st.history.undo.len(), st.history.redo.len()), (undo, 1));
    assert!(!st.is_dirty());
    assert_eq!(app.session.tool_options(), options);
    assert_eq!(app.view().unwrap().artboard, view);
    artboard_history_key(&mut app, &ctx, egui::Key::Z, egui::Modifiers::COMMAND | egui::Modifiers::SHIFT);
    app.sync_views();
    let st = app.session.doc().unwrap();
    assert_eq!(st.doc, moved);
    assert_eq!(st.selection, selection);
    assert_eq!((st.history.undo.len(), st.history.redo.len()), (undo + 1, 0));
    assert!(st.is_dirty());
    assert_eq!(app.session.tool_options(), options);
    assert_eq!(app.view().unwrap().artboard, view);
}

/// #681: Rearrange All opens Rearrange All Artboards in one click, from the Document section
/// (only with several artboards) and from Edit Artboards' Quick Actions.
#[test]
fn rearrange_all_opens_the_dialog() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    let ctx = egui::Context::default();
    assert!(!frame(&mut app, &ctx, vec![]).iter().any(|(t, _)| t == "Rearrange All"), "one artboard: nothing to rearrange");
    app.run("artboard.new", json!({})).unwrap();
    let texts = frame(&mut app, &ctx, vec![]);
    click(&mut app, &ctx, &texts, "Rearrange All");
    let kind = |app: &VectorcraftApp| app.ui.dialog.as_ref().map(|d| d.kind.clone());
    assert_eq!(kind(&app).as_deref(), Some(crate::dialogs::rearrange_artboards::KIND));
    app.ui.dialog = None;
    app.select_tool("artboard");
    let texts = frame(&mut app, &ctx, vec![]);
    click(&mut app, &ctx, &texts, "Rearrange All");
    assert_eq!(kind(&app).as_deref(), Some(crate::dialogs::rearrange_artboards::KIND));
}

/// #671: Edit Artboards shows the artboard's preset (Custom when it matches none) and its
/// orientation; the other orientation swaps its width and height, its corner staying put.
#[test]
fn edit_artboards_shows_the_preset_and_flips_the_orientation() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"preset": "Letter"})).unwrap();
    app.run("artboard.setProps", json!({"index": 0, "x": 10, "y": 20})).unwrap();
    app.select_tool("artboard");
    let ctx = egui::Context::default();
    let texts = frame(&mut app, &ctx, vec![]);
    assert!(texts.iter().any(|(t, _)| t == "Preset:") && texts.iter().any(|(t, _)| t == "Letter"), "{texts:?}");
    // The row's clickable widgets, left to right: the preset combo, Portrait, Landscape.
    let row = texts.iter().find(|(t, _)| t == "Preset:").map(|(_, r)| *r).unwrap();
    let buttons: Vec<Rect> = ctx.viewport(|vp| {
        let mut r: Vec<Rect> = vp
            .prev_pass
            .widgets
            .layers()
            .flat_map(|(_, w)| w.iter())
            .filter(|w| w.sense.senses_click() && w.rect.y_range().contains(row.center().y) && w.rect.left() > row.right())
            .map(|w| w.rect)
            .collect();
        r.sort_by(|a, b| a.left().total_cmp(&b.left()));
        r
    });
    let landscape = buttons.last().unwrap().center();
    let press = |pressed| Event::PointerButton { pos: landscape, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    frame(&mut app, &ctx, vec![Event::PointerMoved(landscape), press(true)]);
    frame(&mut app, &ctx, vec![press(false)]);
    let rect = app.session.active().unwrap().doc.artboards[0].rect;
    assert_eq!((rect.x0, rect.y0, rect.width(), rect.height()), (10.0, 20.0, 792.0, 612.0), "landscape, same corner");
    let texts = frame(&mut app, &ctx, vec![]);
    assert!(texts.iter().any(|(t, _)| t == "Letter"), "still Letter, turned: {texts:?}");
    app.run("artboard.setProps", json!({"index": 0, "width": 333})).unwrap();
    assert!(frame(&mut app, &ctx, vec![]).iter().any(|(t, _)| t == "Custom"));
}

/// The center of Edit Artboards' Portrait (`landscape` false) or Landscape button.
fn orientation_button(ctx: &egui::Context, texts: &[(String, Rect)], landscape: bool) -> Pos2 {
    let row = texts.iter().find(|(t, _)| t == "Preset:").map(|(_, r)| *r).unwrap();
    ctx.viewport(|vp| {
        let mut r: Vec<Rect> = vp
            .prev_pass
            .widgets
            .layers()
            .flat_map(|(_, w)| w.iter())
            .filter(|w| w.sense.senses_click() && w.rect.y_range().contains(row.center().y) && w.rect.left() > row.right())
            .map(|w| w.rect)
            .collect();
        r.sort_by(|a, b| a.left().total_cmp(&b.left()));
        r[r.len() - if landscape { 1 } else { 2 }].center()
    })
}

/// #1482: with Artboards 1 and 3 selected, Portrait swaps the width and height of both in one
/// undo step, each keeping its top-left corner; Artboard 2 stays landscape. An artboard already
/// in that orientation keeps its size.
#[test]
fn edit_artboards_orientation_turns_every_selected_artboard() {
    let mut app = two_selected_artboards();
    let before = boards(&app);
    let undo = app.session.doc().unwrap().history.undo.len();
    let ctx = egui::Context::default();
    let portrait = |app: &mut VectorcraftApp| {
        frame(app, &ctx, vec![]);
        let texts = frame(app, &ctx, vec![]);
        let at = orientation_button(&ctx, &texts, false);
        let press = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() };
        frame(app, &ctx, vec![Event::PointerMoved(at), press(true)]);
        frame(app, &ctx, vec![press(false)]);
    };
    portrait(&mut app);
    let mut want = before.clone();
    for i in [0, 2] {
        (want[i].2, want[i].3) = (before[i].3, before[i].2);
    }
    assert_eq!(boards(&app), want);
    assert_eq!(app.session.doc().unwrap().history.undo.len(), undo + 1, "one undo step");
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(boards(&app), before);
    // Artboard 3 portrait already: Portrait turns Artboard 1 alone.
    app.run("artboard.setProps", json!({"index": 2, "width": 150, "height": 300})).unwrap();
    let before = boards(&app);
    portrait(&mut app);
    let mut want = before.clone();
    (want[0].2, want[0].3) = (100.0, 200.0);
    assert_eq!(boards(&app), want);
}

/// #1482: with Artboards 1 and 3 selected, a preset chosen in Edit Artboards sizes both in one
/// undo step, each in its own orientation and keeping its top-left corner: Letter (612 × 792 pt)
/// stays portrait for Artboard 1 and turns landscape for Artboard 3. Artboard 2 keeps its size.
#[test]
fn edit_artboards_preset_sizes_every_selected_artboard() {
    let mut app = two_selected_artboards();
    app.run("artboard.setProps", json!({"index": 0, "width": 100, "height": 200})).unwrap();
    let before = boards(&app);
    let undo = app.session.doc().unwrap().history.undo.len();
    let ctx = egui::Context::default();
    // A frame on a wide screen, with the texts of nested shapes too (the open list and its
    // submenu), and the center of `label` among them (menu items carry a check-mark column).
    let run = |app: &mut VectorcraftApp, events: Vec<Event>| {
        fn walk(s: &egui::Shape, v: &mut Vec<(String, Rect)>) {
            match s {
                egui::Shape::Text(t) => v.push((t.galley.text().to_string(), Rect::from_min_size(t.pos, t.galley.size()))),
                egui::Shape::Vec(s) => s.iter().for_each(|s| walk(s, v)),
                _ => {}
            }
        }
        let raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(800.0, 700.0))), events, ..Default::default() };
        let mut out = ctx.run_ui(raw, |ui| show(app, ui));
        out.textures_delta.clear();
        let mut v = vec![];
        out.shapes.iter().for_each(|c| walk(&c.shape, &mut v));
        v
    };
    let at = |texts: &[(String, Rect)], label: &str| {
        texts.iter().find(|(t, _)| t.trim() == label).map(|(_, r)| r.center()).unwrap_or_else(|| panic!("no {label:?} in {texts:?}"))
    };
    let press = |pos, pressed| Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Default::default() };
    run(&mut app, vec![]);
    let custom = at(&run(&mut app, vec![]), "Custom");
    run(&mut app, vec![Event::PointerMoved(custom), press(custom, true)]);
    run(&mut app, vec![press(custom, false)]);
    // Hovering Print opens its submenu.
    let print = at(&run(&mut app, vec![]), "Print");
    run(&mut app, vec![Event::PointerMoved(print)]);
    let letter = at(&run(&mut app, vec![]), "Letter");
    run(&mut app, vec![Event::PointerMoved(letter), press(letter, true)]);
    run(&mut app, vec![press(letter, false)]);
    let mut want = before.clone();
    (want[0].2, want[0].3) = (612.0, 792.0);
    (want[2].2, want[2].3) = (792.0, 612.0);
    assert_eq!(boards(&app), want);
    assert_eq!(app.session.doc().unwrap().history.undo.len(), undo + 1, "one undo step");
}

/// #602: Edit Artboards shows Move and Scale Artwork with Artboard; with Scale on, a new size
/// (here the other orientation) takes the art on the artboard along.
#[test]
fn edit_artboards_scales_the_art_with_scale_artwork_with_artboard() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    let r = app.run("shape.rectangle", json!({"x": 20, "y": 20, "width": 40, "height": 40})).unwrap()["id"].as_u64().unwrap();
    app.select_tool("artboard");
    let ctx = egui::Context::default();
    let texts = frame(&mut app, &ctx, vec![]);
    assert!(texts.iter().any(|(t, _)| t == "Move Artwork with Artboard"), "{texts:?}");
    assert_eq!(app.session.tool_options()["scaleArt"], false);
    click(&mut app, &ctx, &texts, "Scale Artwork with Artboard");
    assert_eq!(app.session.tool_options()["scaleArt"], true);
    // Portrait: 100 × 200, the art scaled by ½ across and 2 down.
    let texts = frame(&mut app, &ctx, vec![]);
    let row = texts.iter().find(|(t, _)| t == "Preset:").map(|(_, r)| *r).unwrap();
    let portrait = ctx.viewport(|vp| {
        let mut r: Vec<Rect> = vp
            .prev_pass
            .widgets
            .layers()
            .flat_map(|(_, w)| w.iter())
            .filter(|w| w.sense.senses_click() && w.rect.y_range().contains(row.center().y) && w.rect.left() > row.right())
            .map(|w| w.rect)
            .collect();
        r.sort_by(|a, b| a.left().total_cmp(&b.left()));
        r[r.len() - 2].center()
    });
    let press = |pressed| Event::PointerButton { pos: portrait, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    frame(&mut app, &ctx, vec![Event::PointerMoved(portrait), press(true)]);
    frame(&mut app, &ctx, vec![press(false)]);
    let d = &app.session.active().unwrap().doc;
    let ab = d.artboards[0].rect;
    assert_eq!((ab.width(), ab.height()), (100.0, 200.0));
    let b = d.node(vectorcraft_doc::NodeId(r)).unwrap().geometric_bounds().unwrap();
    assert_eq!((b.x0, b.y0, b.width(), b.height()), (10.0, 40.0, 20.0, 80.0));
}

/// One frame of the panel 1600 points tall, so Quick Actions at its foot are laid out, with
/// `events` → the texts painted, with their rects.
fn tall(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<Event>) -> Vec<(String, Rect)> {
    let raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(300.0, 1600.0))), events, ..Default::default() };
    let mut out = ctx.run_ui(raw, |ui| show(app, ui));
    out.textures_delta.clear();
    out.shapes
        .iter()
        .filter_map(|s| match &s.shape {
            egui::Shape::Text(t) => Some((t.galley.text().to_string(), Rect::from_min_size(t.pos, t.galley.size()))),
            _ => None,
        })
        .collect()
}

/// Clicks `at` in the tall panel over two frames (press, release), then lets a popup it opens
/// lay out → the texts of the frame after.
fn press(app: &mut VectorcraftApp, ctx: &egui::Context, at: Pos2) -> Vec<(String, Rect)> {
    let b = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() };
    tall(app, ctx, vec![Event::PointerMoved(at), b(true)]);
    tall(app, ctx, vec![b(false)]);
    tall(app, ctx, vec![]);
    tall(app, ctx, vec![])
}

/// Where the tall panel painted `label`.
fn text_at(texts: &[(String, Rect)], label: &str) -> Rect {
    texts.iter().find(|(t, _)| t == label).map(|(_, r)| *r).unwrap_or_else(|| panic!("no {label:?} in {texts:?}"))
}

fn shows(texts: &[(String, Rect)], label: &str) -> bool {
    texts.iter().any(|(t, _)| t == label)
}

fn undo_depth(app: &VectorcraftApp) -> usize {
    app.session.doc().unwrap().history.undo.len()
}

fn new_app() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app
}

fn rect(app: &mut VectorcraftApp, x: f64, y: f64, w: f64, h: f64) -> serde_json::Value {
    app.run("shape.rectangle", json!({"x": x, "y": y, "width": w, "height": h})).unwrap()["id"].clone()
}

/// One blend selected with the Blend tool has Blend Options in Quick Actions, which opens the
/// Blend Options dialog on it; OK is one undo step. Not with another tool, nor for a blend
/// selected with another object.
#[test]
fn blend_options_is_a_quick_action_for_one_blend_with_the_blend_tool() {
    let mut app = new_app();
    let (a, b) = (rect(&mut app, 20.0, 20.0, 40.0, 40.0), rect(&mut app, 200.0, 20.0, 40.0, 40.0));
    app.run("select.set", json!({"ids": [a, b]})).unwrap();
    let blend = app.run("object.blend.make", json!({})).unwrap()["id"].clone();
    app.run("select.set", json!({"ids": [blend]})).unwrap();
    let ctx = egui::Context::default();
    assert!(!shows(&tall(&mut app, &ctx, vec![]), "Blend Options"), "not with the Selection tool");
    app.select_tool("blend");
    app.run("select.set", json!({"ids": [blend]})).unwrap();
    let texts = tall(&mut app, &ctx, vec![]);
    let depth = undo_depth(&app);
    press(&mut app, &ctx, text_at(&texts, "Blend Options").center());
    assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some(crate::dialogs::blend_options::KIND));
    assert_eq!(undo_depth(&app), depth, "opening the dialog changes nothing");
    let d = app.ui.dialog.as_mut().unwrap();
    d.fields.insert("spacing".into(), json!("steps"));
    d.fields.insert("steps".into(), json!(3));
    crate::dialogs::confirm(&mut app).unwrap();
    assert_eq!(undo_depth(&app), depth + 1, "OK is one undo step");
    let c = rect(&mut app, 20.0, 200.0, 40.0, 40.0);
    app.select_tool("blend");
    app.run("select.set", json!({"ids": [blend, c]})).unwrap();
    assert!(!shows(&tall(&mut app, &ctx, vec![]), "Blend Options"), "a blend and another object");
    app.run("select.set", json!({"ids": [c]})).unwrap();
    assert!(!shows(&tall(&mut app, &ctx, vec![]), "Blend Options"), "a rectangle");
}

/// A Live Paint group selected with a path has Merge Live Paint in Quick Actions, which adds
/// the path to the group in one undo step. Not for the group alone, nor for plain paths, nor
/// with type selected, which Live Paint refuses.
#[test]
fn merge_live_paint_is_a_quick_action_for_a_live_paint_group_and_more() {
    let mut app = new_app();
    let (a, b) = (rect(&mut app, 20.0, 20.0, 100.0, 60.0), rect(&mut app, 60.0, 40.0, 100.0, 60.0));
    app.run("select.set", json!({"ids": [a, b]})).unwrap();
    let group = app.run("livePaint.make", json!({})).unwrap()["id"].clone();
    let (c, d) = (rect(&mut app, 140.0, 30.0, 60.0, 60.0), rect(&mut app, 300.0, 200.0, 40.0, 40.0));
    let text = app.run("text.create", json!({"x": 50, "y": 250, "text": "Hi"})).unwrap()["id"].clone();
    let ctx = egui::Context::default();
    for (ids, why) in [
        (json!([group]), "the group alone"),
        (json!([c, d]), "two plain paths"),
        (json!([group, text]), "the group and type"),
        (json!([group, c, text]), "the group, a path and type"),
    ] {
        app.run("select.set", json!({"ids": ids})).unwrap();
        assert!(!shows(&tall(&mut app, &ctx, vec![]), "Merge Live Paint"), "{why}");
    }
    app.run("select.set", json!({"ids": [group, c]})).unwrap();
    let c_id = vectorcraft_doc::NodeId(c.as_u64().unwrap());
    let parent = app.session.active().unwrap().doc.parent_of(c_id);
    let texts = tall(&mut app, &ctx, vec![]);
    let depth = undo_depth(&app);
    press(&mut app, &ctx, text_at(&texts, "Merge Live Paint").center());
    assert_eq!(undo_depth(&app), depth + 1, "one undo step");
    let st = app.session.active().unwrap();
    assert_eq!(st.selection.objects.iter().map(|n| json!(n.0)).collect::<Vec<_>>(), std::slice::from_ref(&group));
    assert!(st.doc.node(c_id).is_none() || st.doc.parent_of(c_id) != parent, "the path is merged into the group");
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(app.session.active().unwrap().doc.parent_of(c_id), parent, "Undo takes it out again");
}

/// A stroke with a variable width shows its Width Profile in the Appearance section, as every
/// stroke does with the Width tool; choosing another profile is one undo step. A uniform stroke
/// shows none with other tools.
#[test]
fn the_width_profile_shows_for_a_variable_width_or_the_width_tool() {
    let mut app = new_app();
    let line = app.run("shape.line", json!({"x1": 20, "y1": 100, "x2": 300, "y2": 100})).unwrap()["id"].clone();
    app.run("select.set", json!({"ids": [line]})).unwrap();
    let ctx = egui::Context::default();
    assert!(!shows(&tall(&mut app, &ctx, vec![]), "Profile:"), "a uniform stroke");
    app.select_tool("width");
    app.run("select.set", json!({"ids": [line]})).unwrap();
    assert!(shows(&tall(&mut app, &ctx, vec![]), "Profile:"), "the Width tool");
    app.select_tool("selection");
    app.run("select.set", json!({"ids": [line]})).unwrap();
    app.run("stroke.set", json!({"profile": "lens"})).unwrap();
    let texts = tall(&mut app, &ctx, vec![]);
    // The list sits right of its label.
    let spacing = ctx.global_style().spacing.item_spacing.x;
    let texts = press(&mut app, &ctx, text_at(&texts, "Profile:").right_center() + vec2(spacing + 50.0, 0.0));
    let depth = undo_depth(&app);
    press(&mut app, &ctx, text_at(&texts, "Taper Start").center());
    assert_eq!(undo_depth(&app), depth + 1, "one undo step");
    let profile =
        app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(line.as_u64().unwrap())).unwrap().appearance.stroke().unwrap().profile.clone();
    assert_eq!(WidthProfile::id_of(profile.as_ref()), "taperStart");
}

/// With Direct Selection, a path whose corner is selected shows that corner's kind and radius;
/// each edit sets that corner alone in one undo step. Not with the Selection tool, nor with the
/// whole path selected; a live rectangle keeps its own Corner Radius row instead.
#[test]
fn direct_selected_corners_show_their_kind_and_radius() {
    let mut app = new_app();
    let star = app.run("shape.star", json!({"cx": 200, "cy": 150, "radius1": 80, "radius2": 40})).unwrap()["id"].as_u64().unwrap();
    let ctx = egui::Context::default();
    app.run("select.anchors", json!({"id": star, "anchors": [[0, 2]], "mode": "set"})).unwrap();
    assert!(!shows(&tall(&mut app, &ctx, vec![]), "Corner:"), "not with the Selection tool");
    app.select_tool("directSelection");
    app.run("select.anchors", json!({"id": star, "anchors": [[0, 2]], "mode": "set"})).unwrap();
    let texts = tall(&mut app, &ctx, vec![]);
    assert!(shows(&texts, "Corner:") && shows(&texts, "Corner Radius:"), "{texts:?}");
    let corners = |app: &VectorcraftApp| {
        let n = app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(star)).unwrap().clone();
        let lc = LiveCorners::of(&n).unwrap();
        (lc.radius(0), lc.radius(2), lc.kind(0), lc.kind(2))
    };
    // The radius: the field right of its label.
    let label = text_at(&texts, "Corner Radius:");
    let field = texts.iter().find(|(t, r)| t == "0 pt" && r.left() > label.right() && (r.center().y - label.center().y).abs() < 6.0).unwrap().1;
    press(&mut app, &ctx, field.center());
    let depth = undo_depth(&app);
    let key = |key, modifiers| Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers };
    tall(&mut app, &ctx, vec![key(egui::Key::A, egui::Modifiers::COMMAND)]);
    tall(&mut app, &ctx, vec![Event::Text("6".into())]);
    tall(&mut app, &ctx, vec![key(egui::Key::Enter, egui::Modifiers::NONE)]);
    let texts = tall(&mut app, &ctx, vec![]);
    assert_eq!(undo_depth(&app), depth + 1, "one undo step");
    let round = vectorcraft_geom::shapes::CornerKind::Round;
    assert_eq!(corners(&app), (0.0, 6.0, round, round));
    // The kind, from its list.
    let texts = press(&mut app, &ctx, text_at(&texts, "Round").center());
    press(&mut app, &ctx, text_at(&texts, "Chamfer").center());
    assert_eq!(undo_depth(&app), depth + 2, "one undo step");
    assert_eq!(corners(&app), (0.0, 6.0, round, vectorcraft_geom::shapes::CornerKind::Chamfer));
    // The whole path selected.
    app.run("select.set", json!({"ids": [star]})).unwrap();
    assert!(!shows(&tall(&mut app, &ctx, vec![]), "Corner:"), "no corner selected");
    // A live rectangle's corner: its own Corner Radius row, once.
    let r = rect(&mut app, 20.0, 20.0, 60.0, 40.0).as_u64().unwrap();
    app.run("select.anchors", json!({"id": r, "anchors": [[0, 1]], "mode": "set"})).unwrap();
    let texts = tall(&mut app, &ctx, vec![]);
    assert!(!shows(&texts, "Corner:"), "{texts:?}");
    assert_eq!(texts.iter().filter(|(t, _)| t == "Corner Radius:").count(), 1, "{texts:?}");
}

/// One effect on the one selected object shows its name and a Delete icon beside the fx button,
/// which removes it in one undo step, the object's own or its stroke's. With several effects the
/// icon opens the Appearance panel; without one, or with several objects selected, no name.
#[test]
fn one_effect_has_a_delete_icon_in_appearance() {
    let mut app = new_app();
    let id = rect(&mut app, 20.0, 20.0, 80.0, 60.0);
    let node = vectorcraft_doc::NodeId(id.as_u64().unwrap());
    app.run("select.set", json!({"ids": [id]})).unwrap();
    let ctx = egui::Context::default();
    let spacing = ctx.global_style().spacing.item_spacing.x;
    let effects = |app: &VectorcraftApp| {
        let n = app.session.active().unwrap().doc.node(node).unwrap().clone();
        n.appearance.effects.len() + n.appearance.items.iter().map(|i| i.effects().len()).sum::<usize>()
    };
    let name = super::super::appearance::shown_effect_label("stylize.dropShadow");
    assert!(!shows(&tall(&mut app, &ctx, vec![]), &name), "no effect");
    // The object's own effect.
    app.run("effect.apply", json!({"effect": "stylize.dropShadow"})).unwrap();
    let texts = tall(&mut app, &ctx, vec![]);
    let depth = undo_depth(&app);
    press(&mut app, &ctx, text_at(&texts, &name).right_center() + vec2(spacing + 11.0, 0.0));
    assert_eq!((effects(&app), undo_depth(&app)), (0, depth + 1), "removed in one undo step");
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(effects(&app), 1);
    // The stroke's effect.
    app.run("edit.undo", json!({})).unwrap();
    let stroke = app.session.active().unwrap().doc.node(node).unwrap().appearance.items.iter().position(|i| !i.is_fill()).unwrap();
    app.run("effect.apply", json!({"effect": "stylize.dropShadow", "item": stroke})).unwrap();
    let texts = tall(&mut app, &ctx, vec![]);
    let depth = undo_depth(&app);
    press(&mut app, &ctx, text_at(&texts, &name).right_center() + vec2(spacing + 11.0, 0.0));
    assert_eq!((effects(&app), undo_depth(&app)), (0, depth + 1), "the stroke's effect removed in one undo step");
    // Two effects: the icon opens the Appearance panel.
    app.run("effect.apply", json!({"effect": "stylize.dropShadow"})).unwrap();
    app.run("effect.apply", json!({"effect": "stylize.dropShadow", "item": stroke})).unwrap();
    let texts = tall(&mut app, &ctx, vec![]);
    assert!(!shows(&texts, &name), "{texts:?}");
    let depth = undo_depth(&app);
    press(&mut app, &ctx, text_at(&texts, "fx").center() + vec2(17.0 + spacing + 11.0, 0.0));
    assert_eq!(app.ui.open_panel.as_deref(), Some("appearance"));
    assert_eq!((effects(&app), undo_depth(&app)), (2, depth));
    // Two objects with one effect each: no name.
    app.run("edit.undo", json!({})).unwrap();
    let other = rect(&mut app, 200.0, 20.0, 40.0, 40.0);
    app.run("effect.apply", json!({"effect": "stylize.dropShadow"})).unwrap();
    app.run("select.set", json!({"ids": [id, other]})).unwrap();
    assert!(!shows(&tall(&mut app, &ctx, vec![]), &name), "two objects");
}
