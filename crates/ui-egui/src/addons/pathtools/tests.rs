use craft::ui::patterns::form_dialogs::{FieldValue, HostedParamDialog};
use serde_json::json;
use vectorcraft_engine::Session;
use vectorcraft_tools::addons::parametric_shape::ShapeKind;

use super::{ID, KIND, readout};
use crate::addons::dialogs::tests as host;
use crate::tests::panel_harness::Panel;
use crate::{VectorcraftApp, menus};

fn app() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
    app
}

/// An app with a window, which the dialogs open in.
fn app_with_window() -> VectorcraftApp {
    crate::testing::windowed(app())
}

#[test]
fn the_inspector_shows_the_selected_anchor_and_its_buttons_edit() {
    let mut app = app();
    let r = app.run("path.create", json!({"anchors": [{"x": 10, "y": 10}, {"x": 110, "y": 10}, {"x": 110, "y": 110}], "closed": false})).unwrap();
    let id = vectorcraft_doc::NodeId(r["id"].as_u64().unwrap());
    let none = readout(&app);
    assert_eq!((none.count, none.anchor.is_none()), (0, true), "nothing selected: the hint state");
    app.session
        .select(|_, sel| {
            sel.set([id]);
            sel.anchors.insert(id, [(0, 1)].into_iter().collect());
        })
        .unwrap();
    let origin = app.ruler_origin();
    let shown = readout(&app);
    assert_eq!(shown.count, 1);
    let a = shown.anchor.expect("the selected anchor");
    assert_eq!((a.x, a.y), (110.0 - origin.x, 10.0 - origin.y), "positions are less the ruler origin");
    assert_eq!((a.lengths, a.angles), ([0.0, 0.0], [None, None]), "a zero-length handle has no angle");
    // Make Smooth gives the anchor handles, which the readout then shows.
    app.run("addon.pathtools.smooth", json!({})).unwrap();
    let a = readout(&app).anchor.unwrap();
    assert!(a.lengths.iter().all(|l| *l > 0.0) && a.angles.iter().all(|d| d.is_some()), "{:?} {:?}", a.lengths, a.angles);
}

/// Runs `f` on the held options dialog, `None` when none is open.
fn held<R>(app: &VectorcraftApp, f: impl FnOnce(Option<&mut HostedParamDialog>) -> R) -> R {
    app.dialogs().expect("the window's dialogs").with(|h| f(h.get_mut::<HostedParamDialog>()))
}

#[test]
fn double_clicking_the_brush_opens_its_options_and_ok_sets_them() {
    let mut app = app_with_window();
    let ctx = host::ctx(&app);
    let numbers = |app: &VectorcraftApp| {
        held(app, |d| {
            let f = d.expect("the held dialog").fields();
            (f.number("size"), f.number("tolerance"), f.bool("pressure"))
        })
    };
    let ok = |app: &mut VectorcraftApp, set: &[(&str, FieldValue)]| {
        held(app, |d| {
            let d = d.expect("the held dialog");
            for (key, value) in set {
                d.set_field(*key, value.clone());
            }
        });
        app.dialogs().expect("the window's dialogs").with(|h| h.confirm::<HostedParamDialog>(&ctx));
        crate::dialogs::host::pump(app, &ctx);
        // A held OK closes at the dialog's next pass.
        crate::tests_synthetic::key_frame(app, &ctx, vec![]);
    };
    app.run("tool.options", json!({"tool": "anchorEraserBrush"})).unwrap();
    assert_eq!(numbers(&app), (Some(20.0), Some(10.0), Some(false)));
    ok(&mut app, &[("size", FieldValue::Number(64.0)), ("pressure", FieldValue::Bool(true))]);
    assert!(held(&app, |d| d.is_none()), "OK closes the dialog");
    let o = app.session.tool_options_of("anchorEraserBrush");
    assert_eq!((o["size"].as_f64(), o["pressure"].as_bool()), (Some(64.0), Some(true)));
    // The Extend Path tool's mode: a value the tool refuses keeps the dialog open and the option as it was.
    app.run("tool.options", json!({"tool": "extendPath"})).unwrap();
    ok(&mut app, &[("mode", FieldValue::Text("bogus".into()))]);
    assert!(held(&app, |d| d.is_some()), "a refusal keeps the dialog open");
    assert_eq!(app.session.tool_options_of("extendPath")["mode"], json!("straight"));
    ok(&mut app, &[("mode", FieldValue::Text("arc".into()))]);
    assert_eq!(app.session.tool_options_of("extendPath")["mode"], json!("arc"));
    // The Tangent Circle tool's radius for Shift, Ask For Circle Radius and Auto-Update Default.
    app.run("tool.options", json!({"tool": "tangentCircle"})).unwrap();
    let shown = held(&app, |d| {
        let f = d.expect("the held dialog").fields();
        (f.number("radius"), f.bool("askRadius"), f.bool("autoUpdateDefault"))
    });
    assert_eq!(shown, (Some(36.0), Some(false), Some(false)));
    ok(&mut app, &[("radius", FieldValue::Number(12.5)), ("askRadius", FieldValue::Bool(true))]);
    let o = app.session.tool_options_of("tangentCircle");
    assert_eq!((o["radius"].clone(), o["askRadius"].clone(), o["autoUpdateDefault"].clone()), (json!(12.5), json!(true), json!(false)));
    assert!(app.run("tool.options", json!({"tool": "measureInspect"})).is_err());
    assert_eq!(KIND, "pathToolOptions");
}

/// The Tangent Circle tool's options dialog shows the preview and snapping options, OK sets them,
/// and a value the tool refuses sets none of them and keeps the dialog open.
#[test]
fn the_tangent_circle_options_set_the_preview_and_snapping() {
    let mut app = app_with_window();
    let ctx = host::ctx(&app);
    let ok = |app: &mut VectorcraftApp, set: &[(&str, FieldValue)]| {
        held(app, |d| {
            let d = d.expect("the held dialog");
            for (key, value) in set {
                d.set_field(*key, value.clone());
            }
        });
        app.dialogs().expect("the window's dialogs").with(|h| h.confirm::<HostedParamDialog>(&ctx));
        crate::dialogs::host::pump(app, &ctx);
        crate::tests_synthetic::key_frame(app, &ctx, vec![]);
    };
    app.run("tool.options", json!({"tool": "tangentCircle"})).unwrap();
    let shown = held(&app, |d| {
        let f = d.expect("the held dialog").fields();
        (f.bool("center"), f.bool("radii"), f.text("color").map(str::to_owned), f.number("snapTolerance"), f.bool("snapWithoutGuides"))
    });
    assert_eq!(shown, (Some(true), Some(true), Some("red".to_owned()), Some(3.0), Some(true)));
    ok(
        &mut app,
        &[
            ("center", FieldValue::Bool(false)),
            ("radii", FieldValue::Bool(false)),
            ("color", FieldValue::Text("magenta".into())),
            ("snapTolerance", FieldValue::Number(9.0)),
            ("snapWithoutGuides", FieldValue::Bool(false)),
        ],
    );
    assert!(held(&app, |d| d.is_none()), "OK closes the dialog");
    let o = app.session.tool_options_of("tangentCircle");
    assert_eq!((&o["center"], &o["radii"], &o["color"], &o["snapWithoutGuides"]), (&json!(false), &json!(false), &json!("magenta"), &json!(false)));
    assert_eq!(o["snapTolerance"].as_f64(), Some(9.0));
    // A negative tolerance is refused: nothing changes and the dialog stays open.
    app.run("tool.options", json!({"tool": "tangentCircle"})).unwrap();
    ok(&mut app, &[("center", FieldValue::Bool(true)), ("snapTolerance", FieldValue::Number(-2.0))]);
    assert!(held(&app, |d| d.is_some()), "a refusal keeps the dialog open");
    let o = app.session.tool_options_of("tangentCircle");
    assert_eq!((&o["center"], o["snapTolerance"].as_f64()), (&json!(false), Some(9.0)));
}

/// The Tangent Circle tool's options dialog has a row for each option, a fractional snapping
/// tolerance opens as it is, and every color has a label, in the tool's order.
#[test]
fn the_tangent_circle_options_have_their_rows_and_a_fractional_tolerance() {
    use craft::ui::patterns::form_dialogs::ParamKind;
    let mut app = app_with_window();
    let params = super::options_params("tangentCircle", crate::canvas::rulers::general_lengths(&app));
    let rows: Vec<(&str, &str)> = params.iter().map(|p| (p.key.as_str(), p.label.as_str())).collect();
    assert_eq!(
        rows,
        [
            ("radius", "Radius"),
            ("center", "Circle Center"),
            ("radii", "Circle Radii Lines"),
            ("color", "Color"),
            ("snapTolerance", "Snap Tolerance"),
            ("snapWithoutGuides", "Snap When Smart Guides Are Off"),
            ("askRadius", "Ask For Circle Radius"),
            ("autoUpdateDefault", "Auto-Update Default"),
        ]
    );
    let colors = params.iter().find_map(|p| match &p.kind {
        ParamKind::Choice { options, .. } if p.key == "color" => {
            Some(options.iter().map(|(id, label)| (id.clone(), label.as_str().to_owned())).collect::<Vec<_>>())
        }
        _ => None,
    });
    let expected: Vec<(String, String)> = vectorcraft_tools::addons::tangent_circle::COLORS
        .iter()
        .zip(super::TANGENT_CIRCLE_COLORS)
        .map(|((name, _), label)| ((*name).to_owned(), label.to_owned()))
        .collect();
    assert_eq!(colors, Some(expected));
    for (name, _) in vectorcraft_tools::addons::tangent_circle::COLORS {
        assert!(super::TANGENT_CIRCLE_COLORS.iter().any(|label| label.to_lowercase() == *name), "{name} has a label");
    }
    app.run("tool.setOption", json!({"tool": "tangentCircle", "values": {"snapTolerance": 7.5}})).unwrap();
    app.run("tool.options", json!({"tool": "tangentCircle"})).unwrap();
    assert_eq!(held(&app, |d| d.expect("the held dialog").fields().number("snapTolerance")), Some(7.5));
}

#[test]
fn the_parametric_shape_panel_edits_the_selected_shape_or_the_tools_next_one() {
    let mut app = app();
    // Nothing selected: the tool's next shape.
    let (id, shape) = super::shape::target(&app);
    assert!(id.is_none() && shape.kind == ShapeKind::Polygon && !super::shape::plain_selected(&app));
    app.run("tool.setOption", json!({"tool": "parametricShape", "values": {"kind": "gear"}})).unwrap();
    assert!(super::shape::target(&app).1.kind == ShapeKind::Gear);
    // A shape selected: its own numbers.
    app.run("addon.pathtools.shape.create", json!({"kind": "bubble", "center": [150, 150]})).unwrap();
    let (id, shape) = super::shape::target(&app);
    assert!(id.is_some() && shape.kind == ShapeKind::Bubble && !super::shape::plain_selected(&app));
    // A plain object selected: the fallback, and Convert is on offer.
    let plain = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 40, "height": 40})).unwrap()["id"].as_u64().unwrap();
    app.run("select.set", json!({"ids": [plain]})).unwrap();
    let (id, _) = super::shape::target(&app);
    assert!(id.is_none() && super::shape::plain_selected(&app));
}

/// The panel's W and H keep the point of the selection's box that the Transform panel and the
/// Control bar share in place, as their own W and H do, on upright and turned shapes.
#[test]
fn the_parametric_shape_panels_size_fields_keep_the_shared_reference_point() {
    for (refi, rotation) in [(0usize, 0.0), (8, 0.0), (2, 30.0), (0, 180.0)] {
        let mut app = app();
        let params = json!({"kind": "rectangle", "center": [150, 150], "width": 100, "height": 60, "rotation": rotation});
        let id = vectorcraft_doc::NodeId(app.run("addon.pathtools.shape.create", params).unwrap()["id"].as_u64().unwrap());
        let shape =
            |app: &VectorcraftApp| vectorcraft_tools::addons::parametric_shape::read(app.session.active().unwrap().doc.node(id).unwrap()).unwrap();
        app.ui.transform_panel.reference_point = refi;
        let (before, was) = (shape(&app), app.selection_box().unwrap().reference_point(refi));
        let mut panel = Panel::new(app, super::shape::show);
        panel.type_into("W", "160");
        panel.type_into("H", "20");
        let mut app = panel.app();
        let after = shape(&app);
        assert!(after.width > before.width && after.height < before.height, "reference {refi}: W and H took {after:?}");
        let moved = app.selection_box().unwrap().reference_point(refi).distance(was);
        assert!(moved < 1e-6, "reference {refi}, rotation {rotation}: the reference point moved {moved} pt");
    }
    // With an anchor direct-selected the locator shows the anchors' box, so W keeps the shape's center.
    let mut app = app();
    let params = json!({"kind": "rectangle", "center": [150, 150], "width": 100, "height": 60});
    let id = vectorcraft_doc::NodeId(app.run("addon.pathtools.shape.create", params).unwrap()["id"].as_u64().unwrap());
    app.session
        .select(|_, sel| {
            sel.anchors.insert(id, [(0, 1)].into_iter().collect());
        })
        .unwrap();
    app.ui.transform_panel.reference_point = 0;
    let mut panel = Panel::new(app, super::shape::show);
    panel.type_into("W", "160");
    let app = panel.app();
    let after = vectorcraft_tools::addons::parametric_shape::read(app.session.active().unwrap().doc.node(id).unwrap()).unwrap();
    assert!((after.width - 160.0).abs() < 1e-6 && after.center.distance(vectorcraft_geom::Point::new(150.0, 150.0)) < 1e-6, "{after:?}");
}

/// The Corner Tool and Shared Edge Merge Tool items leave a floating Anchor Inspector showing
/// instead of closing it, and in Presentation Mode they choose the tool without failing.
#[test]
fn the_tool_items_keep_a_floating_anchor_inspector_showing() {
    let mut app = app();
    app.run("window.panel.float", json!({ "panel": ID })).unwrap();
    assert!(crate::panel_docking::visible(&app, ID));
    for command in [super::corner::COMMAND, super::shared_edge::COMMAND] {
        app.run(command, json!({})).unwrap();
        assert!(crate::panel_docking::visible(&app, ID), "{command} leaves the panel showing");
    }
    app.ui.screen_mode = 3;
    app.run(super::corner::COMMAND, json!({})).unwrap();
    assert_eq!(app.session.tool_id(), "cornerTool");
    app.ui.screen_mode = 0;
    assert!(crate::panel_docking::visible(&app, ID), "the panel shows when Presentation Mode ends");
}

#[test]
fn the_corner_section_counts_the_corners_apply_would_edit_and_applies_the_tools_values() {
    let mut app = app();
    let r = app
        .run("path.create", json!({"anchors": [{"x": 10, "y": 10}, {"x": 110, "y": 10}, {"x": 110, "y": 110}, {"x": 10, "y": 110}], "closed": true}))
        .unwrap();
    let id = vectorcraft_doc::NodeId(r["id"].as_u64().unwrap());
    assert_eq!(super::corner::selected_corners(&app), 4, "a path selected whole offers all its corners");
    app.run("tool.setOption", json!({"tool": "cornerTool", "values": {"radius": 12, "kind": "chamfer"}})).unwrap();
    app.session
        .select(|_, sel| {
            sel.set([id]);
            sel.anchors.insert(id, [(0, 1), (0, 2)].into_iter().collect());
        })
        .unwrap();
    assert_eq!(super::corner::selected_corners(&app), 2, "only the direct-selected corners");
    // What Apply runs: the tool's values on the selected corners, one undo step.
    let steps = app.session.doc().unwrap().history.undo.len();
    let options = app.session.tool_options_of("cornerTool");
    app.run("addon.pathtools.cornerSet", json!({"radius": options["radius"], "kind": options["kind"]})).unwrap();
    assert_eq!(app.session.doc().unwrap().history.undo.len(), steps + 1);
    assert_eq!(app.session.doc().unwrap().doc.node(id).unwrap().path_data().unwrap().anchor_count(), 6);
    assert_eq!(super::corner::selected_corners(&app), 2, "still selected");
}

#[test]
fn the_effect_choices_offer_the_effects_default_and_only_values_its_params_name() {
    for (effect, key) in [
        ("addon.path.addAnchors", "target"),
        ("addon.path.addAnchors", "spacing"),
        ("addon.path.extend", "startShape"),
        ("addon.path.extend", "endShape"),
        ("addon.path.close", "mode"),
        ("addon.path.retract", "handles"),
        ("addon.path.split", "anchors"),
    ] {
        let doc = vectorcraft_effects::effect_info(effect).unwrap();
        let options = super::effect_choices(effect, key).expect(key);
        let default = doc.defaults[key].as_str().unwrap();
        assert!(options.iter().any(|(_, value)| *value == default), "{effect} {key}");
        assert!(options.iter().all(|(_, value)| doc.params.contains(&format!("\"{value}\""))), "{effect} {key}");
    }
}

/// Common Tangents… on a point and an ellipse: every Tangents choice previews the same two lines
/// from the point, OK keeps them as one step, and a point inside the ellipse changes nothing and
/// says why.
#[test]
fn common_tangents_on_a_point_gives_the_same_lines_for_every_choice() {
    let mut app = app_with_window();
    let oval = app.run("shape.ellipse", json!({"x": 180.0, "y": 120.0, "width": 80.0, "height": 60.0})).unwrap()["id"].as_u64().unwrap();
    let point =
        |app: &mut VectorcraftApp, x: f64, y: f64| app.run("path.create", json!({"anchors": [{"x": x, "y": y}]})).unwrap()["id"].as_u64().unwrap();
    let dot = point(&mut app, 40.0, 150.0);
    app.run("select.set", json!({"ids": [oval, dot]})).unwrap();
    let steps = |app: &VectorcraftApp| app.session.doc().unwrap().history.undo.len();
    let count = |app: &VectorcraftApp| app.session.doc().unwrap().doc.node_count();
    // The end points of every two-anchor path in the document, rounded and sorted.
    let lines = |app: &VectorcraftApp| {
        let mut ends = Vec::new();
        app.session.doc().unwrap().doc.walk(|node| {
            if let Some(path) = node.path_data() {
                for sub in &path.subpaths {
                    if sub.anchors.len() == 2 {
                        let mut pair: Vec<(i64, i64)> =
                            sub.anchors.iter().map(|a| ((a.p.x * 1e6).round() as i64, (a.p.y * 1e6).round() as i64)).collect();
                        pair.sort();
                        ends.push(pair);
                    }
                }
            }
        });
        ends.sort();
        ends
    };
    let (start, n, nodes) = (app.session.doc().unwrap().doc.clone(), steps(&app), count(&app));
    let open = |app: &mut VectorcraftApp| {
        host::open_item(app, "Path Precision Tools", "Common Tangents…");
        host::dialog_id(app)
    };

    // Each choice previews the same two lines, and OK with that choice keeps them as one step;
    // undo puts the document back as it was.
    let mut first: Option<Vec<Vec<(i64, i64)>>> = None;
    for (_, kind) in super::TANGENT_KINDS {
        let id = open(&mut app);
        host::set(&app, id, "preview", json!(true));
        host::set(&app, id, "tangents", json!(kind));
        host::frame(&mut app);
        assert_eq!(count(&app), nodes + 2, "{kind}: the two lines from the point");
        let previewed = lines(&app);
        assert_eq!(previewed.len(), 2, "{kind}");
        let first = first.get_or_insert_with(|| previewed.clone());
        assert_eq!(&previewed, first, "{kind}: the same lines as the first choice");
        assert!(host::confirm(&app, id), "{kind}: OK is enabled");
        host::frame(&mut app);
        assert_eq!((steps(&app), count(&app)), (n + 1, nodes + 2), "{kind}: one undo step");
        assert_eq!(&lines(&app), first, "{kind}: OK keeps the previewed lines");
        app.run("edit.undo", json!({})).unwrap();
        app.run("select.set", json!({"ids": [oval, dot]})).unwrap();
        assert_eq!((steps(&app), count(&app)), (n, nodes), "{kind}: undone");
        assert_eq!(app.session.doc().unwrap().doc, start, "{kind}: undo restores the document");
    }
    let first = first.unwrap();

    // Switching the choice while the preview is on keeps the same two lines on the canvas.
    let id = open(&mut app);
    host::set(&app, id, "preview", json!(true));
    host::frame(&mut app);
    for (_, kind) in super::TANGENT_KINDS.iter().rev() {
        host::set(&app, id, "tangents", json!(kind));
        host::frame(&mut app);
        assert_eq!(count(&app), nodes + 2, "{kind}: switched in place");
        assert_eq!(lines(&app), first, "{kind}: the same lines after the switch");
    }
    host::cancel(&app, id);
    host::frame(&mut app);
    assert_eq!(app.session.doc().unwrap().doc, start, "Cancel takes the preview away");
    assert_eq!(steps(&app), n, "and adds no undo step");

    // A point inside the ellipse: nothing changes, and a message says why.
    let inside = point(&mut app, 220.0, 150.0);
    app.run("select.set", json!({"ids": [oval, inside]})).unwrap();
    let (n, nodes) = (steps(&app), count(&app));
    let id = open(&mut app);
    assert!(host::confirm(&app, id));
    host::frame(&mut app);
    assert_eq!((steps(&app), count(&app)), (n, nodes));
    assert!(app.ui.status.contains("The point lies inside the path"), "{}", app.ui.status);
}

/// A percentage typed alone into a size field (the Corner Tool's radius, a shape's W and Corner Radius, a handle's
/// Length) is that share of the size the field shows, in a unit other than the stored points too.
#[test]
fn a_lone_percentage_in_a_size_field_is_that_share_of_the_size() {
    let mut app = app();
    app.run("document.setUnits", json!({"units": "Millimeters"})).unwrap();
    app.run("tool.setOption", json!({"tool": "cornerTool", "values": {"radius": 12}})).unwrap();
    app.run("tool.setOption", json!({"tool": "parametricShape", "values": {"radius": 10}})).unwrap();
    let r = app.run("path.create", json!({"anchors": [{"x": 10, "y": 10}, {"x": 110, "y": 10}, {"x": 110, "y": 110}], "closed": false})).unwrap();
    let id = vectorcraft_doc::NodeId(r["id"].as_u64().unwrap());
    app.session
        .select(|_, sel| {
            sel.set([id]);
            sel.anchors.insert(id, [(0, 1)].into_iter().collect());
        })
        .unwrap();
    app.run("addon.pathtools.smooth", json!({})).unwrap();
    let shape = super::shape::target(&app).1;
    let length = readout(&app).anchor.unwrap().lengths[0];
    let halved = |before: f64, after: f64| (after - before / 2.0).abs() < 1e-6;

    // The three panels, each in its own id scope, as their docked panels are.
    let mut panel = Panel::new(app, |app, ui| {
        ui.push_id("corner", |ui| super::corner::section(app, ui));
        ui.push_id("shape", |ui| super::shape::show(app, ui));
        ui.push_id("inspector", |ui| super::show(app, ui));
    });
    for field in ["Radius", "W", "Corner Radius", "Length"] {
        panel.type_into(field, "50%");
    }
    let app = panel.app();
    let radius = app.session.tool_options_of("cornerTool")["radius"].as_f64().unwrap();
    assert!(halved(12.0, radius), "the Corner Tool's radius is {radius}");
    let typed = super::shape::target(&app).1;
    assert!(halved(shape.width, typed.width), "W went from {} to {}", shape.width, typed.width);
    assert!(halved(shape.radius, typed.radius), "Corner Radius went from {} to {}", shape.radius, typed.radius);
    let typed = readout(&app).anchor.unwrap().lengths[0];
    assert!(halved(length, typed), "the handle's Length went from {length} to {typed}");
}

/// Preferences › Units › Numbers Without Units Are Points reaches a panel's length field as it does a command
/// dialog's: in a document in picas, `12` typed alone into the Corner Tool's Radius is 12 points while it is on and
/// 12 picas while it is off.
#[test]
fn a_bare_number_in_a_panels_picas_field_follows_numbers_without_units_are_points() {
    let radius_typed = |on: bool| {
        let mut app = app();
        app.run("document.setUnits", json!({"units": "Picas"})).unwrap();
        app.run("prefs.set", json!({"key": "numbersWithoutUnitsArePoints", "value": on})).unwrap();
        let mut panel = Panel::new(app, super::corner::section);
        panel.type_into("Radius", "12");
        let app = panel.app();
        app.session.tool_options_of("cornerTool")["radius"].as_f64().unwrap()
    };
    let (points, picas) = (radius_typed(true), radius_typed(false));
    assert!((points - 12.0).abs() < 1e-6, "on: {points} pt");
    assert!((picas - 144.0).abs() < 1e-6, "off: {picas} pt");
}

#[test]
fn add_inscribed_circle_draws_its_circles_without_a_dialog() {
    let mut app = app_with_window();
    let find = |app: &VectorcraftApp| {
        let entries = menus::menu_entries(app);
        entries
            .into_iter()
            .find(|e| e.path == ["Object", "Plug-ins", "Path Precision Tools"] && e.label == "Add Inscribed Circle")
            .expect("Add Inscribed Circle")
    };
    let item = find(&app);
    assert_eq!((item.command.as_deref(), &item.params), (Some(super::INSCRIBED_CIRCLE), &json!({})));
    assert!(!item.enabled, "with nothing selected");
    let a = app.run("shape.rectangle", json!({"x": 10, "y": 20, "width": 30, "height": 40})).unwrap()["id"].as_u64().unwrap();
    let b = app.run("shape.rectangle", json!({"x": 100, "y": 20, "width": 20, "height": 40})).unwrap()["id"].as_u64().unwrap();
    app.run("select.set", json!({"ids": [a, b]})).unwrap();
    let (steps, nodes) = (app.session.doc().unwrap().history.undo.len(), app.session.doc().unwrap().doc.node_count());
    let item = find(&app);
    assert!(item.enabled);
    let (cmd, p) = menus::click_target(&item.label, item.command.as_deref().unwrap(), &item.params);
    menus::invoke(&mut app, &cmd, p);
    assert!(held(&app, |d| d.is_none()), "no dialog opens");
    let st = app.session.doc().unwrap();
    assert_eq!((st.history.undo.len(), st.doc.node_count()), (steps + 1, nodes + 2), "a circle in each rectangle, one undo step");
    let made = &st.selection.objects;
    assert_eq!(made.len(), 2, "the two circles are selected: {made:?}");
    for (id, inside) in made.iter().zip([[10.0, 20.0, 40.0, 60.0], [100.0, 20.0, 120.0, 60.0]]) {
        assert!(id.0 != a && id.0 != b, "{id:?} is a new object");
        let live = match &st.doc.node(*id).unwrap().kind {
            vectorcraft_doc::NodeKind::Path { live, .. } => live.as_deref(),
            _ => None,
        };
        assert!(matches!(live, Some(vectorcraft_doc::LiveShape::Ellipse { w, h, .. }) if w == h), "{id:?} is a live circle");
        let r = st.doc.bounds_of(&[*id], false).unwrap();
        assert!(r.x0 >= inside[0] && r.y0 >= inside[1] && r.x1 <= inside[2] && r.y1 <= inside[3], "{r:?} inside {inside:?}");
    }
}
