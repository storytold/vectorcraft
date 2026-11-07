use super::*;

/// One headless frame of the panel (or the Align panel's menu): the texts drawn.
fn texts(app: &mut VectorcraftApp, draw: fn(&mut VectorcraftApp, &mut Ui)) -> Vec<String> {
    fn walk(s: &egui::Shape, out: &mut Vec<String>) {
        match s {
            egui::Shape::Text(t) => out.push(t.galley.text().to_string()),
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
            _ => {}
        }
    }
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| draw(app, ui));
    out.textures_delta.clear();
    let mut v = vec![];
    out.shapes.iter().for_each(|c| walk(&c.shape, &mut v));
    v
}

#[test]
fn use_preview_bounds_shows_the_visual_size_and_checks_the_align_flyout() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
    app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 100, "height": 50})).unwrap();
    app.run("stroke.set", json!({"weight": 10})).unwrap();
    assert!(texts(&mut app, show).iter().any(|t| t == "100 pt"));
    assert!(texts(&mut app, crate::panels::align::menu).iter().any(|t| t == "   Use Preview Bounds"));
    set_pref(&mut app, "usePreviewBounds", true);
    let shown = texts(&mut app, show);
    assert!(shown.iter().any(|t| t == "110 pt") && shown.iter().any(|t| t == "60 pt"), "{shown:?}");
    assert!(texts(&mut app, crate::panels::align::menu).iter().any(|t| t == "✓ Use Preview Bounds"));
}

#[test]
fn use_preview_bounds_shows_the_size_after_geometry_effects() {
    // A 100 × 50 rectangle without a stroke, with Offset Path 10: 120 × 70 as drawn.
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
    app.run("shape.rectangle", json!({"x": 50, "y": 50, "width": 100, "height": 50})).unwrap();
    app.run("paint.setStroke", json!({"none": true})).unwrap();
    app.run("effect.apply", json!({"effect": "path.offsetPath", "params": {"offset": 10}, "item": null})).unwrap();
    let shown = texts(&mut app, show);
    assert!(shown.iter().any(|t| t == "100 pt") && shown.iter().any(|t| t == "50 pt"), "{shown:?}");
    set_pref(&mut app, "usePreviewBounds", true);
    let shown = texts(&mut app, show);
    assert!(shown.iter().any(|t| t == "120 pt") && shown.iter().any(|t| t == "70 pt"), "{shown:?}");
}

#[test]
fn rotation_field_shows_the_angle_the_box_keeps() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
    app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 100, "height": 50})).unwrap();
    app.run("object.rotate", json!({"angle": 45, "absolute": true})).unwrap();
    let shown = texts(&mut app, show);
    // The angle stays, and W/H are the rectangle's own sides.
    assert!(shown.iter().any(|t| t == "45°"), "{shown:?}");
    assert!(shown.iter().any(|t| t == "100 pt") && shown.iter().any(|t| t == "50 pt"), "{shown:?}");
    app.run("object.resetBoundingBox", json!({})).unwrap();
    let shown = texts(&mut app, show);
    assert!(shown.iter().any(|t| t == "0°") && !shown.iter().any(|t| t == "100 pt"), "{shown:?}");
}

/// #812: a selected live polygon shows its Polygon Properties in the Transform panel and the
/// Properties panel's Transform section, and Make Sides Equal evens it out after a stretch.
#[test]
fn a_polygon_shows_its_polygon_properties() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
    app.run("shape.polygon", json!({"cx": 150, "cy": 150, "radius": 50, "sides": 6})).unwrap();
    for draw in [show as fn(&mut VectorcraftApp, &mut Ui), crate::panels::properties::transform_section] {
        let shown = texts(&mut app, draw);
        // A hexagon's side is as long as its radius.
        for t in ["Sides:", "6", "Angle:", "0°", "Corner:", "Round", "Corner Radius:", "Radius:", "Side Length:", "50 pt", "Make Sides Equal"] {
            assert!(shown.iter().any(|s| s == t), "{t} in {shown:?}");
        }
    }
    assert!(texts(&mut app, show).iter().any(|s| s == "Polygon Properties:"));
    app.run("object.scale", json!({"sx": 200, "sy": 100})).unwrap();
    let live = |app: &VectorcraftApp| match &first_selected(app).unwrap().kind {
        NodeKind::Path { live: Some(l), .. } => l.clone(),
        k => panic!("{k:?}"),
    };
    assert!(!live(&app).polygon_sides_equal());
    app.run("object.setLiveShape", json!({"makeSidesEqual": true})).unwrap();
    assert!(live(&app).polygon_sides_equal());
}

/// #3451: a live ellipse shows pie start/end angles and Invert Pie in the Transform panel and
/// the Properties panel's Transform section; typed angles update the shape.
#[test]
fn an_ellipse_shows_its_pie_properties() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
    app.run("shape.ellipse", json!({"x": 50, "y": 50, "width": 120, "height": 80})).unwrap();
    for draw in [show as fn(&mut VectorcraftApp, &mut Ui), crate::panels::properties::transform_section] {
        let shown = texts(&mut app, draw);
        for t in ["Ellipse Properties:", "Pie Start Angle:", "0°", "Pie End Angle:", "360°", "Invert Pie"] {
            assert!(shown.iter().any(|s| s == t), "{t} in {shown:?}");
        }
    }
    app.run("object.setLiveShape", json!({"pieStart": 30, "pieEnd": 120})).unwrap();
    let shown = texts(&mut app, show);
    assert!(shown.iter().any(|s| s == "30°") && shown.iter().any(|s| s == "120°"), "{shown:?}");
}

#[test]
fn the_artboard_tool_shows_the_active_artboard() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
    app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 40, "height": 30})).unwrap();
    assert!(texts(&mut app, show).iter().any(|t| t == "40 pt"), "the selection first");
    app.run("tool.select", json!({"tool": "artboard"})).unwrap();
    let shown = texts(&mut app, show);
    // Centre reference point: X 150, Y 100, W 300, H 200; no rotate/shear for artboards.
    for v in ["150 pt", "100 pt", "300 pt", "200 pt"] {
        assert!(shown.iter().any(|t| t == v), "{v} in {shown:?}");
    }
    assert!(!shown.iter().any(|t| t == "40 pt" || t == "0°"), "{shown:?}");
}

/// #1597: with only ruler guides selected the panel shows their X or Y, and nothing of the art.
#[test]
fn selected_guides_show_their_position() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
    app.run("guide.add", json!({"vertical": true, "pos": 120})).unwrap();
    app.run("guide.add", json!({"vertical": false, "pos": 45})).unwrap();
    app.run("guide.select", json!({"indexes": [0]})).unwrap();
    assert_eq!(selected_guides(&app), Some((Some(120.0), None)));
    let shown = texts(&mut app, show);
    assert!(shown.iter().any(|t| t == "120 pt") && !shown.iter().any(|t| t == "W:" || t == "45 pt"), "{shown:?}");
    app.run("guide.select", json!({"indexes": [0, 1]})).unwrap();
    let shown = texts(&mut app, show);
    assert!(shown.iter().any(|t| t == "120 pt") && shown.iter().any(|t| t == "45 pt"), "{shown:?}");
    // The field's edit: the vertical guide moves, the horizontal one stays.
    app.run("guide.move", json!({"dx": 30})).unwrap();
    assert_eq!(selected_guides(&app), Some((Some(150.0), Some(45.0))));
    // Art selected with them: the art's box, as before.
    app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 40, "height": 30})).unwrap();
    assert_eq!(selected_guides(&app), None);
}

/// #1824: with an anchor selected with Direct Selection the panel shows that anchor's position,
/// and a new X moves it alone.
#[test]
fn an_anchor_selection_shows_and_moves_its_own_box() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
    let square = json!({"anchors": [{"x": 20, "y": 30}, {"x": 120, "y": 30}, {"x": 120, "y": 130}, {"x": 20, "y": 130}], "closed": true});
    let id = app.run("path.create", square).unwrap()["id"].as_u64().unwrap();
    let shown = texts(&mut app, show);
    assert!(shown.iter().any(|t| t == "70 pt") && shown.iter().any(|t| t == "100 pt"), "the whole path: {shown:?}");
    app.run("select.anchors", json!({"id": id, "anchors": [[0, 1]]})).unwrap();
    let shown = texts(&mut app, show);
    for v in ["120 pt", "30 pt", "0 pt"] {
        assert!(shown.iter().any(|t| t == v), "{v} in {shown:?}");
    }
    assert!(!shown.iter().any(|t| t == "70 pt" || t == "100 pt"), "{shown:?}");
    app.run("object.setBounds", json!({"x": 150, "reference": 4})).unwrap();
    let n = first_selected(&app).unwrap();
    let Some(path) = n.path_data() else { panic!("a path") };
    let xs: Vec<f64> = path.subpaths[0].anchors.iter().map(|a| a.p.x).collect();
    assert_eq!(xs, [20.0, 150.0, 120.0, 20.0]);
}

/// #2398: the panel menu's Flip Horizontal and Flip Vertical mirror the selection across the
/// reference point, as the panel's flip buttons do, not across its centre.
#[test]
fn menu_flips_keep_the_reference_point() {
    use crate::panels::tests_appearance::{click, frame_events};
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
    app.run("shape.rectangle", json!({"x": 20, "y": 30, "width": 100, "height": 50})).unwrap();
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    // Top-right reference point: (120, 30).
    ctx.data_mut(|d| d.insert_temp(egui::Id::unique("refpt"), 2usize));
    let flip = |app: &mut VectorcraftApp, item: &str| {
        let texts = frame_events(&ctx, app, vec![], menu);
        let r = texts.iter().find(|(t, _)| t.trim_start() == item).unwrap_or_else(|| panic!("{item} in {texts:?}")).1;
        click(&ctx, app, r.center(), menu);
        app.selection_box().unwrap().rect
    };
    assert_eq!(flip(&mut app, "Flip Horizontal"), Rect::new(120.0, 30.0, 220.0, 80.0));
    assert_eq!(flip(&mut app, "Flip Vertical"), Rect::new(120.0, -20.0, 220.0, 30.0));
}

/// Clicks the item of the panel menu labelled `item` (with or without its check mark).
fn menu_click(ctx: &egui::Context, app: &mut VectorcraftApp, item: &str) {
    use crate::panels::tests_appearance::{click, frame_events};
    let texts = frame_events(ctx, app, vec![], menu);
    let r = texts.iter().find(|(t, _)| t.trim_start_matches('✓').trim_start() == item).unwrap_or_else(|| panic!("{item} in {texts:?}")).1;
    click(ctx, app, r.center(), menu);
}

/// The panel menu's items as drawn, check marks included.
fn menu_texts(ctx: &egui::Context, app: &mut VectorcraftApp) -> Vec<String> {
    crate::panels::tests_appearance::frame_events(ctx, app, vec![], menu).into_iter().map(|(t, _)| t).collect()
}

/// Types `value` into the field of `draw` that shows `shown`, and presses Enter.
fn type_into(ctx: &egui::Context, app: &mut VectorcraftApp, draw: fn(&mut VectorcraftApp, &mut Ui), shown: &str, value: &str) {
    use crate::panels::tests_appearance::{click, frame_events, text_rect};
    let texts = frame_events(ctx, app, vec![], draw);
    click(ctx, app, text_rect(&texts, shown).center(), draw);
    let key = |key, modifiers| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers };
    frame_events(ctx, app, vec![key(egui::Key::A, egui::Modifiers::COMMAND)], draw);
    frame_events(ctx, app, vec![egui::Event::Text(value.into())], draw);
    frame_events(ctx, app, vec![key(egui::Key::Enter, egui::Modifiers::NONE)], draw);
    frame_events(ctx, app, vec![], draw);
}

/// The panel menu's Transform Object Only, Transform Pattern Only and Transform Both, one
/// checked: a new X typed in the panel (or the Properties panel's Transform section) moves the
/// pattern-filled rectangle and leaves its tiles, moves its tiles alone, or moves both. Until
/// one is chosen the panels do as General › Transform Pattern Tiles says, as they did before
/// the menu acted, and the menu checks Transform Both while it is on, Transform Object Only
/// while it is off; a choice overrides it, and resetting the preferences returns to it.
#[test]
fn the_menu_chooses_whether_the_object_its_pattern_or_both_transform() {
    let tiles = |app: &VectorcraftApp, id: u64| {
        let n = app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().clone();
        let vectorcraft_color::Paint::Pattern { xf, .. } = n.appearance.fill_paint() else { panic!("a pattern fill") };
        xf.as_coeffs()
    };
    let labels = ["Transform Object Only", "Transform Pattern Only", "Transform Both"];
    let modes = ["object", "pattern", "both"];
    let checked = |shown: &[String]| -> Vec<usize> { (0..3).filter(|i| shown.contains(&format!("✓ {}", labels[*i]))).collect() };
    for draw in [show as fn(&mut VectorcraftApp, &mut Ui), crate::panels::properties::transform_section] {
        for general in [false, true] {
            for pick in [None, Some(0), Some(1), Some(2)] {
                let case = format!("General › Transform Pattern Tiles {general}, menu choice {pick:?}");
                let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
                app.run("prefs.set", json!({"key": "transformPatternTiles", "value": general})).unwrap();
                app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
                app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
                app.run("object.pattern.make", json!({"name": "Dots", "width": 20, "height": 20})).unwrap();
                app.run("object.pattern.done", json!({})).unwrap();
                let id = app.run("shape.rectangle", json!({"x": 100, "y": 40, "width": 100, "height": 60})).unwrap()["id"].as_u64().unwrap();
                app.run("paint.setFill", json!({"ids": [id], "swatch": "Dots"})).unwrap();
                app.run("select.set", json!({"ids": [id]})).unwrap();
                let ctx = egui::Context::default();
                crate::theme::install_fonts(&ctx);
                let following = if general { 2 } else { 0 };
                assert_eq!(checked(&menu_texts(&ctx, &mut app)), [following], "{case}: before a choice");
                if let Some(item) = pick {
                    menu_click(&ctx, &mut app, labels[item]);
                }
                let item = pick.unwrap_or(following);
                assert_eq!(app.session.prefs.transform_panel_patterns, pick.map_or("default", |i| modes[i]), "{case}");
                assert_eq!(checked(&menu_texts(&ctx, &mut app)), [item], "{case}");
                // The centre reference point: X 150 becomes 170.
                type_into(&ctx, &mut app, draw, "150 pt", "170");
                let x0 = app.selection_box().unwrap().rect.x0;
                assert_eq!(x0, if item == 1 { 100.0 } else { 120.0 }, "{case}: the object");
                assert_eq!(tiles(&app, id), [1.0, 0.0, 0.0, 1.0, if item == 0 { 0.0 } else { 20.0 }, 0.0], "{case}: the tiles");
                // Reset (the General category): the menu follows General › Transform Pattern
                // Tiles again.
                app.run("prefs.reset", json!({"category": "General"})).unwrap();
                assert_eq!(app.session.prefs.transform_panel_patterns, "default", "{case}: reset");
                let following = if app.session.prefs.transform_pattern_tiles { 2 } else { 0 };
                assert_eq!(checked(&menu_texts(&ctx, &mut app)), [following], "{case}: reset");
            }
        }
    }
}

/// Transform Object Only, Transform Pattern Only and Transform Both act on the rotation field
/// of both panels and on the Transform panel's shear field as they do on X: the pattern-filled
/// rectangle turns or shears with its tiles left as they are, its tiles alone, or both.
#[test]
fn the_menu_choice_also_applies_to_rotation_and_shear() {
    let tiles = |app: &VectorcraftApp, id: u64| {
        let n = app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().clone();
        let vectorcraft_color::Paint::Pattern { xf, .. } = n.appearance.fill_paint() else { panic!("a pattern fill") };
        xf.as_coeffs()
    };
    let labels = ["Transform Object Only", "Transform Pattern Only", "Transform Both"];
    // (panel, which "0°" field: 0 the rotation, 1 the shear)
    let fields = [(show as fn(&mut VectorcraftApp, &mut Ui), 0), (show, 1), (crate::panels::properties::transform_section, 0)];
    for (draw, field) in fields {
        for (item, label) in labels.iter().enumerate() {
            let case = format!("{label}, {}", if field == 0 { "rotation" } else { "shear" });
            let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
            app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
            app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
            app.run("object.pattern.make", json!({"name": "Dots", "width": 20, "height": 20})).unwrap();
            app.run("object.pattern.done", json!({})).unwrap();
            let id = app.run("shape.rectangle", json!({"x": 100, "y": 40, "width": 100, "height": 60})).unwrap()["id"].as_u64().unwrap();
            app.run("paint.setFill", json!({"ids": [id], "swatch": "Dots"})).unwrap();
            app.run("select.set", json!({"ids": [id]})).unwrap();
            let ctx = egui::Context::default();
            crate::theme::install_fonts(&ctx);
            menu_click(&ctx, &mut app, label);
            let before = (app.selection_box().unwrap(), tiles(&app, id));
            // Click the field showing "0°" and type 30 over it.
            use crate::panels::tests_appearance::{click, frame_events};
            let texts = frame_events(&ctx, &mut app, vec![], draw);
            let at = texts.iter().filter(|(t, _)| t == "0°").nth(field).unwrap_or_else(|| panic!("{case}: {texts:?}")).1.center();
            click(&ctx, &mut app, at, draw);
            let key = |key, modifiers| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers };
            frame_events(&ctx, &mut app, vec![key(egui::Key::A, egui::Modifiers::COMMAND)], draw);
            frame_events(&ctx, &mut app, vec![egui::Event::Text("30".into())], draw);
            frame_events(&ctx, &mut app, vec![key(egui::Key::Enter, egui::Modifiers::NONE)], draw);
            frame_events(&ctx, &mut app, vec![], draw);
            let after = (app.selection_box().unwrap(), tiles(&app, id));
            assert_eq!(after.0 != before.0, item != 1, "{case}: the object {:?} → {:?}", before.0, after.0);
            // Turned or sheared tiles change their matrix's linear part; moved ones only its offset.
            assert_eq!(after.1[..4] != before.1[..4], item != 0, "{case}: the tiles {:?} → {:?}", before.1, after.1);
        }
    }
}

/// Use Registration Point for Symbol: with one symbol instance selected, the panel and the
/// Properties panel's Transform section show and set X and Y at its symbol's registration point
/// (here the top-left one) instead of the reference point (the centre).
#[test]
fn use_registration_point_for_symbol_measures_an_instance_from_its_registration_point() {
    for draw in [show as fn(&mut VectorcraftApp, &mut Ui), crate::panels::properties::transform_section] {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
        app.run("shape.rectangle", json!({"x": 100, "y": 50, "width": 80, "height": 40})).unwrap();
        let id = app.run("symbol.new", json!({"name": "Corner", "registration": 0})).unwrap()["id"].as_u64().unwrap();
        app.run("select.set", json!({"ids": [id]})).unwrap();
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let shown = |app: &mut VectorcraftApp| {
            crate::panels::tests_appearance::frame_events(&ctx, app, vec![], draw).into_iter().map(|(t, _)| t).collect::<Vec<_>>()
        };
        let off = shown(&mut app);
        assert!(off.iter().any(|t| t == "140 pt") && off.iter().any(|t| t == "70 pt"), "the centre: {off:?}");
        assert!(menu_texts(&ctx, &mut app).contains(&"   Use Registration Point for Symbol".to_string()));
        menu_click(&ctx, &mut app, "Use Registration Point for Symbol");
        assert!(app.session.prefs.use_registration_point_for_symbol);
        assert!(menu_texts(&ctx, &mut app).contains(&"✓ Use Registration Point for Symbol".to_string()));
        let on = shown(&mut app);
        assert!(on.iter().any(|t| t == "100 pt") && on.iter().any(|t| t == "50 pt") && !on.iter().any(|t| t == "140 pt"), "the top-left: {on:?}");
        type_into(&ctx, &mut app, draw, "100 pt", "30");
        assert_eq!(app.selection_box().unwrap().rect, Rect::new(30.0, 50.0, 110.0, 90.0));
        // Any other selection: the reference point.
        app.run("shape.rectangle", json!({"x": 200, "y": 200, "width": 20, "height": 20})).unwrap();
        let rect = shown(&mut app);
        assert!(rect.iter().any(|t| t == "210 pt") && !rect.iter().any(|t| t == "200 pt"), "the centre: {rect:?}");
    }
}

/// Use Registration Point for Symbol on a reflected, a rotated and a sheared symbol instance:
/// the panel and the Properties panel's Transform section show X and Y at the registration
/// point where the instance's transform put it (not at the box's point of the same index), a
/// new X moves it there, and their rotation field and flip buttons, and the panel menu's
/// flips, turn and mirror the instance about it.
#[test]
fn use_registration_point_for_symbol_follows_a_transformed_instance() {
    let panels = [
        (show as fn(&mut VectorcraftApp, &mut Ui), "Flip Horizontal", "Flip Vertical"),
        (crate::panels::properties::transform_section, "Flip Along Horizontal Axis", "Flip Along Vertical Axis"),
    ];
    // How each instance is transformed after it is made: (command, params).
    let shapes = [
        ("object.reflect", json!({"axis": "vertical"})),
        ("object.rotate", json!({"angle": 90})),
        // About the registration point (the top-left corner), which then lies on the box's
        // top edge but on none of its nine reference points.
        ("object.shear", json!({"angle": 30, "axis": "horizontal", "origin": [100, 50]})),
    ];
    // The registration point (the art's top-left) where the instance's transform puts it.
    let registration = |app: &VectorcraftApp, id: u64| match &app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().kind {
        NodeKind::SymbolInstance { xf, .. } => *xf * vectorcraft_geom::Point::new(-vectorcraft_doc::SYMBOL_HALF, -vectorcraft_doc::SYMBOL_HALF),
        k => panic!("not an instance: {k:?}"),
    };
    let pt = |v: f64| format!("{} pt", (v * 1000.0).round() / 1000.0);
    let near = |a: vectorcraft_geom::Point, b: vectorcraft_geom::Point| (a - b).hypot() < 1e-6;
    for (draw, flip_h, flip_v) in panels {
        for (cmd, params) in &shapes {
            let case = format!("{cmd} {params}");
            let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
            app.run("file.new", json!({"width": 400, "height": 400})).unwrap();
            app.run("shape.rectangle", json!({"x": 100, "y": 50, "width": 80, "height": 40})).unwrap();
            let id = app.run("symbol.new", json!({"name": "Corner", "registration": 0})).unwrap()["id"].as_u64().unwrap();
            app.run("select.set", json!({"ids": [id]})).unwrap();
            app.run(cmd, params.clone()).unwrap();
            app.run("prefs.set", json!({"key": "useRegistrationPointForSymbol", "value": true})).unwrap();
            let ctx = egui::Context::default();
            crate::theme::install_fonts(&ctx);
            ctx.enable_accesskit();
            let q = registration(&app, id);
            let shown: Vec<String> =
                crate::panels::tests_appearance::frame_events(&ctx, &mut app, vec![], draw).into_iter().map(|(t, _)| t).collect();
            assert!(shown.contains(&pt(q.x)) && shown.contains(&pt(q.y)), "{case}: X/Y at {q:?} in {shown:?}");
            // A new X moves the registration point there.
            type_into(&ctx, &mut app, draw, &pt(q.x), &format!("{}", q.x - 40.0));
            let q = vectorcraft_geom::Point::new(q.x - 40.0, q.y);
            assert!(near(registration(&app, id), q), "{case}: moved to {q:?}, now {:?}", registration(&app, id));
            // The rotation field turns it about the registration point.
            let angle = app.selection_box().unwrap().angle;
            let texts = crate::panels::tests_appearance::frame_events(&ctx, &mut app, vec![], draw);
            let degrees = texts.iter().find(|(t, _)| t.ends_with('°')).unwrap_or_else(|| panic!("{case}: no angle in {texts:?}")).0.clone();
            let before = app.session.active().unwrap().revision;
            type_into(&ctx, &mut app, draw, &degrees, &format!("{}", angle + 30.0));
            assert_ne!(app.session.active().unwrap().revision, before, "{case}: turned");
            assert!(near(registration(&app, id), q), "{case}: rotated about {q:?}, now {:?}", registration(&app, id));
            // The flip buttons mirror it about the registration point.
            for label in [flip_h, flip_v] {
                let mut out = ctx.run_ui(egui::RawInput::default(), |ui| draw(&mut app, ui));
                out.textures_delta.clear();
                let nodes = &out.platform_output.accesskit_update.as_ref().expect("an accessibility tree").nodes;
                let (_, n) = nodes
                    .iter()
                    .find(|(_, n)| n.label() == Some(label) && n.role() == egui::accesskit::Role::Button)
                    .unwrap_or_else(|| panic!("no button named {label:?}"));
                let b = n.bounds().expect("bounds");
                let at = egui::pos2((b.x0 + b.x1) as f32 * 0.5, (b.y0 + b.y1) as f32 * 0.5);
                let before = app.session.active().unwrap().revision;
                crate::panels::tests_appearance::click(&ctx, &mut app, at, draw);
                assert_ne!(app.session.active().unwrap().revision, before, "{case}: {label} flipped");
                assert!(near(registration(&app, id), q), "{case}: {label} about {q:?}, now {:?}", registration(&app, id));
            }
            // So do the panel menu's flips.
            for item in ["Flip Horizontal", "Flip Vertical"] {
                menu_click(&ctx, &mut app, item);
                assert!(near(registration(&app, id), q), "{case}: menu {item} about {q:?}, now {:?}", registration(&app, id));
            }
        }
    }
}

#[test]
fn artboard_fields_keep_the_reference_point() {
    let r = Rect::new(0.0, 0.0, 300.0, 200.0);
    // Centre: a new X centres it there, a new W grows it both ways.
    assert_eq!(artboard_rect(r, 4, Some(200.0), None, None, None, false), Rect::new(50.0, 0.0, 350.0, 200.0));
    assert_eq!(artboard_rect(r, 4, None, None, Some(400.0), None, false), Rect::new(-50.0, 0.0, 350.0, 200.0));
    // Top-left: a new W keeps the left edge; linked, H follows.
    assert_eq!(artboard_rect(r, 0, None, None, Some(600.0), None, true), Rect::new(0.0, 0.0, 600.0, 400.0));
    // Bottom-right: a new Y puts the bottom edge there.
    assert_eq!(artboard_rect(r, 8, None, Some(500.0), None, None, false), Rect::new(0.0, 300.0, 300.0, 500.0));
}

#[test]
fn constrain_proportions() {
    assert_eq!(constrained(100.0, 50.0, Some(200.0), None), (200.0, 100.0));
    assert_eq!(constrained(100.0, 50.0, None, Some(25.0)), (50.0, 25.0));
    assert_eq!(constrained(0.0, 50.0, Some(10.0), None), (10.0, 50.0));
}

/// The Transform panel names its Rotate and Shear inputs.
#[test]
fn rotate_and_shear_fields_are_named() {
    use egui_kittest::kittest::Queryable;
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
    app.run("shape.rectangle", json!({"x": 20, "y": 20, "width": 100, "height": 50})).unwrap();
    let mut fonts = false;
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(280.0, 500.0)).with_accessibility_check(false).build_ui_state(
        move |ui, app: &mut VectorcraftApp| {
            if !std::mem::replace(&mut fonts, true) {
                crate::theme::install_fonts(ui.ctx());
                return;
            }
            show(app, ui);
        },
        app,
    );
    h.run_steps(3);
    let _ = h.get_by_role_and_label(egui::accesskit::Role::TextInput, "Rotate");
    let _ = h.get_by_role_and_label(egui::accesskit::Role::TextInput, "Shear");
}
