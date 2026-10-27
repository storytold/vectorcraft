use super::*;
use vectorcraft_engine::Session;

fn app() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app
}

/// The labels the open dialog paints.
fn painted(app: &mut VectorcraftApp) -> String {
    crate::tests_labels::painted_text(app, |app, ui| super::super::show(app, ui.ctx()))
}

#[test]
fn the_fidelity_rail_is_logarithmic_and_round_trips_two_decimals() {
    let rail = (0.01, FIDELITY_MAX);
    assert_eq!((fidelity_at(0.01, rail), fidelity_at(20.0, rail)), (0.0, 1.0));
    assert_eq!((fidelity_from(0.0, rail), fidelity_from(1.0, rail)), (0.01, 20.0));
    // 0.01–1 pt gets more than half of the rail.
    assert!(fidelity_at(1.0, rail) > 0.5, "{}", fidelity_at(1.0, rail));
    // The values a user picks come back as they were, and the rail only ever rises.
    for v in [0.01, 0.02, 0.05, 0.1, 0.25, 0.5, 1.5, 6.5, 12.0, 20.0] {
        assert_eq!(fidelity_from(fidelity_at(v, rail), rail), v, "{v}");
    }
    let steps: Vec<f64> = (0..=200).map(|i| fidelity_from(i as f32 / 200.0, rail)).collect();
    assert!(steps.windows(2).all(|w| w[0] <= w[1]), "{steps:?}");
    // Past the ends and on bad input it stays on the rail.
    assert_eq!((fidelity_at(0.0, rail), fidelity_at(50.0, rail), fidelity_at(f64::NAN, rail)), (0.0, 1.0, 0.0));
    assert_eq!((fidelity_from(-1.0, rail), fidelity_from(2.0, rail)), (0.01, 20.0));
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

#[test]
fn the_fidelity_field_takes_a_hundredth_for_the_pencil() {
    let mut app = app();
    app.run("tool.options", json!({"tool": "pencil"})).unwrap();
    app.ui.dialog.as_mut().unwrap().fields.insert("fidelity".into(), json!(0.02));
    let text = painted(&mut app);
    assert!(text.contains("0.02"), "{text}");
    super::super::confirm(&mut app).unwrap();
    assert_eq!(app.session.tool_options_of("pencil")["fidelity"].as_f64(), Some(0.02));
}

#[test]
fn each_tool_shows_its_options_and_ok_sets_them() {
    let mut app = app();
    // Pencil: Fidelity, fill, the two tolerances; no Size.
    app.run("tool.options", json!({"tool": "pencil"})).unwrap();
    let text = painted(&mut app);
    for label in [
        "Pencil Tool Options",
        "Fidelity:",
        "Accurate",
        "Smooth",
        "Fill new strokes",
        "Alt key toggles to Smooth Tool",
        "Close paths when ends are within:",
        "Edit selected paths within:",
        "Live preview",
    ] {
        assert!(text.contains(label), "{label} in {text}");
    }
    assert!(!text.contains("Size:"), "{text}");
    let d = app.ui.dialog.as_mut().unwrap();
    assert_eq!(
        (d.f64("fidelity", 0.0), d.bool("fill"), d.bool("altSmooth"), d.f64("closeWithin", 0.0), d.f64("editWithin", 0.0)),
        (1.5, false, false, 15.0, 12.0)
    );
    for (k, v) in [("fidelity", json!(6.5)), ("fill", json!(true)), ("altSmooth", json!(true)), ("closeWithin", json!(0)), ("editWithin", json!(20))]
    {
        d.fields.insert(k.into(), v);
    }
    super::super::confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none());
    let o = app.session.tool_options_of("pencil");
    assert_eq!(
        (o["fidelity"].as_f64(), o["fill"].as_bool(), o["altSmooth"].as_bool(), o["closeWithin"].as_f64(), o["editWithin"].as_f64()),
        (Some(6.5), Some(true), Some(true), Some(0.0), Some(20.0))
    );
    // The Paintbrush keeps its own: the Pencil's change left it alone.
    app.run("tool.options", json!({"tool": "paintbrush"})).unwrap();
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!((d.f64("fidelity", 0.0), d.bool("fill")), (1.5, false));
    let pb_text = painted(&mut app);
    assert!(pb_text.contains("Paintbrush Tool Options"));
    assert!(!pb_text.contains("Alt key toggles to Smooth Tool") && !pb_text.contains("Live preview"), "{pb_text}");
    // Smooth has Fidelity only, the Eraser Size only, and the Blob Brush Size, Keep selected and Merge only with selection.
    app.run("tool.options", json!({"tool": "smooth"})).unwrap();
    let text = painted(&mut app);
    assert!(text.contains("Smooth Tool Options") && text.contains("Fidelity:"), "{text}");
    assert!(
        !text.contains("Fill new strokes")
            && !text.contains("Keep selected")
            && !text.contains("Alt key toggles to Smooth Tool")
            && !text.contains("Size:"),
        "{text}"
    );
    for tool in ["blobBrush", "eraser"] {
        app.run("tool.options", json!({"tool": tool})).unwrap();
        let text = painted(&mut app);
        assert!(text.contains("Size:") && !text.contains("Fidelity:"), "{tool}: {text}");
        // The Blob Brush also has Keep selected and Merge only with selection.
        let blob = tool == "blobBrush";
        assert_eq!((text.contains("Keep selected"), text.contains("Merge only with selection")), (blob, blob), "{tool}: {text}");
        // The Eraser also has Angle and Roundness.
        assert_eq!((text.contains("Angle:"), text.contains("Roundness:")), (!blob, !blob), "{tool}: {text}");
        app.ui.dialog.as_mut().unwrap().fields.insert("size".into(), json!(24));
        super::super::confirm(&mut app).unwrap();
        assert_eq!(app.session.tool_options_of(tool)["size"].as_f64(), Some(24.0), "{tool}");
    }
}

/// Live curve fitting (the Pencil's, on by default): on, the path shown while a stroke is
/// drawn is fitted; off, it joins the stroke's points with straight segments, and the release
/// fits it as with the option on.
#[test]
fn live_curve_fitting_off_shows_the_points_until_the_release_fits_them() {
    let mut app = app();
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
            doc.layers[0].children().unwrap().last().unwrap().path_data().unwrap().subpaths[0].anchors.clone()
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
    // On by default: the Pencil's dialog shows it and the Paintbrush's does not.
    app.run("tool.options", json!({"tool": "pencil"})).unwrap();
    assert!(painted(&mut app).contains("Live curve fitting"));
    assert!(app.ui.dialog.as_ref().unwrap().bool("liveFit"));
    super::super::confirm(&mut app).unwrap();
    let (drawing, on) = draw(&mut app, 100.0);
    assert!(!corners(&drawing) && drawing.len() < 20, "fitted while drawn: {} anchors", drawing.len());
    // Off: the points while the stroke is drawn, fitted at the release.
    app.run("tool.options", json!({"tool": "pencil"})).unwrap();
    app.ui.dialog.as_mut().unwrap().fields.insert("liveFit".into(), json!(false));
    super::super::confirm(&mut app).unwrap();
    app.select_tool("selection");
    assert_eq!(app.session.tool_options_of("pencil")["liveFit"].as_bool(), Some(false), "it lasts while another tool is chosen");
    let (drawing, off) = draw(&mut app, 200.0);
    assert!(corners(&drawing) && drawing.len() > 30, "the points while drawn: {} anchors", drawing.len());
    assert_eq!(off.len(), on.len(), "the release fits the stroke as with the option on");
    assert!(off.iter().zip(&on).all(|(a, b)| (a.p.x - b.p.x).abs() < 1e-9 && (a.p.y - b.p.y - 100.0).abs() < 1e-9), "{off:?}");
    app.run("tool.options", json!({"tool": "paintbrush"})).unwrap();
    assert!(!painted(&mut app).contains("Live curve fitting"));
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
    app.run("tool.options", json!({"tool": "pencil"})).unwrap();
    app.ui.dialog.as_mut().unwrap().fields.insert("fidelity".into(), json!(12));
    super::super::confirm(&mut app).unwrap();
    let smooth = anchors(&mut app, 200.0);
    assert!(smooth < precise, "{smooth} anchors at fidelity 12, {precise} at the default");
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
        app.run("tool.options", json!({"tool": tool})).unwrap();
        assert!(painted(&mut app).contains("Keep selected"), "{tool}");
        let d = app.ui.dialog.as_mut().unwrap();
        assert!(d.bool("keepSelected"), "{tool}");
        super::super::confirm(&mut app).unwrap();
        let (selected, id) = stroke(&mut app, tool, 100.0);
        assert_eq!(selected, vec![id], "{tool}");
        // Off: the next stroke leaves the selection as it was.
        app.run("tool.options", json!({"tool": tool})).unwrap();
        app.ui.dialog.as_mut().unwrap().fields.insert("keepSelected".into(), json!(false));
        super::super::confirm(&mut app).unwrap();
        // It lasts while another tool is chosen.
        app.select_tool("selection");
        assert_eq!(app.session.tool_options_of(tool)["keepSelected"].as_bool(), Some(false), "{tool}");
        let (selected, new) = stroke(&mut app, tool, 200.0);
        assert_ne!(new, id, "{tool} drew a new path");
        assert_eq!(selected, vec![id], "{tool}");
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
    app.run("tool.options", json!({"tool": "blobBrush"})).unwrap();
    let d = app.ui.dialog.as_mut().unwrap();
    // Both are off by default.
    assert!(!d.bool("keepSelected") && !d.bool("mergeOnlySelection"));
    d.fields.insert("keepSelected".into(), json!(true));
    d.fields.insert("mergeOnlySelection".into(), json!(true));
    super::super::confirm(&mut app).unwrap();
    let o = app.session.tool_options_of("blobBrush");
    assert_eq!((o["keepSelected"].as_bool(), o["mergeOnlySelection"].as_bool()), (Some(true), Some(true)));
    // With `b` selected, the stroke merges into `b` alone, and leaves it selected.
    app.run("select.set", json!({"ids": [b.0]})).unwrap();
    assert_eq!(stroke(&mut app), vec![b]);
    assert_eq!(ids(&app), vec![a, b], "merged into b only");
}

/// Eraser Tool Options' Angle and Roundness shape the tip the next Eraser drag erases with,
/// and the tool keeps them while another tool is chosen.
#[test]
fn the_eraser_options_set_the_tips_angle_and_roundness() {
    let mut app = app();
    // The width of the gap a vertical Eraser drag cuts through the middle of a 100 pt square.
    let erase = |app: &mut VectorcraftApp| {
        app.run("select.set", json!({"ids": []})).unwrap();
        let r = app.run("shape.rectangle", json!({"x": 100, "y": 100, "width": 100, "height": 100})).unwrap();
        let id = r["id"].as_u64().unwrap();
        app.run("select.set", json!({"ids": [id]})).unwrap();
        app.select_tool("eraser");
        let view = app.view_info();
        let pointer = |kind, y| vectorcraft_tools::PointerEvent::new(kind, 150.0, y);
        crate::canvas::dispatch(app, &pointer(vectorcraft_tools::PointerKind::Down, 50.0), view);
        for y in [100.0, 150.0, 200.0] {
            crate::canvas::dispatch(app, &pointer(vectorcraft_tools::PointerKind::Drag, y), view);
        }
        crate::canvas::dispatch(app, &pointer(vectorcraft_tools::PointerKind::Up, 250.0), view);
        let st = app.session.doc().unwrap();
        let bounds: Vec<_> = st.selection.objects.iter().filter_map(|id| st.doc.node(*id)?.path_data()?.control_bounds()).collect();
        let [a, b] = bounds[..] else { panic!("two halves: {bounds:?}") };
        let (l, r) = if a.x0 < b.x0 { (a, b) } else { (b, a) };
        r.x0 - l.x1
    };
    app.run("tool.options", json!({"tool": "eraser"})).unwrap();
    let d = app.ui.dialog.as_mut().unwrap();
    // A round tip by default.
    assert_eq!((d.f64("angle", -1.0), d.f64("roundness", -1.0)), (0.0, 100.0));
    d.fields.insert("size".into(), json!(20));
    super::super::confirm(&mut app).unwrap();
    let round = erase(&mut app);
    assert!((round - 20.0).abs() < 0.2, "a 20 pt round tip cuts {round} pt");
    // Upright and a quarter as wide: the band it erases is 5 pt wide.
    app.run("tool.options", json!({"tool": "eraser"})).unwrap();
    let d = app.ui.dialog.as_mut().unwrap();
    d.fields.insert("angle".into(), json!(90));
    d.fields.insert("roundness".into(), json!(25));
    super::super::confirm(&mut app).unwrap();
    app.select_tool("selection");
    let o = app.session.tool_options_of("eraser");
    assert_eq!((o["angle"].as_f64(), o["roundness"].as_f64(), o["size"].as_f64()), (Some(90.0), Some(25.0), Some(20.0)));
    let narrow = erase(&mut app);
    assert!((narrow - 5.0).abs() < 0.2, "a 20 by 5 pt upright tip cuts {narrow} pt");
}

#[test]
fn the_other_tools_have_no_freehand_options() {
    let mut app = app();
    assert!(open(&mut app, "pen").is_err());
    assert!(app.ui.dialog.is_none());
}
