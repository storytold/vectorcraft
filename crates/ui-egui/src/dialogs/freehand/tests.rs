use super::*;
use craft::ui::dialog::DialogId;
use vectorcraft_engine::Session;

fn app() -> VectorcraftApp {
    let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app
}

/// The id of the open dialog `key`.
fn open_id(app: &VectorcraftApp, key: &str) -> DialogId {
    app.dialogs()
        .expect("a window")
        .with(|host| host.open_dialogs().into_iter().find(|d| d.key.as_deref() == Some(key)).map(|d| d.id))
        .unwrap_or_else(|| panic!("`{key}` is open"))
}

/// Opens `tool`'s Tool Options as the user does, sets `fields` through the open dialog, presses OK and lets the
/// held answer through.
fn set_through_dialog(app: &mut VectorcraftApp, tool: &str, fields: &[(&str, Value)]) {
    let key = app.run("tool.options", json!({"tool": tool})).unwrap()["dialog"].as_str().unwrap().to_owned();
    let (ctx, id) = (app.ui_ctx.clone().expect("a window"), open_id(app, &key));
    let host = app.dialogs().expect("a window");
    for (field, value) in fields {
        host.with(|host| host.set_json_field(&ctx, id, field, value.clone())).unwrap();
    }
    // The frames after OK take the held answer and close the dialog, before the next open.
    assert!(crate::dialogs::tests::confirm(app), "{tool}: OK closes the dialog");
}

#[test]
fn the_fidelity_keys_step_finely_at_the_low_end() {
    let up: Vec<f64> = std::iter::successors(Some(0.01), |&v| Some(fidelity_step(v, 1.0))).take(21).collect();
    assert_eq!(up, [0.01, 0.02, 0.03, 0.04, 0.05, 0.06, 0.07, 0.08, 0.09, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0, 2.0, 3.0]);
    // Down retraces the same steps.
    let down: Vec<f64> = std::iter::successors(Some(3.0), |&v| Some(fidelity_step(v, -1.0))).take(21).collect();
    assert_eq!(down, up.iter().rev().copied().collect::<Vec<_>>());
    // Ten steps at once (Shift) multiply or divide by ten.
    assert_eq!((fidelity_step(0.01, 10.0), fidelity_step(0.1, 10.0), fidelity_step(1.0, 10.0)), (0.1, 1.0, 10.0));
    assert_eq!((fidelity_step(10.0, -10.0), fidelity_step(1.0, -10.0), fidelity_step(0.1, -10.0), fidelity_step(0.03, 10.0)), (1.0, 0.1, 0.01, 0.3));
    // A value between steps goes to the next one in its direction.
    assert_eq!((fidelity_step(0.035, 1.0), fidelity_step(0.035, -1.0), fidelity_step(1.5, 1.0), fidelity_step(1.5, -1.0)), (0.04, 0.03, 2.0, 1.0));
}

/// Smooth Tool Options shows Smoothness under Fidelity, at 50% until it is set, and OK sets it from 0% to 100%; a
/// Smoothness past 100% is refused and sets nothing. The other freehand tools have no Smoothness.
#[test]
fn smooth_tool_options_set_the_smoothness() {
    let mut app = app();
    let smooth = app.session.tool_options_of("smooth");
    assert_eq!(smooth["smoothness"], json!(50.0));
    let present: Vec<&str> = KEYS.iter().copied().filter(|key| smooth.get(*key).is_some()).collect();
    let keys: Vec<String> = params("smooth", &present).iter().map(|p| p.key.to_string()).collect();
    let (fidelity, smoothness) = (keys.iter().position(|k| k == "fidelity"), keys.iter().position(|k| k == "smoothness"));
    assert!(fidelity.is_some() && fidelity < smoothness, "{keys:?}");
    for value in [30.0, 0.0, 100.0] {
        set_through_dialog(&mut app, "smooth", &[("smoothness", json!(value))]);
        assert_eq!(app.session.tool_options_of("smooth")["smoothness"], json!(value));
    }
    let key = app.run("tool.options", json!({"tool": "smooth"})).unwrap()["dialog"].as_str().unwrap().to_owned();
    let (ctx, id) = (app.ui_ctx.clone().expect("a window"), open_id(&app, &key));
    let host = app.dialogs().expect("a window");
    assert!(host.with(|host| host.set_json_field(&ctx, id, "smoothness", json!(120))).is_err(), "120% is refused");
    host.with(|host| host.cancel_by_id(&ctx, id));
    assert_eq!(app.session.tool_options_of("smooth")["smoothness"], json!(100.0));
    for tool in ["pencil", "paintbrush", "blobBrush", "eraser"] {
        assert!(app.session.tool_options_of(tool).get("smoothness").is_none(), "{tool}");
    }
}

/// Each tool's dialog, opened as the user opens it, stores the values set in it as the tool's options: the field
/// keys, the units (Liquify's percent is a fraction) and the companion keys of the variation rows.
#[test]
fn ok_sets_each_tools_options() {
    let mut app = app();
    let layers = app.session.doc().unwrap().doc.layers.clone();
    let circle = vectorcraft_tools::circle::ID;
    let arc = vectorcraft_tools::three_point_arc::ID;
    let curvature = vectorcraft_tools::curvature_circle::ID;
    // The tool opened, the fields set, the tool whose options are read, and the options expected there.
    let rows: Vec<(&str, Value, &str, Value)> = vec![
        (
            "pencil",
            json!({"fidelity": 6.5, "fill": true, "altSmooth": true, "closeWithin": 0, "editWithin": 20, "liveFit": false}),
            "pencil",
            json!({"fidelity": 6.5, "fill": true, "altSmooth": true, "closeWithin": 0.0, "editWithin": 20.0, "liveFit": false}),
        ),
        ("paintbrush", json!({"fidelity": 3, "keepSelected": false}), "paintbrush", json!({"fidelity": 3.0, "keepSelected": false})),
        ("smooth", json!({"fidelity": 5, "smoothness": 75}), "smooth", json!({"fidelity": 5.0, "smoothness": 75.0})),
        (
            "blobBrush",
            json!({"angle": 45, "roundness": 0, "sizeMode": "pressure", "sizeVariation": 6, "fidelity": 0.01, "keepSelected": true, "mergeOnlySelection": true}),
            "blobBrush",
            json!({"angle": 45.0, "roundness": 0.0, "sizeMode": "pressure", "sizeVariation": 6.0, "fidelity": 0.01, "keepSelected": true, "mergeOnlySelection": true}),
        ),
        (
            "eraser",
            json!({"angle": 90, "roundness": 25, "size": 20, "angleMode": "bearing", "angleVariation": 30, "roundnessMode": "random",
                   "roundnessVariation": 20, "sizeMode": "rotation", "sizeVariation": 5}),
            "eraser",
            json!({"angle": 90.0, "roundness": 25.0, "size": 20.0, "angleMode": "bearing", "angleVariation": 30.0, "roundnessMode": "random",
                   "roundnessVariation": 20.0, "sizeMode": "rotation", "sizeVariation": 5.0}),
        ),
        (
            circle,
            json!({"radii": false, "standardAnchors": false, "dragThird": false, "color": "magenta"}),
            circle,
            json!({"radii": false, "standardAnchors": false, "dragThird": false, "color": "magenta"}),
        ),
        (
            curvature,
            json!({"snapTolerance": 8, "annotateRadius": false, "standardAnchors": false}),
            curvature,
            json!({"snapTolerance": 8.0, "snapWithoutGuides": true, "annotateRadius": false, "standardAnchors": false, "constantRadius": true}),
        ),
        (arc, json!({"direction": true}), arc, json!({"direction": true})),
        ("objectsOnPath", json!({"pivot": 7, "rotate": 30}), "objectsOnPath", json!({"pivot": 7, "rotate": 30.0})),
        (
            "twirl",
            json!({"width": 80, "intensity": 30, "rate": -120, "simplifyOn": false, "showBrush": false}),
            "twirl",
            json!({"width": 80.0, "intensity": 0.3, "rate": -120.0, "simplifyOn": false, "showBrush": false}),
        ),
        // One brush serves the eight Symbolism tools. Each tool's dialog takes the rows that tool shows and enables
        // (od.OD4): the Sprayer's own options in its dialog, Intensity, Method, pressure and the Sizer's own in the
        // Sizer's.
        (
            "symbolSprayer",
            json!({"diameter": 120, "density": 2, "showBrush": false, "sprayStain": "userDefined", "spraySpin": "userDefined"}),
            "symbolScreener",
            json!({"diameter": 120.0, "density": 2.0, "showBrush": false, "sprayStain": "userDefined", "spraySpin": "userDefined"}),
        ),
        (
            "symbolSizer",
            json!({"intensity": 9, "method": "random", "usePressure": true, "proportional": false, "affectsDensity": true}),
            "symbolScreener",
            json!({"diameter": 120.0, "intensity": 9.0, "density": 2.0, "method": "random", "usePressure": true, "showBrush": false,
                   "sprayStain": "userDefined", "spraySpin": "userDefined", "proportional": false, "affectsDensity": true}),
        ),
        ("flare", json!({"rays": 4, "ringsOn": false, "growth": 80}), "flare", json!({"rays": 4.0, "ringsOn": false, "growth": 80.0})),
    ];
    for (tool, set, read, expected) in rows {
        let set: Vec<(&str, Value)> = set.as_object().unwrap().iter().map(|(k, v)| (k.as_str(), v.clone())).collect();
        set_through_dialog(&mut app, tool, &set);
        let options = app.session.tool_options_of(read);
        for (key, want) in expected.as_object().unwrap() {
            let got = &options[key.as_str()];
            let same = match (got.as_f64(), want.as_f64()) {
                (Some(a), Some(b)) => (a - b).abs() < 1e-9,
                _ => got == want,
            };
            assert!(same, "{tool}: `{key}` is {got}, expected {want}");
        }
    }
    assert_eq!(app.session.doc().unwrap().doc.layers, layers, "OK from a toolbar draws nothing");
}

/// Live curve fitting (the Pencil's, on by default): on, the path shown while a stroke is
/// drawn is fitted; off, it joins the stroke's points with straight segments, and the release
/// fits it as with the option on.
#[test]
fn live_curve_fitting_off_shows_the_points_until_the_release_fits_them() {
    let mut app = app();
    // The window's first frames fit the view, as the dialog's frames below would: the Pencil's Fidelity is in screen
    // pixels, so both strokes are drawn at one zoom.
    crate::dialogs::tests::frame(&mut app, vec![]);
    crate::dialogs::tests::frame(&mut app, vec![]);
    // The anchors of the newest path while a wave around height `y0` is drawn and after the
    // release, with nothing selected (a stroke starting at the end of a selected path would
    // continue it).
    let draw = |app: &mut VectorcraftApp, y0: f64| {
        app.run("select.set", json!({"ids": []})).unwrap();
        app.select_tool("pencil");
        let view = app.view_info();
        let pts: Vec<_> = (0..=40).map(|i| (20.0 + i as f64 * 8.0, y0 + 30.0 * (i as f64 * 0.25).sin())).collect();
        let pointer = |kind, (x, y)| vectorcraft_tools::PointerEvent::new(kind, x, y);
        let newest = |app: &VectorcraftApp| {
            let doc = &app.session.doc().unwrap().doc;
            doc.layers[0].children().unwrap().last().unwrap().path_data().unwrap().subpaths[0].anchors.to_vec()
        };
        crate::canvas::dispatch(app, &pointer(vectorcraft_tools::PointerKind::Down, pts[0]), view);
        for p in &pts[1..] {
            crate::canvas::dispatch(app, &pointer(vectorcraft_tools::PointerKind::Drag, *p), view);
        }
        let drawing = newest(app);
        crate::canvas::dispatch(app, &pointer(vectorcraft_tools::PointerKind::Up, pts[40]), view);
        (drawing, newest(app))
    };
    let corners = |anchors: &[vectorcraft_geom::Anchor]| anchors.iter().all(|a| a.h_in == a.p && a.h_out == a.p);
    // On by default.
    assert_eq!(app.session.tool_options_of("pencil")["liveFit"].as_bool(), Some(true));
    let (drawing, on) = draw(&mut app, 100.0);
    assert!(!corners(&drawing) && drawing.len() < 20, "fitted while drawn: {} anchors", drawing.len());
    // Off: the points while the stroke is drawn, fitted at the release.
    set_through_dialog(&mut app, "pencil", &[("liveFit", json!(false))]);
    app.select_tool("selection");
    assert_eq!(app.session.tool_options_of("pencil")["liveFit"].as_bool(), Some(false), "it lasts while another tool is chosen");
    let (drawing, off) = draw(&mut app, 200.0);
    assert!(corners(&drawing) && drawing.len() > 30, "the points while drawn: {} anchors", drawing.len());
    assert_eq!(off.len(), on.len(), "the release fits the stroke as with the option on");
    assert!(off.iter().zip(&on).all(|(a, b)| (a.p.x - b.p.x).abs() < 1e-9 && (a.p.y - b.p.y - 100.0).abs() < 1e-9), "{off:?}");
    assert!(app.session.tool_options_of("paintbrush").get("liveFit").is_none());
}

#[test]
fn a_new_fidelity_changes_what_the_pencil_draws() {
    let mut app = app();
    // The anchors of a wave drawn around height `y0`, with nothing selected (a stroke starting
    // at the end of a selected path would continue it).
    let anchors = |app: &mut VectorcraftApp, y0: f64| {
        app.run("select.set", json!({"ids": []})).unwrap();
        app.select_tool("pencil");
        let view = app.view_info();
        let pts: Vec<_> = (0..=40).map(|i| (20.0 + i as f64 * 8.0, y0 + 30.0 * (i as f64 * 0.25).sin())).collect();
        let pointer = |kind, (x, y)| vectorcraft_tools::PointerEvent::new(kind, x, y);
        crate::canvas::dispatch(app, &pointer(vectorcraft_tools::PointerKind::Down, pts[0]), view);
        for p in &pts[1..] {
            crate::canvas::dispatch(app, &pointer(vectorcraft_tools::PointerKind::Drag, *p), view);
        }
        crate::canvas::dispatch(app, &pointer(vectorcraft_tools::PointerKind::Up, pts[40]), view);
        let doc = &app.session.doc().unwrap().doc;
        doc.layers[0].children().unwrap().last().unwrap().path_data().map_or(0, |p| p.subpaths.iter().map(|s| s.anchors.len()).sum::<usize>())
    };
    let precise = anchors(&mut app, 100.0);
    set_through_dialog(&mut app, "pencil", &[("fidelity", json!(12))]);
    let smooth = anchors(&mut app, 200.0);
    assert!(smooth < precise, "{smooth} anchors at fidelity 12, {precise} at the default");
}

/// The Blob Brush's Fidelity, set in its Tool Options, changes the shape its strokes paint: at
/// 0.01 pt the brush follows a stroke's jitter, and at the default 1.5 pt it smooths it away.
#[test]
fn a_new_fidelity_changes_what_the_blob_brush_draws() {
    let mut app = app();
    // The anchors of the blob a jittery wave paints around height `y0`.
    let anchors = |app: &mut VectorcraftApp, y0: f64| {
        app.select_tool("blobBrush");
        let view = app.view_info();
        let jitter = |i: i32| if i % 2 == 0 { 1.0 } else { -1.0 };
        let pts: Vec<_> = (0..=40).map(|i| (20.0 + f64::from(i) * 8.0, y0 + 30.0 * (f64::from(i) * 0.25).sin() + jitter(i))).collect();
        let pointer = |kind, (x, y)| vectorcraft_tools::PointerEvent::new(kind, x, y);
        crate::canvas::dispatch(app, &pointer(vectorcraft_tools::PointerKind::Down, pts[0]), view);
        for p in &pts[1..] {
            crate::canvas::dispatch(app, &pointer(vectorcraft_tools::PointerKind::Drag, *p), view);
        }
        crate::canvas::dispatch(app, &pointer(vectorcraft_tools::PointerKind::Up, pts[40]), view);
        let doc = &app.session.doc().unwrap().doc;
        doc.layers[0].children().unwrap().last().unwrap().path_data().map_or(0, |p| p.subpaths.iter().map(|s| s.anchors.len()).sum::<usize>())
    };
    assert_eq!(app.session.tool_options_of("blobBrush")["fidelity"].as_f64(), Some(1.5));
    let smooth = anchors(&mut app, 100.0);
    set_through_dialog(&mut app, "blobBrush", &[("fidelity", json!(0.01))]);
    app.select_tool("selection");
    assert_eq!(app.session.tool_options_of("blobBrush")["fidelity"].as_f64(), Some(0.01), "it lasts while another tool is chosen");
    let precise = anchors(&mut app, 200.0);
    assert!(smooth > 0 && smooth < precise, "{smooth} anchors at the default, {precise} at fidelity 0.01");
}

#[test]
fn the_new_path_is_selected_only_with_keep_selected_on() {
    let mut app = app();
    // A stroke of `tool` along height `y0`; the ids selected afterwards and the new path's id.
    let stroke = |app: &mut VectorcraftApp, tool: &str, y0: f64| {
        app.select_tool(tool);
        let view = app.view_info();
        let pointer = |kind, x| vectorcraft_tools::PointerEvent::new(kind, x, y0);
        crate::canvas::dispatch(app, &pointer(vectorcraft_tools::PointerKind::Down, 20.0), view);
        for x in [60.0, 100.0, 140.0] {
            crate::canvas::dispatch(app, &pointer(vectorcraft_tools::PointerKind::Drag, x), view);
        }
        crate::canvas::dispatch(app, &pointer(vectorcraft_tools::PointerKind::Up, 180.0), view);
        let st = app.session.doc().unwrap();
        (st.selection.objects.clone(), st.doc.layers[0].children().unwrap().last().unwrap().id)
    };
    for tool in ["pencil", "paintbrush"] {
        app.run("select.set", json!({"ids": []})).unwrap();
        // On by default: the new path is selected.
        assert_eq!(app.session.tool_options_of(tool)["keepSelected"].as_bool(), Some(true), "{tool}");
        let (selected, id) = stroke(&mut app, tool, 100.0);
        assert_eq!(selected, vec![id], "{tool}");
        // Off: the next stroke leaves nothing selected, the earlier path included (#5752).
        set_through_dialog(&mut app, tool, &[("keepSelected", json!(false))]);
        // It lasts while another tool is chosen.
        app.select_tool("selection");
        assert_eq!(app.session.tool_options_of(tool)["keepSelected"].as_bool(), Some(false), "{tool}");
        let (selected, new) = stroke(&mut app, tool, 200.0);
        assert_ne!(new, id, "{tool} drew a new path");
        assert!(selected.is_empty(), "{tool}: the earlier path is no longer selected either");
    }
}

#[test]
fn the_blob_brush_options_set_what_a_stroke_merges_with_and_selects() {
    let mut app = app();
    // A Blob Brush stroke across the canvas at height 100; the ids selected afterwards.
    let stroke = |app: &mut VectorcraftApp| {
        app.select_tool("blobBrush");
        let view = app.view_info();
        let pointer = |kind, x| vectorcraft_tools::PointerEvent::new(kind, x, 100.0);
        crate::canvas::dispatch(app, &pointer(vectorcraft_tools::PointerKind::Down, 20.0), view);
        for x in [80.0, 140.0, 200.0, 260.0] {
            crate::canvas::dispatch(app, &pointer(vectorcraft_tools::PointerKind::Drag, x), view);
        }
        crate::canvas::dispatch(app, &pointer(vectorcraft_tools::PointerKind::Up, 300.0), view);
        app.session.doc().unwrap().selection.objects.clone()
    };
    let ids = |app: &VectorcraftApp| app.session.doc().unwrap().doc.layers[0].children().unwrap().iter().map(|n| n.id).collect::<Vec<_>>();
    // Two blobs the stroke crosses, drawn with the same color.
    let blob = |app: &mut VectorcraftApp, x: f64| {
        let r = app.run("path.blob", json!({"points": [[x, 60], [x, 140]], "size": 20})).unwrap();
        vectorcraft_doc::NodeId(r["id"].as_u64().unwrap())
    };
    let (a, b) = (blob(&mut app, 100.0), blob(&mut app, 240.0));
    // Both are off by default.
    let o = app.session.tool_options_of("blobBrush");
    assert_eq!((o["keepSelected"].as_bool(), o["mergeOnlySelection"].as_bool()), (Some(false), Some(false)));
    set_through_dialog(&mut app, "blobBrush", &[("keepSelected", json!(true)), ("mergeOnlySelection", json!(true))]);
    let o = app.session.tool_options_of("blobBrush");
    assert_eq!((o["keepSelected"].as_bool(), o["mergeOnlySelection"].as_bool()), (Some(true), Some(true)));
    // With `b` selected, the stroke merges into `b` alone, and leaves it selected.
    app.run("select.set", json!({"ids": [b.0]})).unwrap();
    assert_eq!(stroke(&mut app), vec![b]);
    assert_eq!(ids(&app), vec![a, b], "merged into b only");
}

/// The Blob Brush's and the Eraser's Size is Fixed or Pressure (#4590, #3961), with the Variation
/// Pressure adds and takes; OK sets both, each tool keeping its own, and the Pencil has neither.
#[test]
fn the_blob_brush_and_eraser_size_can_follow_the_pen_pressure() {
    let mut app = app();
    for (tool, variation) in [("blobBrush", 6.0), ("eraser", 4.0)] {
        let o = app.session.tool_options_of(tool);
        assert_eq!((o["sizeMode"].as_str(), o["sizeVariation"].as_f64()), (Some("fixed"), Some(0.0)), "{tool}: Fixed with no variation by default");
        set_through_dialog(&mut app, tool, &[("sizeMode", json!("pressure")), ("sizeVariation", json!(variation))]);
        let o = app.session.tool_options_of(tool);
        assert_eq!((o["sizeMode"].clone(), o["sizeVariation"].as_f64()), (json!("pressure"), Some(variation)), "{tool}");
    }
    let o = app.session.tool_options_of("pencil");
    assert!(o.get("sizeMode").is_none() && o.get("sizeVariation").is_none());
}

/// The names and values the window's access tree shows.
fn shown_text(app: &mut VectorcraftApp) -> String {
    crate::dialogs::tests::settle(app);
    let out = crate::dialogs::tests::frame(app, vec![]);
    let update = out.platform_output.accesskit_update.expect("an accessibility tree");
    update.nodes.iter().flat_map(|(_, node)| [node.label(), node.value()]).flatten().collect::<Vec<_>>().join("\n")
}

/// The Blob Brush's Size menu offers Stylus Wheel beside Fixed and Pressure, Fixed still the
/// default; the menu shows it once chosen, and OK sets it with the Variation.
#[test]
fn the_blob_brush_size_offers_the_stylus_wheel() {
    let mut app = app();
    app.run("tool.options", json!({"tool": "blobBrush"})).unwrap();
    assert_eq!(crate::dialogs::tests::fields(&app)["sizeMode"], json!("fixed"), "Fixed by default");
    crate::dialogs::tests::set(&mut app, "sizeMode", json!("stylusWheel"));
    crate::dialogs::tests::set(&mut app, "sizeVariation", json!(5));
    let text = shown_text(&mut app);
    assert!(text.contains("Stylus Wheel"), "{text}");
    assert!(crate::dialogs::tests::confirm(&mut app), "OK closes the dialog");
    let o = app.session.tool_options_of("blobBrush");
    assert_eq!((o["sizeMode"].clone(), o["sizeVariation"].as_f64()), (json!("stylusWheel"), Some(5.0)));
    app.run("tool.options", json!({"tool": "blobBrush"})).unwrap();
    let text = shown_text(&mut app);
    assert!(text.contains("Stylus Wheel"), "it shows again on the next open: {text}");
}
