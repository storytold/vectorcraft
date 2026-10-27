use super::*;
use vectorcraft_engine::Session;

fn app() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
    app
}

fn set(app: &mut VectorcraftApp, fields: Value) {
    let d = app.ui.dialog.as_mut().unwrap();
    for (k, v) in fields.as_object().unwrap() {
        d.fields.insert(k.clone(), v.clone());
    }
}

fn shown(app: &mut VectorcraftApp) -> String {
    crate::tests_labels::painted_text(app, |app, ui| super::super::show(app, ui.ctx()))
}

#[test]
fn calligraphic_options_show_each_value_with_its_variation_and_ok_applies_them() {
    let mut app = app();
    app.run("ui.brushOptions", json!({"name": "6 pt. Flat"})).unwrap();
    let text = shown(&mut app);
    for label in ["Calligraphic Brush Options", "Name:", "Angle:", "Roundness:", "Size:", "Variation:", "Fixed", "OK", "Cancel"] {
        assert!(text.contains(label), "{label} in {text}");
    }
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!((d.f64("angle", 0.0), d.f64("roundness", 0.0), d.f64("size", 0.0)), (45.0, 15.0, 6.0));
    // Out-of-range values are kept within their ranges.
    set(&mut app, json!({"angleMode": "random", "angleVariation": 500, "roundness": 140, "sizeMode": "pressure", "sizeVariation": 4}));
    assert!(shown(&mut app).contains("Pressure"));
    super::super::confirm(&mut app).unwrap();
    let def = app.run("brush.get", json!({"name": "6 pt. Flat"})).unwrap();
    assert_eq!(def["modes"], json!(["random", "fixed", "pressure"]));
    assert_eq!((def["variation"].clone(), def["roundness"].clone()), (json!([180.0, 0.0, 4.0]), json!(100.0)));
    // Opened again, it shows what was set.
    app.run("ui.brushOptions", json!({"name": "6 pt. Flat"})).unwrap();
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!((d.str("angleMode"), d.f64("sizeVariation", 0.0)), ("random".into(), 4.0));
}

#[test]
fn a_taken_name_keeps_the_dialog_open() {
    let mut app = app();
    app.run("ui.brushOptions", json!({"name": "6 pt. Flat"})).unwrap();
    set(&mut app, json!({"name": "3 pt. Round"}));
    assert!(super::super::confirm(&mut app).is_err());
    assert!(app.ui.dialog.is_some());
}

#[test]
fn bristle_options_and_an_unknown_brush() {
    let mut app = app();
    app.run("ui.brushOptions", json!({"name": "Bristle Round"})).unwrap();
    let text = shown(&mut app);
    for label in ["Bristle Brush Options", "Shape:", "Bristle Length:", "Stiffness:", "Paint Opacity:"] {
        assert!(text.contains(label), "{label} in {text}");
    }
    set(&mut app, json!({"shape": "flatFan", "density": 0, "length": 120}));
    super::super::confirm(&mut app).unwrap();
    let def = app.run("brush.get", json!({"name": "Bristle Round"})).unwrap();
    assert_eq!((def["shape"].clone(), def["density"].clone(), def["length"].clone()), (json!("flatFan"), json!(1.0), json!(120.0)));
    // No such brush: an error, nothing opens.
    app.ui.dialog = None;
    assert!(app.run("ui.brushOptions", json!({"name": "Nope"})).is_err());
    assert!(app.ui.dialog.is_none());
}

/// Bristle Brush Options' Name field takes at most 31 characters as they are typed; the
/// Calligraphic Brush Options' Name field takes more.
#[test]
fn bristle_brush_options_name_field_takes_at_most_31_characters() {
    let mut app = app();
    let typed = "é".repeat(40);
    for (brush, kept) in [("Bristle Round", 31), ("6 pt. Flat", 40)] {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let frame = |app: &mut VectorcraftApp, events: Vec<egui::Event>| {
            let mut out = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| super::super::show(app, ui.ctx()));
            out.textures_delta.clear();
        };
        app.run("ui.brushOptions", json!({ "name": brush })).unwrap();
        // The Name field takes the focus with its text selected; typing replaces it.
        frame(&mut app, vec![]);
        frame(&mut app, vec![]);
        frame(&mut app, vec![egui::Event::Text(typed.clone())]);
        assert_eq!(app.ui.dialog.as_ref().unwrap().str("name"), "é".repeat(kept), "{brush}");
        super::super::confirm(&mut app).unwrap();
        assert!(app.run("brush.get", json!({ "name": "é".repeat(kept) })).is_ok(), "{brush}");
    }
    // A new name of more than 31 characters in the field (a longer saved name with some of its
    // characters deleted) is refused, and the dialog stays open.
    app.run("ui.brushOptions", json!({ "name": "é".repeat(31) })).unwrap();
    set(&mut app, json!({ "name": "é".repeat(35) }));
    assert!(super::super::confirm(&mut app).is_err());
    assert!(app.ui.dialog.is_some());
}

/// A path painted with brush `name`, and the height of the art its stroke draws.
fn brushed(app: &mut VectorcraftApp, name: &str) -> (vectorcraft_doc::NodeId, f64) {
    let id = app.run("shape.line", json!({"x1": 20, "y1": 100, "x2": 280, "y2": 100})).unwrap()["id"].as_u64().unwrap();
    let id = vectorcraft_doc::NodeId(id);
    app.run("brush.apply", json!({ "name": name, "ids": [id.0] })).unwrap();
    (id, drawn_height(app, id))
}

fn drawn_height(app: &VectorcraftApp, id: vectorcraft_doc::NodeId) -> f64 {
    let st = app.session.active().unwrap();
    let pieces = vectorcraft_brush::node_pieces(&st.doc, st.doc.node(id).unwrap()).unwrap();
    vectorcraft_brush::pieces_bounds(&pieces).unwrap().height()
}

#[test]
fn scatter_options_show_each_value_with_its_mode_and_ok_applies_them() {
    let mut app = app();
    let (id, before) = brushed(&mut app, "Confetti");
    app.run("ui.brushOptions", json!({"name": "Confetti"})).unwrap();
    let text = shown(&mut app);
    for label in [
        "Scatter Brush Options",
        "Name:",
        "Size:",
        "Spacing:",
        "Scatter:",
        "Rotation:",
        "Random",
        "Rotation relative to:",
        "Page",
        "Colorization",
        "Method:",
        "Tints",
        "Key Color:",
    ] {
        assert!(text.contains(label), "{label} in {text}");
    }
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!((d.f64("sizeMin", 0.0), d.f64("sizeMax", 0.0), d.str("sizeMode")), (60.0, 140.0, "random".into()));
    assert_eq!((d.str("rotationRelativeTo"), d.str("colorization")), ("page".into(), "tints".into()));
    // Values are kept within their ranges (rotation here); Fixed takes the first value.
    set(
        &mut app,
        json!({"sizeMode": "fixed", "sizeMin": 5000, "spacingMode": "pressure", "rotationMin": -400, "rotationRelativeTo": "path", "colorization": "hueShift"}),
    );
    assert!(shown(&mut app).contains("Hue Shift"));
    super::super::confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none());
    let def = app.run("brush.get", json!({"name": "Confetti"})).unwrap();
    assert_eq!(
        (&def["size"], &def["rotation"][0], &def["modes"]),
        (&json!([5000.0, 140.0]), &json!(-180.0), &json!(["fixed", "pressure", "random", "random"]))
    );
    assert_eq!((&def["rotation_relative_to_path"], &def["colorization"]["method"]), (&json!(true), &json!("hueShift")));
    assert!(def["colorization"]["key"].is_object(), "the art's colour as the key: {def}");
    assert!(drawn_height(&app, id) > before, "the stroke redraws with the bigger confetti");
    // Opened again, it shows what was set.
    app.run("ui.brushOptions", json!({"name": "Confetti"})).unwrap();
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!((d.str("sizeMode"), d.str("spacingMode"), d.str("colorization")), ("fixed".into(), "pressure".into(), "hueShift".into()));
}

#[test]
fn brush_key_color_eyedropper_updates_the_open_dialog() {
    use vectorcraft_tools::{PointerEvent, PointerKind};
    let mut app = app();
    app.run("paint.setFill", json!({"color": "#336699"})).unwrap();
    app.run("shape.rectangle", json!({"x": 40, "y": 40, "width": 80, "height": 60})).unwrap();
    app.run("ui.brushOptions", json!({"name": "Confetti"})).unwrap();
    set(&mut app, json!({"colorization": "hueShift"}));
    let back = app.session.tool_id();
    app.select_tool("eyedropper");
    app.session.set_tool_option("brushKeyColor", &json!(true));
    app.session.set_tool_option("stop", &json!(back));
    let view = app.view_info();
    crate::canvas::dispatch(&mut app, &PointerEvent::new(PointerKind::Down, 80.0, 70.0), view);
    let key = app.ui.dialog.as_ref().unwrap().fields.get("keyColor").cloned().unwrap();
    assert_eq!(vectorcraft_engine::cmd::color_value(&key).unwrap().to_hex(), "#336699");
    assert_eq!(app.session.tool_id(), back);
}

#[test]
fn art_options_set_width_scale_direction_flips_and_colorization() {
    let mut app = app();
    let (id, before) = brushed(&mut app, "Arrow");
    app.run("ui.brushOptions", json!({"name": "Arrow"})).unwrap();
    let text = shown(&mut app);
    for label in [
        "Art Brush Options",
        "Width:",
        "Brush Scale Options",
        "Scale Proportionately",
        "Stretch to Fit Stroke Length",
        "Stretch Between Guides",
        "Start:",
        "End:",
        "Direction:",
        "Flip Along",
        "Flip Across",
        "Colorization",
    ] {
        assert!(text.contains(label), "{label} in {text}");
    }
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!(
        (d.f64("width", 0.0), d.str("scaleMode"), d.f64("guideEnd", 0.0), d.str("direction")),
        (100.0, "betweenGuides".into(), 78.0, "leftToRight".into())
    );
    set(&mut app, json!({"width": 250, "guideStart": 10, "guideEnd": 140, "direction": "bottomToTop", "flipAcross": true, "colorization": "none"}));
    super::super::confirm(&mut app).unwrap();
    let def = app.run("brush.get", json!({"name": "Arrow"})).unwrap();
    assert_eq!(
        (&def["width"], &def["scale"], &def["direction"]),
        (&json!(250.0), &json!({"mode": "betweenGuides", "start": 0.1, "end": 1.0}), &json!("bottomToTop"))
    );
    assert_eq!((&def["flip_across"], &def["flip_along"], &def["colorization"]), (&json!(true), &json!(false), &json!({"method": "none"})));
    assert!(drawn_height(&app, id) != before, "the stroke redraws");
    // Stretch to Fit drops the guides.
    app.run("ui.brushOptions", json!({"name": "Arrow"})).unwrap();
    set(&mut app, json!({"scaleMode": "stretch"}));
    super::super::confirm(&mut app).unwrap();
    assert_eq!(app.run("brush.get", json!({"name": "Arrow"})).unwrap()["scale"], json!({"mode": "stretch"}));
}

/// Art Brush Options' Width menu: Pressure takes a second width for the heaviest pen pressure,
/// and a stroke drawn without pressure is half way between the two; Fixed drops the second width.
#[test]
fn art_options_width_follows_the_pen_pressure() {
    let mut app = app();
    let (id, before) = brushed(&mut app, "Tapered Stroke");
    app.run("ui.brushOptions", json!({"name": "Tapered Stroke"})).unwrap();
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!((d.str("widthMode"), d.f64("widthMax", 0.0)), ("fixed".into(), 100.0));
    assert!(shown(&mut app).contains("Fixed"));
    set(&mut app, json!({"widthMode": "pressure", "widthMax": 300}));
    assert!(shown(&mut app).contains("Pressure"));
    super::super::confirm(&mut app).unwrap();
    let def = app.run("brush.get", json!({"name": "Tapered Stroke"})).unwrap();
    assert_eq!((&def["width"], &def["width_mode"], &def["width_max"]), (&json!(100.0), &json!("pressure"), &json!(300.0)));
    let after = drawn_height(&app, id);
    assert!((after - 2.0 * before).abs() < 0.01 * before, "200 % half way: {before} then {after}");
    // Opened again, it shows what was set; Fixed drops the second width.
    app.run("ui.brushOptions", json!({"name": "Tapered Stroke"})).unwrap();
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!((d.str("widthMode"), d.f64("widthMax", 0.0)), ("pressure".into(), 300.0));
    set(&mut app, json!({"widthMode": "fixed"}));
    super::super::confirm(&mut app).unwrap();
    let def = app.run("brush.get", json!({"name": "Tapered Stroke"})).unwrap();
    assert!(def.get("width_mode").is_none() && def.get("width_max").is_none(), "{def}");
    assert!((drawn_height(&app, id) - before).abs() < 1e-6);
}

/// A context with the app's fonts and theme, for [`frame`].
fn ctx() -> egui::Context {
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    crate::theme::apply(&ctx, Default::default());
    ctx
}

/// Runs one 1200 × 900 frame with `events` through the app's key handling and the dialogs, and
/// returns the center of the text `label` if the frame shows it.
fn frame(ctx: &egui::Context, app: &mut VectorcraftApp, events: Vec<egui::Event>, label: &str) -> Option<egui::Pos2> {
    use egui::{Pos2, Rect};
    let raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1200.0, 900.0))), events, ..Default::default() };
    let mut out = ctx.run_ui(raw, |ui| {
        crate::shortcuts::handle(app, ui.ctx());
        super::super::show(app, ui.ctx());
    });
    out.textures_delta.clear();
    out.shapes.iter().find_map(|s| match &s.shape {
        egui::Shape::Text(t) if t.galley.text() == label => Some(egui::Rect::from_min_size(t.pos, t.galley.size()).center()),
        _ => None,
    })
}

/// Clicks the text `label` the open dialog shows.
fn click_text(ctx: &egui::Context, app: &mut VectorcraftApp, label: &str) {
    use egui::{Event, PointerButton};
    frame(ctx, app, vec![], label);
    let at = frame(ctx, app, vec![], label).unwrap_or_else(|| panic!("no {label:?} shown"));
    let button = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() };
    frame(ctx, app, vec![Event::PointerMoved(at), button(true)], label);
    frame(ctx, app, vec![button(false)], label);
    frame(ctx, app, vec![], label);
}

/// Presses Escape.
fn escape(ctx: &egui::Context, app: &mut VectorcraftApp) {
    let key = egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() };
    frame(ctx, app, vec![key], "");
    frame(ctx, app, vec![], "");
}

/// Scatter, Art and Pattern Brush Options have a Tips button beside Method. A click on it opens
/// Colorization Tips, which names each method and what it does beside a sample of the brush,
/// and a second click closes it. Screen readers name the button, the popover and each sample.
#[test]
fn tips_names_each_colorization_method_beside_a_sample() {
    use crate::widgets::accessibility_tests::{role_and_state, tree};
    use egui::accesskit::Role;
    let mut app = app();
    for name in ["Confetti", "Arrow", "Chain"] {
        app.run("ui.brushOptions", json!({ "name": name })).unwrap();
        let text = shown(&mut app);
        assert!(text.lines().any(|l| l == "Tips") && !text.contains("Colorization Tips"), "{name}: {text}");
    }
    app.run("ui.brushOptions", json!({"name": "Confetti"})).unwrap();
    let ctx = ctx();
    click_text(&ctx, &mut app, "Tips");
    assert!(app.ui.dialog.as_ref().unwrap().bool("tips"), "Tips opens Colorization Tips");
    let text = shown(&mut app);
    for label in [
        "Colorization Tips",
        "The samples show this brush on a blue stroke.",
        "None",
        "The art keeps its own colors.",
        "Tints",
        "Black becomes the stroke color and white stays white. Other colors become tints of the stroke color by their lightness.",
        "Tints and Shades",
        "Hue Shift",
        "The key color becomes the stroke color. The other colors shift their hue by the same amount and keep their saturation and brightness.",
        "Black and white stay as they are. Dark colors become shades of the stroke color and light colors become tints of it.",
    ] {
        assert!(text.contains(label), "{label} in {text}");
    }
    let nodes = tree(|ui| super::super::show(&mut app, ui.ctx()));
    assert_eq!(role_and_state(&nodes, "Tips").0, Role::Button);
    assert_eq!(role_and_state(&nodes, "Colorization Tips").0, Role::Dialog);
    for method in ["None", "Tints", "Tints and Shades", "Hue Shift"] {
        assert!(nodes.iter().any(|n| n.role() == Role::Image && n.label() == Some(method)), "a sample named {method}");
    }
    click_text(&ctx, &mut app, "Tips");
    assert!(!app.ui.dialog.as_ref().unwrap().bool("tips"), "a second click closes it");
    assert!(!shown(&mut app).contains("Colorization Tips"));
    // `ui.dialog.set {field: "tips", value: true}` opens it too.
    set(&mut app, json!({"tips": true}));
    assert!(shown(&mut app).contains("Colorization Tips"));
}

/// Escape with Colorization Tips or the Method list open closes only that: Brush Options stays
/// open with its edits, and the next Escape cancels it.
#[test]
fn escape_closes_tips_or_the_method_list_and_leaves_the_dialog_open() {
    let (mut app, ctx) = (app(), ctx());
    app.run("ui.brushOptions", json!({"name": "Confetti"})).unwrap();
    set(&mut app, json!({"colorization": "tints"}));
    click_text(&ctx, &mut app, "Tips");
    assert!(app.ui.dialog.as_ref().unwrap().bool("tips"));
    escape(&ctx, &mut app);
    let d = app.ui.dialog.as_ref().expect("Escape closes Tips, not Brush Options");
    assert!(!d.bool("tips") && d.str("colorization") == "tints");
    assert!(frame(&ctx, &mut app, vec![], "Colorization Tips").is_none());
    click_text(&ctx, &mut app, "Tints");
    assert!(egui::Popup::is_any_open(&ctx), "the Method list is open");
    escape(&ctx, &mut app);
    assert!(app.ui.dialog.as_ref().is_some_and(|d| d.str("colorization") == "tints"), "Escape closes the list, not Brush Options");
    assert!(!egui::Popup::is_any_open(&ctx));
    escape(&ctx, &mut app);
    assert!(app.ui.dialog.is_none(), "the next Escape cancels Brush Options");
    // Calligraphic Brush Options has no Tips: with `tips` set, Escape still cancels it.
    app.run("ui.brushOptions", json!({"name": "6 pt. Flat"})).unwrap();
    set(&mut app, json!({"tips": true}));
    escape(&ctx, &mut app);
    assert!(app.ui.dialog.is_none(), "Escape cancels Calligraphic Brush Options");
}

/// Each Colorization Tips sample paints the brush with its own method on the blue stroke:
/// Confetti's black and gray art keeps its colors with None, its black becomes the stroke color
/// with Tints and with Hue Shift (black is its key color), and stays black with Tints and Shades.
#[test]
fn each_tips_sample_paints_the_brush_with_its_method() {
    let mut app = app();
    app.run("ui.brushOptions", json!({"name": "Confetti"})).unwrap();
    let d = app.ui.dialog.as_ref().unwrap();
    let key: vectorcraft_color::Color = serde_json::from_value(d.fields["keyColor"].clone()).unwrap();
    let colors = |id: &str| -> Vec<String> {
        let method = vectorcraft_brush::Colorization::parse(id, key).unwrap();
        let doc = brush_art_options::tip_doc(&brush_art_options::edited_def(d), "scatter", method, 120.0, 44.0).unwrap();
        let mut hex: Vec<String> = vectorcraft_brush::art_colors(doc.layers.iter().map(|l| l.as_ref())).iter().map(|c| c.to_hex()).collect();
        hex.sort();
        hex
    };
    let (black, blue) = ("#000000".to_string(), brush_art_options::TIP_STROKE.to_hex());
    let confetti = vectorcraft_brush::defaults().iter().find(|b| b.name == "Confetti").unwrap();
    let art: Vec<String> = vectorcraft_brush::art_colors(confetti.art()).iter().map(|c| c.to_hex()).collect();
    let none = colors("none");
    assert!(none.len() == 2 && art.iter().all(|c| none.contains(c)), "None keeps the art's colors {art:?}: {none:?}");
    let (tints, shades, hue) = (colors("tints"), colors("tintsAndShades"), colors("hueShift"));
    assert!(tints.contains(&blue) && !tints.contains(&black), "{tints:?}");
    assert!(shades.contains(&black) && !shades.contains(&blue), "{shades:?}");
    assert!(hue.contains(&blue) && !hue.contains(&black), "{hue:?}");
    assert!(tints != hue && tints != shades && none != shades, "four different samples");
}

/// `ui.dialog.set` writes any field, `__def` included. With a brush definition that is not
/// an object, Brush Options and Colorization Tips still draw.
#[test]
fn a_brush_definition_that_is_not_an_object_still_draws() {
    let (mut app, ctx) = (app(), ctx());
    app.run("ui.brushOptions", json!({"name": "Confetti"})).unwrap();
    crate::tests_synthetic::control(&mut app, &ctx, "ui.dialog.set", json!({"field": "__def", "value": 5}));
    crate::tests_synthetic::control(&mut app, &ctx, "ui.dialog.set", json!({"field": "tips", "value": true}));
    let text = shown(&mut app);
    assert!(text.contains("Scatter Brush Options") && text.contains("Colorization Tips"), "{text}");
}

#[test]
fn pattern_options_show_the_tiles_and_set_scale_spacing_fit() {
    let mut app = app();
    app.run("ui.brushOptions", json!({"name": "Chain"})).unwrap();
    let text = shown(&mut app);
    for label in [
        "Pattern Brush Options",
        "Scale:",
        "Spacing:",
        "Outer Corner Tile",
        "Side Tile",
        "Inner Corner Tile",
        "Start Tile",
        "End Tile",
        "None",
        "Fit",
        "Stretch to fit",
        "Add space to fit",
        "Approximate path",
        "Flip Along",
    ] {
        assert!(text.contains(label), "{label} in {text}");
    }
    let d = app.ui.dialog.as_ref().unwrap();
    let mut tiles: Vec<&String> = d.fields["__tiles"].as_object().unwrap().keys().collect();
    tiles.sort();
    assert_eq!(tiles, ["innerCorner", "outerCorner", "side"], "Chain has side and corner tiles, no start or end");
    // Diamonds (straight-edged tiles, so sizes compare exactly).
    let (id, before) = brushed(&mut app, "Diamonds");
    app.run("ui.brushOptions", json!({"name": "Diamonds"})).unwrap();
    assert_eq!((app.ui.dialog.as_ref().unwrap().f64("spacing", 0.0), app.ui.dialog.as_ref().unwrap().str("fit")), (25.0, "addSpace".into()));
    set(&mut app, json!({"scale": 200, "spacing": 30, "fit": "approximate", "flipAlong": true}));
    super::super::confirm(&mut app).unwrap();
    let def = app.run("brush.get", json!({"name": "Diamonds"})).unwrap();
    assert_eq!((&def["scale"], &def["spacing"], &def["fit"], &def["flip_along"]), (&json!(200.0), &json!(30.0), &json!("approximate"), &json!(true)));
    assert!((drawn_height(&app, id) / before - 2.0).abs() < 1e-6, "tiles twice the size");
}

/// Art Brush Options' Width and Pattern Brush Options' Scale each have a slider beside their
/// field, which screen readers name as the row, with the field's number and text over the
/// value's whole range.
#[test]
fn art_width_and_pattern_scale_have_sliders() {
    use crate::widgets::accessibility_tests::{slider_value, tree};
    let mut app = app();
    app.run("ui.brushOptions", json!({"name": "Arrow"})).unwrap();
    set(&mut app, json!({"width": 250}));
    let shown = tree(|ui| super::super::show(&mut app, ui.ctx()));
    assert_eq!(slider_value(&shown, "Width:"), (Some(250.0), Some(1.0), Some(1000.0), Some("250%".into())));
    app.ui.dialog = None;
    app.run("ui.brushOptions", json!({"name": "Diamonds"})).unwrap();
    let shown = tree(|ui| super::super::show(&mut app, ui.ctx()));
    assert_eq!(slider_value(&shown, "Scale:"), (Some(100.0), Some(1.0), Some(10000.0), Some("100%".into())));
}
