use super::*;
use craft::ui::dialog::{DialogId, HeldOk};
use craft::ui::patterns::form_dialogs::{DialogFields, FieldValue, HostedParamDialog};
use vectorcraft_engine::Session;

fn app() -> VectorcraftApp {
    let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
    app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
    app
}

fn ctx(app: &VectorcraftApp) -> egui::Context {
    app.ui_ctx.clone().expect("the test app has a context")
}

/// Whether Brush Options is open.
fn is_open(app: &VectorcraftApp) -> bool {
    app.ui.brush_options.is_some()
}

fn open_id(app: &VectorcraftApp) -> DialogId {
    app.ui.brush_options.expect("Brush Options is open")
}

/// Opens Brush Options of brush `name`.
fn open(app: &mut VectorcraftApp, name: &str) {
    app.run("ui.brushOptions", json!({ "name": name })).unwrap();
    assert!(is_open(app), "{name}");
}

/// The values of the open Brush Options dialog.
fn fields_of(app: &VectorcraftApp) -> DialogFields {
    let id = open_id(app);
    let dialogs = app.dialogs().expect("a window");
    dialogs.with(|host| host.get_by_id::<HostedParamDialog>(id).map(|dialog| dialog.fields().clone())).expect("the dialog is open")
}

/// Sets fields of the open dialog as automation does.
fn set(app: &mut VectorcraftApp, fields: Value) {
    let (id, ctx) = (open_id(app), ctx(app));
    let dialogs = app.dialogs().expect("a window");
    for (key, value) in fields.as_object().unwrap() {
        dialogs.with(|host| host.set_json_field(&ctx, id, key, value.clone())).unwrap_or_else(|error| panic!("{key}: {error}"));
    }
}

/// Presses OK: the held OK is answered by the app's pump, which then lets the dialog close. An error
/// is the reason the dialog stays open.
fn confirm(app: &mut VectorcraftApp) -> Result<(), String> {
    let (id, ctx) = (open_id(app), ctx(app));
    let dialogs = app.dialogs().expect("a window");
    for _ in 0..2 {
        dialogs.with(|host| host.confirm_by_id(&ctx, id));
        super::super::host::pump(app, &ctx);
    }
    match dialogs.with(|host| host.held_state(id)) {
        Some(HeldOk::Refused(reason)) => Err(reason.as_str().to_string()),
        _ if is_open(app) => Err("the dialog is still open".into()),
        _ => Ok(()),
    }
}

/// Cancels the open dialog.
fn cancel(app: &mut VectorcraftApp) {
    let (id, ctx) = (open_id(app), ctx(app));
    app.dialogs().expect("a window").with(|host| host.cancel_by_id(&ctx, id));
    super::super::host::pump(app, &ctx);
}

/// Runs one 1200 × 900 frame with `events`: the dialogs draw, then the app's pump runs.
fn frame(app: &mut VectorcraftApp, events: Vec<egui::Event>) {
    use egui::{Pos2, Rect};
    let ctx = ctx(app);
    let dialogs = app.dialogs().expect("a window");
    let raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1200.0, 900.0))), events, ..Default::default() };
    let mut out = ctx.run_ui(raw, |ui| {
        dialogs.with(|host| host.run(ui));
        super::super::host::pump(app, ui.ctx());
    });
    out.textures_delta.clear();
}

/// Presses and releases Escape.
fn escape(app: &mut VectorcraftApp) {
    let key = |pressed| egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed, repeat: false, modifiers: Default::default() };
    frame(app, vec![key(true)]);
    frame(app, vec![key(false)]);
}

/// What a step of the table expects at a JSON pointer of the saved brush.
enum Expect {
    Is(Value),
    Object,
    Absent,
}

use Expect::{Absent, Is, Object};

/// One table step: the values set on the open dialog in order, and what the brush holds after OK.
type Step = (Vec<Value>, Vec<(&'static str, Expect)>);

/// Opens `brush` once for each step, sets the step's fields, presses OK and checks the saved brush.
fn run_steps(app: &mut VectorcraftApp, brush: &str, steps: Vec<Step>) {
    for (sets, expect) in steps {
        open(app, brush);
        for fields in sets {
            set(app, fields);
        }
        confirm(app).unwrap_or_else(|error| panic!("{brush}: {error}"));
        let def = app.run("brush.get", json!({ "name": brush })).unwrap();
        for (pointer, want) in expect {
            let got = def.pointer(pointer);
            match want {
                Is(want) => assert_eq!(got, Some(&want), "{brush} {pointer}: {def}"),
                Object => assert!(got.is_some_and(Value::is_object), "{brush} {pointer}: {def}"),
                Absent => assert!(got.is_none_or(Value::is_null), "{brush} {pointer}: {def}"),
            }
        }
    }
}

/// The table "dialog fields to `brush.options` params": each brush type's fields, set on the open
/// dialog, become the brush's values at OK. Out-of-range values are kept within their ranges, Fixed
/// takes the first value and the art's second width goes with Fixed.
#[test]
fn dialog_fields_become_brush_options_params() {
    let mut app = app();
    let modes = |a: &str, b: &str, c: &str| Is(json!([a, b, c]));
    run_steps(
        &mut app,
        "6 pt. Flat",
        vec![
            (
                vec![json!({"angleMode": "random", "angleVariation": 500, "roundness": 140, "sizeMode": "pressure", "sizeVariation": 4})],
                vec![
                    ("/modes", modes("random", "fixed", "pressure")),
                    ("/variation", Is(json!([180.0, 0.0, 4.0]))),
                    ("/roundness", Is(json!(100.0))),
                ],
            ),
            // #764: Tilt, Bearing and Rotation.
            (
                vec![json!({"angleMode": "bearing", "roundnessMode": "tilt", "sizeMode": "rotation"})],
                vec![("/modes", modes("bearing", "tilt", "rotation"))],
            ),
        ],
    );
    run_steps(
        &mut app,
        "Confetti",
        vec![
            (
                vec![json!({"sizeMode": "fixed", "sizeMin": 5000, "spacingMode": "pressure", "scatterMode": "random", "rotationMode": "random",
                    "rotationMin": -400, "rotationRelativeTo": "path", "colorization": "hueShift", "hueJitter": 400})],
                vec![
                    ("/size", Is(json!([5000.0, 140.0]))),
                    ("/rotation/0", Is(json!(-180.0))),
                    ("/modes", Is(json!(["fixed", "pressure", "random", "random"]))),
                    ("/rotation_relative_to_path", Is(json!(true))),
                    ("/hue_jitter", Is(json!(180.0))),
                    ("/colorization/method", Is(json!("hueShift"))),
                    ("/colorization/key", Object),
                ],
            ),
            (
                vec![json!({"sizeMode": "tilt", "spacingMode": "bearing", "scatterMode": "rotation", "rotationMode": "bearing"})],
                vec![("/modes", Is(json!(["tilt", "bearing", "rotation", "bearing"])))],
            ),
        ],
    );
    run_steps(
        &mut app,
        "Arrow",
        vec![
            (
                vec![
                    json!({"width": 250, "guideStart": 10, "guideEnd": 140, "direction": "bottomToTop", "flipAcross": true, "colorization": "none"}),
                ],
                vec![
                    ("/width", Is(json!(250.0))),
                    ("/scale", Is(json!({"mode": "betweenGuides", "start": 0.1, "end": 1.0}))),
                    ("/direction", Is(json!("bottomToTop"))),
                    ("/flip_across", Is(json!(true))),
                    ("/flip_along", Is(json!(false))),
                    ("/colorization", Is(json!({"method": "none"}))),
                ],
            ),
            // Stretch to Fit drops the guides.
            (vec![json!({"scaleMode": "stretch"})], vec![("/scale", Is(json!({"mode": "stretch"})))]),
        ],
    );
    let pen = |mode: &str| -> Step {
        (
            vec![json!({"widthMode": mode, "widthMax": 300})],
            vec![("/width", Is(json!(100.0))), ("/width_mode", Is(json!(mode))), ("/width_max", Is(json!(300.0)))],
        )
    };
    run_steps(
        &mut app,
        "Tapered Stroke",
        vec![
            pen("pressure"),
            pen("tilt"),
            pen("bearing"),
            pen("rotation"),
            (vec![json!({"widthMode": "fixed"})], vec![("/width_mode", Absent), ("/width_max", Absent)]),
        ],
    );
    run_steps(
        &mut app,
        "Diamonds",
        vec![
            (
                vec![json!({"scale": 200, "spacing": 30, "fit": "approximate", "flipAlong": true})],
                vec![
                    ("/scale", Is(json!(200.0))),
                    ("/spacing", Is(json!(30.0))),
                    ("/fit", Is(json!("approximate"))),
                    ("/flip_along", Is(json!(true))),
                ],
            ),
            (
                vec![json!({"outerCornerTile": "autoBetween", "innerCornerTile": "autoOverlap"})],
                vec![("/outer_source", Is(json!("autoBetween"))), ("/inner_source", Is(json!("autoOverlap")))],
            ),
        ],
    );
    // Chain has corner art: Original keeps it, None and an automatic tile replace it, one corner at a time.
    run_steps(
        &mut app,
        "Chain",
        vec![
            (
                vec![json!({"outerCornerTile": "autoBetween"}), json!({"outerCornerTile": "original"})],
                vec![("/outer_corner", Object), ("/inner_corner", Object), ("/outer_source", Absent)],
            ),
            (
                vec![json!({"innerCornerTile": "none"})],
                vec![("/outer_corner", Object), ("/inner_corner", Absent), ("/inner_source", Is(json!("none")))],
            ),
            (
                vec![json!({"outerCornerTile": "autoSliced"})],
                vec![
                    ("/outer_corner", Absent),
                    ("/inner_corner", Absent),
                    ("/outer_source", Is(json!("autoSliced"))),
                    ("/inner_source", Is(json!("none"))),
                ],
            ),
        ],
    );
    // Opened again, the tiles show what the brush has: the corners the saved sources chose.
    open(&mut app, "Chain");
    let f = fields_of(&app);
    let shown = |key| brush_art_options::tile_choice(&f, key);
    assert_eq!((shown("outerCornerTile"), shown("innerCornerTile")), ("autoSliced".into(), "none".into()));
}

/// Pressing OK redraws the strokes painted with the brush.
#[test]
fn ok_redraws_the_strokes_painted_with_the_brush() {
    let mut app = app();
    let (id, before) = brushed(&mut app, "Diamonds");
    open(&mut app, "Diamonds");
    set(&mut app, json!({"scale": 200}));
    confirm(&mut app).unwrap();
    assert!((drawn_height(&app, id) / before - 2.0).abs() < 1e-6, "tiles twice the size");
    // The art's width at 200 % half way: a stroke drawn without pressure is between the two widths.
    let (id, before) = brushed(&mut app, "Tapered Stroke");
    open(&mut app, "Tapered Stroke");
    set(&mut app, json!({"widthMode": "pressure", "widthMax": 300}));
    confirm(&mut app).unwrap();
    let after = drawn_height(&app, id);
    assert!((after - 2.0 * before).abs() < 0.01 * before, "200 % half way: {before} then {after}");
    open(&mut app, "Tapered Stroke");
    set(&mut app, json!({"widthMode": "fixed"}));
    confirm(&mut app).unwrap();
    assert!((drawn_height(&app, id) - before).abs() < 1e-6);
}

#[test]
fn a_taken_name_keeps_the_dialog_open() {
    let mut app = app();
    open(&mut app, "6 pt. Flat");
    set(&mut app, json!({"name": "3 pt. Round"}));
    assert!(confirm(&mut app).is_err());
    assert!(is_open(&app));
}

#[test]
fn bristle_options_and_an_unknown_brush() {
    let mut app = app();
    open(&mut app, "Bristle Round");
    set(&mut app, json!({"shape": "flatFan", "density": 0, "length": 120}));
    confirm(&mut app).unwrap();
    let def = app.run("brush.get", json!({"name": "Bristle Round"})).unwrap();
    assert_eq!((def["shape"].clone(), def["density"].clone(), def["length"].clone()), (json!("flatFan"), json!(1.0), json!(120.0)));
    // No such brush: an error, nothing opens.
    assert!(app.run("ui.brushOptions", json!({"name": "Nope"})).is_err());
    assert!(!is_open(&app));
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
fn brush_key_color_eyedropper_updates_the_open_dialog() {
    use vectorcraft_tools::{PointerEvent, PointerKind};
    let mut app = app();
    app.run("paint.setFill", json!({"color": "#336699"})).unwrap();
    app.run("shape.rectangle", json!({"x": 40, "y": 40, "width": 80, "height": 60})).unwrap();
    open(&mut app, "Confetti");
    set(&mut app, json!({"colorization": "hueShift", "keyColorPick": true}));
    let back = app.session.tool_id().to_string();
    // The row arms its dropper and the pump arms the Eyedropper tool.
    frame(&mut app, vec![]);
    assert_eq!(app.session.tool_id(), "eyedropper");
    let view = app.view_info();
    crate::canvas::dispatch(&mut app, &PointerEvent::new(PointerKind::Down, 80.0, 70.0), view);
    frame(&mut app, vec![]);
    let [r, g, b] = fields_of(&app).color("keyColor").expect("the sample answers the dialog's Key Color");
    assert_eq!(vectorcraft_color::Color::rgb(r, g, b).to_hex(), "#336699");
    assert_eq!(app.session.tool_id(), back);
    assert!(app.ui.brush_key_color_pick.is_none());
    assert_ne!(fields_of(&app).bool("keyColorPick"), Some(true), "the pick is over");
}

/// #8024: the first Escape ends the Key Color pick and returns to the tool that armed it, leaving
/// Brush Options open with its Key Color. The dialog half (Escape ends a sampling row before it
/// cancels the dialog) is the framework's; this is the app half.
#[test]
fn escape_ends_the_key_color_pick_and_leaves_brush_options_open() {
    let mut app = app();
    app.select_tool("paintbrush");
    open(&mut app, "Confetti");
    set(&mut app, json!({"colorization": "hueShift", "keyColorPick": true}));
    let before = fields_of(&app).get("keyColor").cloned();
    frame(&mut app, vec![]);
    assert_eq!(app.session.tool_id(), "eyedropper");
    assert!(app.ui.brush_key_color_pick.is_some());
    escape(&mut app);
    frame(&mut app, vec![]);
    assert!(is_open(&app), "Brush Options stays open");
    assert_eq!(fields_of(&app).get("keyColor").cloned(), before, "the Key Color stays");
    assert_eq!(app.session.tool_id(), "paintbrush");
    assert!(app.ui.brush_key_color_pick.is_none());
    assert_ne!(fields_of(&app).bool("keyColorPick"), Some(true));
}

/// Each Colorization Tips sample paints the brush with its own method on the blue stroke:
/// Confetti's black and gray art keeps its colors with None, its black becomes the stroke color
/// with Tints and with Hue Shift (black is its key color), and stays black with Tints and Shades.
#[test]
fn each_tips_sample_paints_the_brush_with_its_method() {
    let mut app = app();
    open(&mut app, "Confetti");
    let d = fields_of(&app);
    let Some(FieldValue::Json(key)) = d.get("keyColor") else { panic!("the Key Color opens as a color") };
    let key: vectorcraft_color::Color = serde_json::from_value(key.clone()).unwrap();
    let colors = |id: &str| -> Vec<String> {
        let method = vectorcraft_brush::Colorization::parse(id, key).unwrap();
        let doc = brush_art_options::tip_doc(&brush_art_options::edited_def(&d), "scatter", method, 120.0, 44.0).unwrap();
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

/// With a brush definition that is not an object, the preview and Colorization Tips still draw.
#[test]
fn a_brush_definition_that_is_not_an_object_still_draws() {
    let mut f = DialogFields::new();
    f.set("__type", "scatter");
    f.set("__def", FieldValue::Json(json!(5)));
    let def = brush_art_options::edited_def(&f);
    assert_eq!(def, json!(5));
    let method = vectorcraft_brush::Colorization::default();
    // A sample of a definition that is no object is drawn from the type alone, or not drawn: it never panics.
    let _ = brush_art_options::tip_doc(&def, "scatter", method, 120.0, 44.0);
}

/// Each corner tile has a menu: Original (only when the brush has corner art), None and the four
/// automatic tiles. An empty tile key is the brush's own tile; a pick is kept, even a None or the
/// value already shown.
#[test]
fn pattern_options_tiles_open_empty_and_show_the_brushs_own_tile() {
    let mut app = app();
    open(&mut app, "Diamonds");
    let d = fields_of(&app);
    for key in ["outerCornerTile", "innerCornerTile", "sideTile", "startTile", "endTile"] {
        assert_eq!(d.text(key).unwrap_or_default(), "", "{key} opens empty");
    }
    let shown = |key| brush_art_options::tile_choice(&d, key);
    assert_eq!((shown("outerCornerTile"), shown("innerCornerTile")), ("none".into(), "none".into()), "Diamonds has no corner art");
    for corner in ["outer_corner", "inner_corner"] {
        assert!(!brush_art_options::original_available(&d, corner), "{corner}: no Original without corner art");
    }
    cancel(&mut app);
    open(&mut app, "Chain");
    let d = fields_of(&app);
    let mut tiles: Vec<&String> = match d.get("__tiles") {
        Some(FieldValue::Json(tiles)) => tiles.as_object().unwrap().keys().collect(),
        other => panic!("tiles: {other:?}"),
    };
    tiles.sort();
    assert_eq!(tiles, ["innerCorner", "outerCorner", "side"], "Chain has side and corner tiles, no start or end");
    assert_eq!(brush_art_options::tile_choice(&d, "outerCornerTile"), "original");
    // Original stays in a corner's menu after another choice, and goes back to the corner art.
    let mut d = d;
    d.set("outerCornerTile", "autoBetween");
    assert!(brush_art_options::original_available(&d, "outer_corner"));
    d.set("outerCornerTile", "original");
    assert_eq!(brush_art_options::tile_choice(&d, "outerCornerTile"), "original");
}

#[test]
fn pattern_tile_menus_use_document_patterns_and_side_none_keeps_dialog_open() {
    use std::sync::Arc;
    use vectorcraft_doc::{Node, PatternDef};
    use vectorcraft_geom::Rect;

    let mut app = app();
    app.session
        .edit("Pattern", |doc, _| {
            let art = Node::path(
                doc.alloc_id(),
                vectorcraft_geom::shapes::rectangle(Rect::new(0.0, 0.0, 30.0, 4.0)),
                vectorcraft_doc::Appearance::default_art(),
            );
            doc.patterns.push(PatternDef { tile: Rect::new(0.0, 0.0, 10.0, 4.0), ..PatternDef::new("Wide", vec![Arc::new(art)]) });
            Ok(())
        })
        .unwrap();
    let before = app.run("brush.get", json!({"name":"Diamonds"})).unwrap();
    open(&mut app, "Diamonds");
    let d = fields_of(&app);
    assert_eq!(brush_art_options::pattern_names(&d), vec!["Wide".to_string()]);
    for corner in [false, true] {
        let ids: Vec<_> = brush_art_options::tile_choices(&d, corner).into_iter().map(|(id, _)| id).collect();
        assert!(ids.contains(&"pattern:Wide".to_string()));
        if corner {
            for auto in ["autoCentered", "autoBetween", "autoSliced", "autoOverlap"] {
                assert!(ids.contains(&auto.to_string()));
            }
        } else {
            assert_eq!(ids.first().map(String::as_str), Some("none"));
            assert_eq!(ids.get(1).map(String::as_str), Some("original"));
        }
    }
    // Original stays available after a swatch is chosen, and restores the tile the dialog opened with.
    let mut d = d;
    d.set("sideTile", "Wide");
    assert!(brush_art_options::original_available(&d, "side"));
    d.set("sideTile", "original");
    let restored = brush_art_options::edited_def(&d);
    assert_eq!((&restored["side"], &restored["side_frame"]), (&before["side"], &before["side_frame"]));
    set(&mut app, json!({"sideTile": "original"}));
    confirm(&mut app).unwrap();
    let kept = app.run("brush.get", json!({"name":"Diamonds"})).unwrap();
    assert_eq!((&kept["side"], &kept["side_frame"]), (&before["side"], &before["side_frame"]));
    open(&mut app, "Diamonds");
    set(&mut app, json!({"sideTile": "Wide"}));
    let preview = brush_art_options::edited_def(&fields_of(&app));
    assert_eq!(preview["side_frame"], json!(Rect::new(0.0, 0.0, 10.0, 4.0)));
    assert_ne!(preview["side"], before["side"]);
    confirm(&mut app).unwrap();
    // A tile that opens on a swatch can also go back to the art it opened with.
    let opened = app.run("brush.get", json!({"name":"Diamonds"})).unwrap();
    open(&mut app, "Diamonds");
    let mut d = fields_of(&app);
    assert_eq!(brush_art_options::tile_choice(&d, "sideTile"), "pattern:Wide");
    assert!(brush_art_options::original_available(&d, "side"));
    d.set("sideTile", "original");
    assert_eq!(brush_art_options::edited_def(&d)["side"], opened["side"]);
    cancel(&mut app);
    app.session
        .edit("Delete pattern", |doc, _| {
            doc.patterns.clear();
            Ok(())
        })
        .unwrap();
    open(&mut app, "Diamonds");
    assert_eq!(brush_art_options::tile_choice(&fields_of(&app), "sideTile"), "original");
    let saved = app.run("brush.get", json!({"name":"Diamonds"})).unwrap();
    let undo = app.session.doc().unwrap().history.undo.len();
    set(&mut app, json!({"sideTile": "none"}));
    let error = confirm(&mut app).unwrap_err();
    assert!(error.contains("You must choose a side pattern."), "{error}");
    assert!(is_open(&app));
    assert_eq!(app.run("brush.get", json!({"name":"Diamonds"})).unwrap(), saved);
    assert_eq!(app.session.doc().unwrap().history.undo.len(), undo);
}

#[test]
fn new_pattern_brush_keeps_a_tile_swatch_choice() {
    use std::sync::Arc;
    use vectorcraft_doc::{Node, PatternDef};
    use vectorcraft_geom::Rect;

    let mut app = app();
    app.run("shape.rectangle", json!({"x":0,"y":0,"width":20,"height":4})).unwrap();
    app.session
        .edit("Pattern", |doc, _| {
            let art = Node::path(
                doc.alloc_id(),
                vectorcraft_geom::shapes::rectangle(Rect::new(0.0, 0.0, 30.0, 4.0)),
                vectorcraft_doc::Appearance::default_art(),
            );
            doc.patterns.push(PatternDef { tile: Rect::new(0.0, 0.0, 10.0, 4.0), ..PatternDef::new("Wide", vec![Arc::new(art)]) });
            Ok(())
        })
        .unwrap();
    super::open_new(&mut app, "pattern", None).unwrap();
    let name = fields_of(&app).text("name").unwrap().to_string();
    set(&mut app, json!({"sideTile": "Wide"}));
    confirm(&mut app).unwrap();
    let def = app.run("brush.get", json!({"name":name})).unwrap();
    assert_eq!(def["side_swatch"], "Wide");
    assert_eq!(def["side_frame"], json!(Rect::new(0.0, 0.0, 10.0, 4.0)));
}

/// A pattern brush with art for one kind of corner draws it at both kinds. Its other corner tile
/// shows that art as Original, and OK keeps it there when the first corner tile changes.
#[test]
fn pattern_options_keep_the_corner_art_a_corner_borrows() {
    let mut app = app();
    let art = app.run("brush.get", json!({"name": "Chain"})).unwrap()["outer_corner"].clone();
    app.run("brush.new", json!({"type": "pattern", "name": "One Corner", "params": {"side": art.clone(), "outer_corner": art}})).unwrap();
    open(&mut app, "One Corner");
    let d = fields_of(&app);
    let shown = |key| brush_art_options::tile_choice(&d, key);
    assert_eq!((shown("outerCornerTile"), shown("innerCornerTile")), ("original".into(), "original".into()));
    set(&mut app, json!({"outerCornerTile": "none"}));
    confirm(&mut app).unwrap();
    let def = app.run("brush.get", json!({"name": "One Corner"})).unwrap();
    assert!(def.get("outer_corner").is_none() && def["inner_corner"].is_object(), "{def}");
    assert_eq!(def["outer_source"], json!("none"));
}

#[test]
fn pattern_options_preview_and_save_borrowed_corner_before_clearing_source() {
    use vectorcraft_geom::Rect;

    let mut app = app();
    let chain = app.run("brush.get", json!({"name": "Chain"})).unwrap();
    let art = chain["outer_corner"].clone();
    let frame = json!(Rect::new(1.0, 2.0, 15.0, 18.0));
    for (name, source, borrowed) in [("Borrow Outer", "inner", "outer"), ("Borrow Inner", "outer", "inner")] {
        let source_art = format!("{source}_corner");
        let source_frame = format!("{source_art}_frame");
        let borrowed_art = format!("{borrowed}_corner");
        let borrowed_frame = format!("{borrowed_art}_frame");
        let source_tile = format!("{source}CornerTile");
        let borrowed_tile = format!("{borrowed}CornerTile");
        app.run("brush.new", json!({"type":"pattern", "name":name, "params":{"side":chain["side"], source_art:art, source_frame:frame}})).unwrap();
        open(&mut app, name);
        set(&mut app, json!({ source_tile: "none", borrowed_tile: "original" }));
        let preview = brush_art_options::edited_def(&fields_of(&app));
        assert_eq!(preview[&borrowed_art], chain["outer_corner"], "{borrowed} preview art");
        assert_eq!(preview[&borrowed_frame], frame, "{borrowed} preview frame");
        confirm(&mut app).unwrap();
        let saved = app.run("brush.get", json!({"name":name})).unwrap();
        assert_eq!(saved[&borrowed_art], chain["outer_corner"], "{borrowed} saved art");
        assert_eq!(saved[&borrowed_frame], frame, "{borrowed} saved frame");
    }
}

#[test]
fn pattern_options_borrowed_swatch_corner_keeps_its_frame() {
    use std::sync::Arc;
    use vectorcraft_doc::{Node, PatternDef};
    use vectorcraft_geom::Rect;

    let mut app = app();
    app.session
        .edit("Pattern", |doc, _| {
            let art = Node::path(
                doc.alloc_id(),
                vectorcraft_geom::shapes::rectangle(Rect::new(0.0, 0.0, 30.0, 4.0)),
                vectorcraft_doc::Appearance::default_art(),
            );
            doc.patterns.push(PatternDef { tile: Rect::new(0.0, 0.0, 10.0, 4.0), ..PatternDef::new("Corner", vec![Arc::new(art)]) });
            Ok(())
        })
        .unwrap();
    let side = app.run("brush.get", json!({"name":"Diamonds"})).unwrap()["side"].clone();
    app.run("brush.new", json!({"type":"pattern", "name":"Borrow", "params":{"side":side}})).unwrap();
    app.run("brush.options", json!({"name":"Borrow", "params":{"outer_corner_tile":{"pattern":"Corner"}}})).unwrap();
    let before = app.run("brush.get", json!({"name":"Borrow"})).unwrap();
    open(&mut app, "Borrow");
    set(&mut app, json!({"outerCornerTile":"none"}));
    confirm(&mut app).unwrap();
    let after = app.run("brush.get", json!({"name":"Borrow"})).unwrap();
    assert_eq!(after["inner_corner"], before["outer_corner"]);
    assert_eq!(after["inner_corner_frame"], before["outer_corner_frame"]);
    assert_eq!(after["outer_source"], json!("none"));
}

#[test]
fn pattern_options_selects_swatches_with_reserved_names() {
    use std::sync::Arc;
    use vectorcraft_doc::{Node, PatternDef};
    use vectorcraft_geom::Rect;

    let mut app = app();
    let names = ["none", "original", "autoCentered", "autoBetween", "autoSliced", "autoOverlap"];
    app.session
        .edit("Patterns", |doc, _| {
            for name in names {
                let art = Node::path(
                    doc.alloc_id(),
                    vectorcraft_geom::shapes::rectangle(Rect::new(0.0, 0.0, 30.0, 4.0)),
                    vectorcraft_doc::Appearance::default_art(),
                );
                doc.patterns.push(PatternDef { tile: Rect::new(0.0, 0.0, 10.0, 4.0), ..PatternDef::new(name, vec![Arc::new(art)]) });
            }
            Ok(())
        })
        .unwrap();
    open(&mut app, "Diamonds");
    let d = fields_of(&app);
    for name in names {
        let choice = (format!("pattern:{name}"), name.to_string());
        assert!(brush_art_options::tile_choices(&d, true).contains(&choice));
    }
    for (field, name) in [
        ("sideTile", "none"),
        ("outerCornerTile", "original"),
        ("innerCornerTile", "autoCentered"),
        ("startTile", "autoBetween"),
        ("endTile", "autoSliced"),
    ] {
        set(&mut app, json!({ field: format!("pattern:{name}") }));
    }
    let preview = brush_art_options::edited_def(&fields_of(&app));
    assert_eq!(preview["side_swatch"], "none");
    confirm(&mut app).unwrap();
    let saved = app.run("brush.get", json!({"name":"Diamonds"})).unwrap();
    for (art, name) in
        [("side", "none"), ("outer_corner", "original"), ("inner_corner", "autoCentered"), ("start", "autoBetween"), ("end", "autoSliced")]
    {
        assert_eq!(saved[format!("{art}_swatch")], name);
        assert_eq!(saved[format!("{art}_frame")], json!(Rect::new(0.0, 0.0, 10.0, 4.0)));
    }
}

/// OK leaves the corner fields of a corner tile the menus were not set to as they were: a corner
/// without art keeps borrowing the other kind's art when it gets some, and a corner that borrows
/// art still borrows it. A None picked from the menu is saved, after another choice too, and so is
/// a None set to the value the tile already shows.
#[test]
fn pattern_options_ok_leaves_corner_tiles_the_menus_were_not_set_to() {
    let mut app = app();
    open(&mut app, "Diamonds");
    set(&mut app, json!({"scale": 150}));
    confirm(&mut app).unwrap();
    let def = app.run("brush.get", json!({"name": "Diamonds"})).unwrap();
    assert!(def.get("outer_source").is_none() && def.get("inner_source").is_none(), "{def}");
    let art = app.run("brush.get", json!({"name": "Chain"})).unwrap()["outer_corner"].clone();
    app.run("brush.options", json!({"name": "Diamonds", "params": {"outer_corner": art.clone()}})).unwrap();
    open(&mut app, "Diamonds");
    assert_eq!(brush_art_options::tile_choice(&fields_of(&app), "innerCornerTile"), "original", "the inner corners draw the outer art");
    confirm(&mut app).unwrap();
    let def = app.run("brush.get", json!({"name": "Diamonds"})).unwrap();
    assert!(def["outer_corner"].is_object() && def.get("inner_corner").is_none(), "{def}");

    app.run("brush.new", json!({"type": "pattern", "name": "Plain", "params": {"side": art}})).unwrap();
    open(&mut app, "Plain");
    set(&mut app, json!({"outerCornerTile": "autoCentered"}));
    set(&mut app, json!({"outerCornerTile": "none"}));
    confirm(&mut app).unwrap();
    let def = app.run("brush.get", json!({"name": "Plain"})).unwrap();
    assert_eq!(def["outer_source"], json!("none"));
    assert!(def.get("inner_source").is_none(), "{def}");
    // A None set to what the tile shows is saved too.
    open(&mut app, "Plain");
    set(&mut app, json!({"innerCornerTile": "none"}));
    confirm(&mut app).unwrap();
    let def = app.run("brush.get", json!({"name": "Plain"})).unwrap();
    assert_eq!(def["inner_source"], json!("none"));
}

/// A level line, selected, painted with brush `brush`.
fn brushed_line(app: &mut VectorcraftApp, y: f64, brush: &str) -> u64 {
    let id = app.run("shape.line", json!({"x1": 20, "y1": y, "x2": 220, "y2": y})).unwrap()["id"].as_u64().unwrap();
    app.run("brush.apply", json!({"name": brush, "ids": [id]})).unwrap();
    id
}

/// The own brush settings of the first stroke of path `id`.
fn own_options(app: &VectorcraftApp, id: u64) -> Value {
    let st = app.session.active().unwrap().doc.node(NodeId(id)).and_then(|n| n.appearance.stroke().cloned()).unwrap();
    Value::Object(st.brush_options)
}

fn select(app: &mut VectorcraftApp, ids: &[u64]) {
    app.run("select.set", json!({ "ids": ids })).unwrap();
}

/// The keys of the rows Options of Selected Object shows for brush type `ty`.
fn object_keys(ty: &str) -> Vec<String> {
    brush_art_options::rows(ty, true).into_iter().map(|row| row.key).collect()
}

#[test]
fn options_of_selected_object_set_one_paths_art_brush_settings() {
    let mut app = app();
    let a = brushed_line(&mut app, 50.0, "Arrow");
    let b = brushed_line(&mut app, 100.0, "Arrow");
    select(&mut app, &[a]);
    assert!(object_available(&app));
    app.run("ui.brushObjectOptions", json!({})).unwrap();
    // The object form has the width, flips, overlap and colorization, not the scale or direction.
    let keys = object_keys("art");
    for key in ["width", "flipAlong", "flipAcross", "overlap", "colorization"] {
        assert!(keys.iter().any(|k| k == key), "{key} in {keys:?}");
    }
    for key in ["scaleMode", "direction", "guideStart"] {
        assert!(!keys.iter().any(|k| k == key), "{key} in {keys:?}");
    }
    set(&mut app, json!({"width": 50}));
    confirm(&mut app).unwrap();
    assert!(!is_open(&app));
    assert_eq!(own_options(&app, a), json!({"width": 50.0}));
    assert_eq!(own_options(&app, b), json!({}));
    assert_eq!(app.run("brush.get", json!({"name": "Arrow"})).unwrap()["width"], json!(100.0), "the brush keeps its width");
    // Opened again, it shows the path's own width, and OK without a change changes nothing.
    app.run("ui.brushObjectOptions", json!({})).unwrap();
    assert_eq!(fields_of(&app).number("width"), Some(50.0));
    confirm(&mut app).unwrap();
    assert_eq!(own_options(&app, a), json!({"width": 50.0}));
}

#[test]
fn options_of_selected_object_change_only_the_values_edited_on_each_path() {
    let mut app = app();
    let a = brushed_line(&mut app, 50.0, "Confetti");
    let b = brushed_line(&mut app, 100.0, "Confetti");
    app.run("brush.objectOptions", json!({"ids": [a], "params": {"size": [50, 80]}})).unwrap();
    app.run("brush.objectOptions", json!({"ids": [b], "params": {"size": [60, 120]}})).unwrap();
    select(&mut app, &[a, b]);
    app.run("ui.brushObjectOptions", json!({})).unwrap();
    // The dialog shows the first path's values; only the smallest size is edited.
    assert_eq!(fields_of(&app).number("sizeMax"), Some(80.0));
    set(&mut app, json!({"sizeMin": 40}));
    confirm(&mut app).unwrap();
    assert_eq!(own_options(&app, a)["size"], json!([40.0, 80.0]));
    assert_eq!(own_options(&app, b)["size"], json!([40.0, 120.0]), "the second path keeps its own largest size");
}

/// The modes the first stroke of path `id` draws its brush `brush` with.
fn drawn_modes(app: &VectorcraftApp, id: u64, brush: &str) -> Vec<Variation> {
    let lib = vectorcraft_brush::library(&app.session.active().unwrap().doc);
    let b = lib.iter().find(|b| b.name == brush).unwrap();
    let own = own_options(app, id);
    match b.with_stroke_options(own.as_object().unwrap()).unwrap().kind {
        BrushKind::Calligraphic(c) => c.modes().to_vec(),
        BrushKind::Scatter(s) => s.modes().to_vec(),
        _ => vec![],
    }
}

#[test]
fn options_of_selected_object_keep_the_modes_they_show_when_a_value_changes() {
    let mut app = app();
    // Confetti's size varies at random from 60% to 140%; a size of 60% to 60% leaves the mode the
    // dialog shows.
    let s = brushed_line(&mut app, 50.0, "Confetti");
    select(&mut app, &[s]);
    app.run("ui.brushObjectOptions", json!({})).unwrap();
    assert_eq!(fields_of(&app).text("sizeMode"), Some("random"));
    set(&mut app, json!({"sizeMax": 60}));
    confirm(&mut app).unwrap();
    assert_eq!(own_options(&app, s)["size"], json!([60.0, 60.0]));
    assert_eq!(drawn_modes(&app, s, "Confetti")[0], Variation::Random);
    // 6 pt. Flat's angle is Fixed; a variation set without a mode keeps it Fixed.
    let c = brushed_line(&mut app, 100.0, "6 pt. Flat");
    select(&mut app, &[c]);
    app.run("ui.brushObjectOptions", json!({})).unwrap();
    assert_eq!(fields_of(&app).text("angleMode"), Some("fixed"));
    set(&mut app, json!({"angleVariation": 30}));
    confirm(&mut app).unwrap();
    assert_eq!(drawn_modes(&app, c, "6 pt. Flat")[0], Variation::Fixed);
}

#[test]
fn options_of_selected_object_show_what_a_path_can_set_for_each_type() {
    let mut app = app();
    // Calligraphic: OK without a change leaves the path on the brush's settings.
    let c = brushed_line(&mut app, 40.0, "6 pt. Flat");
    select(&mut app, &[c]);
    app.run("ui.brushObjectOptions", json!({})).unwrap();
    confirm(&mut app).unwrap();
    assert_eq!(own_options(&app, c), json!({}));
    // Pattern: scale, spacing, flips and fit, not the tiles.
    let keys = object_keys("pattern");
    for key in ["scale", "spacing", "fit", "flipAlong", "flipAcross", "colorization"] {
        assert!(keys.iter().any(|k| k == key), "{key} in {keys:?}");
    }
    assert!(!keys.iter().any(|k| k.ends_with("Tile")), "{keys:?}");
    // Scatter: the four values with their modes, the rotation reference and the hue jitter.
    let keys = object_keys("scatter");
    for key in ["sizeMin", "spacingMin", "scatterMin", "rotationMin", "rotationRelativeTo", "hueJitter"] {
        assert!(keys.iter().any(|k| k == key), "{key} in {keys:?}");
    }
    let p = brushed_line(&mut app, 80.0, "Diamonds");
    select(&mut app, &[p]);
    app.run("ui.brushObjectOptions", json!({})).unwrap();
    set(&mut app, json!({"scale": 200}));
    confirm(&mut app).unwrap();
    assert_eq!(own_options(&app, p), json!({"scale": 200.0}));
    // Bristle brushes, nothing selected and two brush types are refused.
    let r = brushed_line(&mut app, 120.0, "Bristle Round");
    select(&mut app, &[r]);
    assert!(!object_available(&app));
    assert!(app.run("ui.brushObjectOptions", json!({})).is_err());
    select(&mut app, &[c, p]);
    assert!(!object_available(&app), "the menu item is off for two brush types");
    assert!(app.run("ui.brushObjectOptions", json!({})).is_err());
    assert!(!is_open(&app));
    select(&mut app, &[p, r]);
    assert!(!object_available(&app), "the menu item is off for a pattern and a bristle brush");
    app.run("select.none", json!({})).unwrap();
    assert!(!object_available(&app));
}

#[test]
fn options_of_selected_object_change_nothing_after_another_document_became_active() {
    let mut app = app();
    let a = brushed_line(&mut app, 50.0, "Arrow");
    select(&mut app, &[a]);
    app.run("ui.brushObjectOptions", json!({})).unwrap();
    set(&mut app, json!({"width": 50}));
    // A second document with a brushed path of the same id becomes the active one.
    app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
    app.run("document.activate", json!({"index": 1})).unwrap();
    let other = brushed_line(&mut app, 50.0, "Arrow");
    assert_eq!(other, a);
    // OK closes the dialog as Cancel does.
    confirm(&mut app).unwrap();
    assert!(!is_open(&app));
    assert_eq!(own_options(&app, other), json!({}), "the second document's path keeps the brush's settings");
    app.run("document.activate", json!({"index": 0})).unwrap();
    assert_eq!(own_options(&app, a), json!({}), "the first document's path keeps the brush's settings");
}
