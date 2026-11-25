use super::*;
use vectorcraft_engine::Session;

fn app() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap();
    app
}

fn paints_hex(app: &VectorcraftApp) -> (String, String) {
    let (f, s) = crate::panels::current_paints(app);
    (f.color().map(|c| c.to_hex()).unwrap_or_default(), s.color().map(|c| c.to_hex()).unwrap_or_default())
}

/// The fill's R, G and B from 0 to 1.
fn fill_rgb(app: &VectorcraftApp) -> [f32; 3] {
    working_rgb(&crate::panels::current_paints(app).0.color().expect("a solid fill"))
}

/// The mode the panel shows for the current target.
fn shown_mode(app: &VectorcraftApp) -> Option<ColorPanelMode> {
    match shown(app, &target_now(app)) {
        ColorPanelShown::Mode(m) => Some(m),
        _ => None,
    }
}

/// In a CMYK document the sliders keep the model picked in the panel for a gradient stop and a
/// freeform point, as they do for a solid fill.
#[test]
fn a_gradient_stop_and_a_freeform_point_keep_the_panels_model_in_a_cmyk_document() {
    let mut app = app();
    app.run("file.documentColorMode", json!({"mode": "cmyk"})).unwrap();
    let stops = json!([{"offset": 0, "color": {"c": 0, "m": 0, "y": 0, "k": 0}}, {"offset": 1, "color": {"c": 0, "m": 100, "y": 100, "k": 0}}]);
    app.run("paint.setFill", json!({"gradient": {"stops": stops}})).unwrap();
    app.select_tool("gradient");
    app.run("gradient.selectStop", json!({"index": 1})).unwrap();
    let orange = Color::rgb8(255, 128, 0);
    let tgt = target_with(&app, false);
    apply(&mut app, &tgt, orange, Live::Released, false);
    let Paint::Gradient(g) = crate::panels::current_paints(&app).0 else { panic!("still a gradient") };
    assert_eq!(g.gradient.stops[1].color, orange);
    app.run("paint.editGradient", json!({"kind": "freeform"})).unwrap();
    let tgt = target_with(&app, false);
    assert!(matches!(tgt, Target::Point { index: 0, .. }), "a freeform point is the target");
    apply(&mut app, &tgt, orange, Live::Released, false);
    let Paint::Gradient(g) = crate::panels::current_paints(&app).0 else { panic!("still a gradient") };
    assert_eq!(g.freeform.map(|f| f.points[0].color), Some(orange));
}

/// In a CMYK document the menu's Create New Swatch saves the color the panel shows in that color's
/// model: an RGB object gives an RGB swatch, and CMYK, gray and spot colors give swatches of their
/// own colors.
#[test]
fn create_new_swatch_keeps_the_shown_colors_model_in_a_cmyk_document() {
    let mut app = app();
    app.run("file.documentColorMode", json!({"mode": "cmyk"})).unwrap();
    app.run("swatch.new", json!({"name": "Ink", "color": {"c": 0, "m": 60, "y": 90, "k": 0}, "spot": true})).unwrap();
    let newest = |app: &VectorcraftApp| app.session.doc().unwrap().doc.swatches.last().cloned().expect("a swatch");
    let ink = app.session.doc().unwrap().doc.swatch("Ink").and_then(|w| w.paint.color()).unwrap();
    for (fill, model) in [
        (json!({"color": "#336699", "keepModel": true}), vectorcraft_color::cms::Model::Rgb),
        (json!({"color": {"c": 10, "m": 20, "y": 30, "k": 5}}), vectorcraft_color::cms::Model::Cmyk),
        (json!({"color": {"gray": 0.4}}), vectorcraft_color::cms::Model::Gray),
        (json!({"swatch": "Ink"}), ink.model()),
    ] {
        app.run("paint.setFill", fill.clone()).unwrap();
        let shown = crate::panels::current_paints(&app).0.color().expect("a solid fill");
        assert_eq!(shown.model(), model, "{fill}");
        let count = app.session.doc().unwrap().doc.swatches.len();
        let tgt = target_now(&app);
        carry_out(&mut app, &tgt, ColorPanelAction::NewSwatch);
        assert_eq!(app.session.doc().unwrap().doc.swatches.len(), count + 1, "{fill}: one new swatch");
        let w = newest(&app);
        assert_eq!(w.paint.color(), Some(shown), "{fill}: the swatch has the shown color, {}", w.name);
    }
}

#[test]
fn the_mode_follows_the_colours_model() {
    let mut app = app();
    app.ui.color_panel.mode = ColorPanelMode::RgbSliders;
    app.run("paint.setFill", json!({"color": {"c": 0.1, "m": 0.2, "y": 0.3, "k": 0.4}})).unwrap();
    assert_eq!(shown_mode(&app), Some(ColorPanelMode::CmykSliders), "a CMYK colour shows CMYK");
    app.run("paint.setFill", json!({"color": {"gray": 0.5}})).unwrap();
    assert_eq!(shown_mode(&app), Some(ColorPanelMode::GrayscaleSlider));
    app.run("paint.setFill", json!({"color": "#336699", "keepModel": true})).unwrap();
    app.ui.color_panel.mode = ColorPanelMode::HsbSliders;
    assert_eq!(shown_mode(&app), Some(ColorPanelMode::HsbSliders), "HSB is remembered for RGB colours");
    app.ui.color_panel.mode = ColorPanelMode::CmykSliders;
    assert_eq!(shown_mode(&app), Some(ColorPanelMode::RgbSliders), "the stored mode wins only when it holds the colour");
    app.ui.color_panel.mode = ColorPanelMode::HueCube;
    assert_eq!(shown_mode(&app), Some(ColorPanelMode::RgbSliders), "a mode off the menu is none picked");
    app.run("select.none", json!({})).unwrap();
    app.ui.color_panel.mode = ColorPanelMode::CmykSliders;
    assert_eq!(shown_mode(&app), Some(ColorPanelMode::CmykSliders), "new art shows the mode picked last");
}

/// #1675: the mode picked last is kept with the UI state, which the app saves with its
/// preferences. The old four-key `ui` table still reads, and a saved value that isn't a menu mode
/// reads as none, with the rest of the state still loading.
#[test]
fn the_picked_mode_comes_back_after_a_restart() {
    let read = |v: Value| legacy_panel_state(v.as_object().unwrap());
    let old = read(json!({"color_mode": "websafe", "color_hide_options": true, "color_show_recent_colors": false, "color_rgb_percent": true}));
    assert_eq!(picked_mode(old.mode), Some(ColorPanelMode::WebColorSliders));
    assert!(old.hide_options && !old.recent_colors && old.rgb_percent);
    for (id, mode) in MODE_IDS {
        assert_eq!(picked_mode(read(json!({"color_mode": id})).mode), Some(mode), "{id}");
    }
    for v in [json!("lab"), json!("CMYK"), json!(3), Value::Null] {
        assert_eq!(picked_mode(read(json!({"color_mode": v})).mode), None, "{v}");
    }
    for v in [json!(7), json!("hide"), Value::Null] {
        let s = read(json!({"color_hide_options": v, "color_show_recent_colors": v, "color_rgb_percent": v}));
        assert!(!s.hide_options && s.recent_colors && !s.rgb_percent, "{v}");
    }
    assert_eq!(read(json!({})), ColorPanelState::default());
    // The new section reads leniently too: junk gives the default and never fails.
    let mut app = app();
    app.run("ui.colorPanel", json!({"mode": "cmyk", "options": false})).unwrap();
    let saved = serde_json::to_value(app.ui.color_panel).unwrap();
    assert_eq!(deserialize_panel_state(saved).unwrap(), app.ui.color_panel);
    for v in [json!(7), json!("hide"), json!({"hideOptions": "yes"}), Value::Null] {
        assert_eq!(deserialize_panel_state(v).unwrap(), ColorPanelState::default());
    }
    assert!(deserialize_lenient_bool(json!(true)).unwrap());
    assert!(!deserialize_lenient_bool(json!("yes")).unwrap());
}

#[test]
fn lab_colours_show_lab_sliders_with_signed_ranges() {
    let lab = Color::lab(50.0, -20.0, 30.0);
    let v = model_values(&lab);
    assert_eq!((v.model, v.values[..3].to_vec()), ("lab", vec![50.0, -20.0, 30.0]));
    assert_eq!(color_of_values(&v), lab);
    let mut wild = v;
    wild.values[..3].copy_from_slice(&[150.0, -300.0, f32::NAN]);
    assert_eq!(color_of_values(&wild), Color::lab(100.0, -128.0, 0.0), "clamped");
    let mut app = app();
    app.ui.color_panel.mode = ColorPanelMode::RgbSliders;
    app.run("paint.setFill", json!({"color": {"l": 50, "a": -20, "b": 30}})).unwrap();
    assert_eq!(shown_mode(&app), Some(ColorPanelMode::LabSliders));
    assert!(picked_mode(ColorPanelMode::LabSliders).is_none(), "not a panel menu mode");
}

#[test]
fn pure_blue_is_out_of_gamut_in_rgb() {
    let app = app();
    let g = gamut(&app);
    let fix = g.nearest(Srgba::rgb(0.0, 0.0, 1.0)).expect("pure blue can't be printed");
    assert!(!Color::rgb(fix.r, fix.g, fix.b).out_of_gamut(), "{fix:?}");
    assert_eq!(g.nearest(Srgba::rgb(0.5, 0.5, 0.5)), None);
}

/// The gamut follows the active colour settings with no key: device CMYK has none to fall outside
/// of, and the generic profile brings it back. Colour settings are process-wide, so it runs alone.
#[test]
fn gamut_follows_a_profile_change() {
    const CHILD: &str = "VECTORCRAFT_TEST_GAMUT_PROFILE_CHANGE";
    if std::env::var_os(CHILD).is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "panels::color::tests::gamut_follows_a_profile_change", "--nocapture"])
            .env(CHILD, "1")
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success() && stdout.contains("test result: ok. 1 passed; 0 failed;"),
            "isolated gamut test failed:\n{stdout}\n{}",
            String::from_utf8_lossy(&output.stderr),
        );
        return;
    }
    let app = app();
    let g = gamut(&app);
    let blue = Srgba::rgb(0.0, 0.0, 1.0);
    let fix = g.nearest(blue);
    assert!(fix.is_some(), "pure blue is out of the default CMYK gamut");
    let set = |cmyk: &str| cms::set_active(&cms::ColorSettings { cmyk: cmyk.into(), ..Default::default() }).unwrap();
    set(cms::DEVICE_CMYK);
    assert_eq!(g.nearest(blue), None, "the same gamut follows the new settings");
    set(cms::GENERIC_CMYK);
    assert_eq!(g.nearest(blue), fix);
}

/// What the panel reports runs the app's commands: a picked colour recolours the selected gradient
/// stop and becomes the Color Guide's base, a recolour click recolours the stop in place, and a
/// mode change converts the colour.
#[test]
fn color_panel_events_run_their_commands() {
    let mut app = app();
    app.run("paint.setFill", json!({"color": "#ff0000"})).unwrap();
    let stops = json!([{"offset": 0, "color": "#ffffff"}, {"offset": 1, "color": "#000000"}]);
    app.run("paint.setFill", json!({"gradient": {"stops": stops, "start": [10, 35], "end": [60, 35]}})).unwrap();
    app.select_tool("gradient");
    app.run("gradient.selectStop", json!({"index": 1})).unwrap();
    let stop_hexes = |app: &VectorcraftApp| {
        let Paint::Gradient(g) = crate::panels::current_paints(app).0 else { panic!("still a gradient") };
        g.gradient.stops.iter().map(|s| s.color.to_hex()).collect::<Vec<String>>()
    };
    // A picked colour (a recent colour's click, a released pick).
    let tgt = target_now(&app);
    choose(&mut app, &tgt, Color::rgb8(255, 0, 0), Live::Released, false);
    assert_eq!(stop_hexes(&app), ["#ffffff", "#ff0000"], "the stop, not the whole gradient");
    assert_eq!(crate::panels::color_guide::base_color(&app).to_hex(), "#ff0000");
    // A recolour click: Invert on the selected stop.
    let tgt = target_now(&app);
    recolor(&mut app, &tgt, true, false);
    assert_eq!(stop_hexes(&app), ["#ffffff", "#00ffff"]);
    // A mode change converts the colour.
    app.run("paint.setFill", json!({"color": "#336699"})).unwrap();
    let tgt = target_now(&app);
    set_mode(&mut app, &tgt, ColorPanelMode::CmykSliders);
    assert_eq!(picked_mode(app.ui.color_panel.mode), Some(ColorPanelMode::CmykSliders));
    assert!(matches!(crate::panels::current_paints(&app).0.color(), Some(Color::Cmyk { .. })), "CMYK converts the colour");
    // A pick for the inactive proxy leaves the fill alone.
    let fill = paints_hex(&app).0;
    let tgt = target_now(&app);
    choose(&mut app, &tgt, Color::rgb8(0, 255, 0), Live::Released, true);
    assert_eq!(paints_hex(&app), (fill, "#00ff00".into()));
}

#[test]
fn a_global_colour_shows_the_tint_row_and_the_t_row_sets_its_tint() {
    let mut app = app();
    app.run("swatch.new", json!({"name": "Ink", "color": {"c": 0, "m": 1, "y": 0, "k": 0}, "spot": true})).unwrap();
    app.run("paint.setFill", json!({"swatch": "Ink", "tint": 50})).unwrap();
    let tgt = target_now(&app);
    assert!(matches!(shown(&app, &tgt), ColorPanelShown::Tint(_)), "a swatch-linked colour shows Tint");
    let tint = tgt.tint().expect("the tint");
    apply_tint(&mut app, &tgt, tint, 0.4, Live::Released);
    let Paint::Solid { swatch, tint, .. } = crate::panels::current_paints(&app).0 else { panic!("still solid") };
    assert_eq!(swatch.as_deref(), Some("Ink"), "the link stays");
    assert!((tint - 0.4).abs() < 1e-3, "{tint}");
}

/// `ui.colorPanel {rgbPercent}` sets the panel's units, and `ui.colorRgbPercent` switches them.
#[test]
fn rgb_percent_switches_the_units_through_the_commands() {
    let mut app = app();
    app.run("paint.setFill", json!({"color": "#336699"})).unwrap();
    assert_eq!(app.run("ui.colorPanel", json!({"rgbPercent": true})).unwrap()["rgbPercent"], true);
    assert!(app.ui.color_panel.rgb_percent);
    assert_eq!(paints_hex(&app).0, "#336699", "the color does not change");
    assert_eq!(app.run("ui.colorRgbPercent", json!({})).unwrap(), json!(false));
    assert!(!app.ui.color_panel.rgb_percent);
    // Typed percents reach the colour as working 0 to 255 values: 50 % is 127.5.
    let mut v = model_values(&Color::rgb(0.2, 0.4, 0.6));
    v.values[0] = 50.0 * 255.0 / 100.0;
    let c = color_of_values(&v);
    let Color::Rgb { r, g, b } = c else { panic!("RGB") };
    assert!((r - 0.5).abs() < 1e-6 && (g - 0.4).abs() < 1e-6 && (b - 0.6).abs() < 1e-6, "{r} {g} {b}");
    app.run("paint.setFill", json!({"color": "#803366"})).unwrap();
    assert!((fill_rgb(&app)[0] - 128.0 / 255.0).abs() < 1e-6);
}

/// `ui.colorPanel {mode}` picks a mode as the panel menu does: HSB shows an RGB fill as H, S and
/// B, and CMYK converts it as one undo step.
#[test]
fn ui_color_panel_picks_a_mode_as_the_menu_does() {
    let mut app = app();
    app.run("paint.setFill", json!({"color": "#336699"})).unwrap();
    let r = app.run("ui.colorPanel", json!({"mode": "hsb"})).unwrap();
    assert_eq!(r, json!({"mode": "hsb", "options": true, "recentColors": true, "rgbPercent": false, "shown": "hsb"}));
    assert_eq!(shown_mode(&app), Some(ColorPanelMode::HsbSliders));
    assert_eq!(app.ui.color_panel.mode, ColorPanelMode::HsbSliders);
    assert_eq!(paints_hex(&app).0, "#336699", "HSB keeps the RGB color");
    assert_eq!(app.run("ui.colorPanel", json!({"mode": "cmyk"})).unwrap()["shown"], "cmyk");
    assert!(matches!(crate::panels::current_paints(&app).0.color(), Some(Color::Cmyk { .. })), "CMYK converts the color");
    app.run("edit.undo", json!({})).unwrap();
    assert!(matches!(crate::panels::current_paints(&app).0.color(), Some(Color::Rgb { .. })), "undo restores it");
    assert_eq!(paints_hex(&app).0, "#336699");
    // `{}` reads the settings: CMYK stays picked, and the RGB fill shows in its own model.
    assert_eq!(
        app.run("ui.colorPanel", json!({})).unwrap(),
        json!({"mode": "cmyk", "options": true, "recentColors": true, "rgbPercent": false, "shown": "rgb"})
    );
    // A tint of a global swatch shows Tint mode.
    app.run("swatch.new", json!({"name": "Ink", "color": {"c": 0, "m": 1, "y": 0, "k": 0}, "spot": true})).unwrap();
    app.run("paint.setFill", json!({"swatch": "Ink", "tint": 50})).unwrap();
    assert_eq!(app.run("ui.colorPanel", json!({})).unwrap()["shown"], "tint");
}

/// A mode the panel menu doesn't list, Lab included, and an options value that isn't true or
/// false answer an error, and the command then changes nothing.
#[test]
fn ui_color_panel_answers_an_error_for_a_mode_off_the_menu() {
    let mut app = app();
    for m in [json!("lab"), json!("oklch"), json!("hsl"), json!("future"), json!("RGB"), json!(3)] {
        let e = app.run("ui.colorPanel", json!({"mode": m})).unwrap_err();
        assert_eq!(e, "`mode` must be grayscale, rgb, hsb, cmyk or websafe", "{m}");
    }
    assert_eq!(app.run("ui.colorPanel", json!({"mode": "cmyk", "options": "no"})).unwrap_err(), "`options` must be true or false");
    assert_eq!((picked_mode(app.ui.color_panel.mode), app.ui.color_panel.hide_options), (None, false));
}

/// `ui.colorPanel` answers every junk parameter, with and without a document, without a panic.
#[test]
fn ui_color_panel_answers_junk_params_without_a_panic() {
    let doc = crate::menus::UI_COMMANDS.iter().find(|c| c.0 == "ui.colorPanel").expect("registered").3;
    let mut app = app();
    app.run("paint.setFill", json!({"color": "#336699"})).unwrap();
    for p in vectorcraft_testkit::strategies::junk_params(doc, "/nonexistent").into_iter().chain(vectorcraft_testkit::strategies::junk_values()) {
        let _ = app.run("ui.colorPanel", p);
    }
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    assert_eq!(app.run("ui.colorPanel", json!({"mode": "cmyk"})).unwrap()["mode"], "cmyk");
}

#[test]
fn a_mixed_selection_has_no_colour_to_edit() {
    let mut app = app();
    let ctx = crate::testing::ui_context();
    let b = app.run("shape.rectangle", json!({"x": 80, "y": 10, "width": 50, "height": 50})).unwrap()["id"].clone();
    app.run("select.all", json!({})).unwrap();
    app.run("paint.setFill", json!({"color": "#ff0000"})).unwrap();
    app.run("paint.setFill", json!({"color": "#00ff00", "ids": [b]})).unwrap();
    assert_eq!(crate::panels::mixed_paints(&app, &ctx), (true, false));
    ctx.run_ui(Default::default(), |ui| assert_eq!(target(&app, ui).color(), None)).textures_delta.clear();
    // The Eyedropper armed for a stop of the Live Paint Bucket's gradient shows the bucket's color.
    app.select_tool("eyedropper");
    assert_eq!(crate::panels::mixed_paints(&app, &ctx), (true, false));
    app.session.set_tool_option("stop", &json!("livePaintBucket"));
    assert_eq!(crate::panels::mixed_paints(&app, &ctx), (false, false), "arming the pick recomputes it");
    app.session.set_tool_option("stop", &serde_json::Value::Null);
    app.run("paint.setFill", json!({"color": "#0000ff"})).unwrap();
    assert_eq!(crate::panels::mixed_paints(&app, &ctx), (false, false), "a new revision recomputes it");
}

/// With gradient mesh points picked, the panel edits and shows their color (a solid fill
/// recolors just them), not the mesh object's fill, so the sliders keep the value they set,
/// through a drag too; "?" when the picked points differ, which picking alone (no new revision)
/// updates. The Control bar's opacity is the picked point's.
#[test]
fn picked_mesh_points_are_the_color_the_panel_shows_and_edits() {
    use vectorcraft_doc::NodeKind;
    use vectorcraft_engine::ViewInfo;
    use vectorcraft_tools::{PointerEvent, PointerKind};
    let mut app = app();
    let ctx = crate::testing::ui_context();
    let id = app.session.active().unwrap().selection.objects[0];
    app.run("paint.setFill", json!({"color": "#ff8500"})).unwrap();
    app.run("object.mesh.create", json!({"rows": 2, "cols": 2})).unwrap();
    app.run("object.mesh.setPointColor", json!({"id": id.0, "index": 0, "color": "#0000ff", "opacity": 0.5})).unwrap();
    let v = ViewInfo::default();
    app.session.select_tool("mesh", v).unwrap();
    let click = |app: &mut VectorcraftApp, x: f64, y: f64, shift: bool| {
        for k in [PointerKind::Down, PointerKind::Up] {
            let mut ev = PointerEvent::new(k, x, y);
            ev.mods.shift = shift;
            app.session.pointer(&ev, v).unwrap();
        }
    };
    let picked = |app: &VectorcraftApp| app.session.fill_mesh_points().into_iter().flat_map(|(_, v)| v).collect::<Vec<_>>();
    // The top-left corner point (index 0).
    click(&mut app, 10.0, 10.0, false);
    assert_eq!(picked(&app), vec![0], "the click picks the top-left point");
    assert_eq!(crate::panels::mixed_paints(&app, &ctx), (false, false));
    let tgt = target_with(&app, false);
    assert_eq!(tgt.color().map(|c| c.to_hex()), Some("#0000ff".into()), "the picked point's color, not the object's fill");
    assert_eq!(crate::panels::current_transparency(&app).map(|t| t.0), Some(0.5), "the picked point's opacity");
    // A slider drag: each step shows the color it set, and the release keeps it.
    for c in [Color::rgb8(250, 0, 120), Color::rgb8(240, 10, 140)] {
        let tgt = target_with(&app, false);
        apply(&mut app, &tgt, c, Live::Dragging, false);
        assert_eq!(target_with(&app, false).color(), Some(c), "the drag shows the color it set");
    }
    let orange = Color::rgb8(255, 128, 0);
    let tgt = target_with(&app, false);
    apply(&mut app, &tgt, orange, Live::Released, false);
    assert_eq!(target_with(&app, false).color(), Some(orange), "the panel keeps the color it set");
    let NodeKind::Mesh(m) = &app.session.active().unwrap().doc.node(id).unwrap().kind else { panic!("still a gradient mesh") };
    assert_eq!(m.points[0].color, orange);
    let middle = m.idx(0, 1);
    assert_ne!(m.points[middle].color, orange, "only the picked point");
    assert_eq!(crate::panels::mixed_paints(&app, &ctx), (false, false));
    // Shift-click the top middle point (still the object's color): the two differ.
    click(&mut app, 35.0, 10.0, true);
    assert_eq!(picked(&app), vec![0, middle], "the Shift-click adds the top middle point");
    assert_eq!(crate::panels::mixed_paints(&app, &ctx), (true, false), "picking recomputes it");
}

/// The panel's Fill and Stroke wells show a gradient and None as the Tools panel's proxy does,
/// not as black.
#[test]
fn the_wells_show_a_gradient_and_none_as_the_tools_proxy_does() {
    use crate::tests::panel_harness::Panel;
    let painted = || {
        let mut app = app();
        let stops = json!([{"offset": 0, "color": "#ff0000"}, {"offset": 1, "color": "#0000ff"}]);
        app.run("paint.setFill", json!({"gradient": {"stops": stops}})).unwrap();
        app.run("paint.setStroke", json!({"none": true})).unwrap();
        app
    };
    let color = Panel::new(painted(), show);
    let tools = Panel::new(painted(), crate::panels::proxy);
    // The wells are unnamed parts of the pair, so they are found by the value each one reads as.
    let wells = tools.values();
    assert_eq!(wells.len(), 2, "the Tools proxy's two wells read as {wells:?}");
    assert_ne!(wells[0], wells[1], "a gradient and None read differently");
    for well in &wells {
        assert!(color.shows_value(well), "the Color panel shows a well that reads as {well:?}");
    }
}
