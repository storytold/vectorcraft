//! The Color Wheel panel's Show Document Colors row drawn headless through the `Fixture`: its colors
//! and their order, the heading that follows the selection, a click on a color, swatch links,
//! gradient stops and freeform points, the colors kept during an interaction, `ui.colorWheel
//! {documentColors}` and the saved setting.

use serde_json::{Value, json};
use vectorcraft_color::Paint;
use vectorcraft_engine::Session;

use super::tests_color_wheel::Panel;
use crate::VectorcraftApp;

/// A rectangle filled with `fill` (any paint `paint.setFill` takes), without a stroke, which
/// becomes the selection. → its id.
fn rect(app: &mut VectorcraftApp, x: f64, fill: Value) -> Value {
    let id = app.run("shape.rectangle", json!({"x": x, "y": 10, "width": 30, "height": 30})).unwrap()["id"].clone();
    app.run("paint.setFill", fill).unwrap();
    app.run("paint.setStroke", json!({"none": true, "focus": false})).unwrap();
    id
}

/// A document with three red rectangles and one blue one (selected), with Show Document Colors on.
/// → the app, the ids of the red rectangles and the id of the blue one.
fn app() -> (VectorcraftApp, Vec<Value>, Value) {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
    let reds = (0..3).map(|i| rect(&mut app, 10.0 + 40.0 * i as f64, json!({"color": "#ff0000"}))).collect();
    let blue = rect(&mut app, 130.0, json!({"color": "#0000ff"}));
    app.run("ui.colorWheel", json!({"documentColors": true})).unwrap();
    (app, reds, blue)
}

fn label(hex: &str, n: u64) -> String {
    format!("{hex}\n{}", if n == 1 { "Used 1 time".to_string() } else { format!("Used {n} times") })
}

fn has(p: &Panel, label: &str) -> bool {
    p.fx.by_label(label).is_some()
}

/// The hex value of the one color object `id` uses (its fill: it has no stroke).
fn fill_of(app: &mut VectorcraftApp, id: &Value) -> String {
    let r = app.session.execute("recolor.colors", &json!({"ids": [id]})).unwrap();
    assert_eq!(r["colors"].as_array().map(Vec::len), Some(1), "{r}");
    r["colors"][0]["hex"].as_str().unwrap().to_string()
}

/// The hex of the first listed color used with swatch `swatch`.
fn used_hex(app: &mut VectorcraftApp, swatch: &str) -> String {
    app.session.execute("color.used", &json!({})).unwrap()["colors"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["swatch"] == swatch)
        .expect("the color is listed")["hex"]
        .as_str()
        .unwrap()
        .to_string()
}

fn undo_len(app: &VectorcraftApp) -> usize {
    app.session.doc().unwrap().history.undo.len()
}

/// With nothing selected, the row is headed Document Colors and lists the art's colors, the red
/// that three rectangles use first and the blue that one uses after it.
#[test]
fn show_document_colors_lists_the_art_colors_most_used_first() {
    let (mut app, _, _) = app();
    app.run("select.none", json!({})).unwrap();
    let used = app.session.execute("color.used", &json!({})).unwrap();
    assert_eq!((used["colors"][0]["hex"].as_str(), used["colors"][1]["hex"].as_str()), (Some("#ff0000"), Some("#0000ff")));
    let p = Panel::new(app);
    assert!(has(&p, "Document Colors"));
    assert!(has(&p, &label("#ff0000", 3)) && has(&p, &label("#0000ff", 1)));
}

/// With something selected the row is headed Selection Colors and lists the selection's colors.
#[test]
fn the_heading_and_the_colors_follow_the_selection() {
    let (app, _, _) = app();
    let mut p = Panel::new(app);
    assert!(has(&p, "Selection Colors"));
    assert!(has(&p, &label("#0000ff", 1)));
    assert!(!has(&p, &label("#ff0000", 3)));
    p.run("select.none", json!({}));
    assert!(has(&p, "Document Colors") && !has(&p, "Selection Colors"));
    assert!(has(&p, &label("#ff0000", 3)));
}

/// A click on a color fills the selection with it, as one undo step.
#[test]
fn a_click_on_a_color_sets_the_fill() {
    let (mut app, reds, blue) = app();
    app.run("select.set", json!({"ids": [reds[0], blue]})).unwrap();
    let undo = undo_len(&app);
    let mut p = Panel::new(app);
    p.point(&label("#0000ff", 1));
    assert_eq!(undo_len(&p.app()), undo + 1);
    let mut app = p.app_mut();
    assert_eq!(fill_of(&mut app, &reds[0]), "#0000ff");
    assert_eq!(fill_of(&mut app, &blue), "#0000ff");
    assert_eq!(fill_of(&mut app, &reds[1]), "#ff0000");
}

/// A color linked to a spot swatch stays linked at its tint when a click applies it.
#[test]
fn a_click_keeps_a_swatch_link_and_its_tint() {
    let (mut app, _, blue) = app();
    app.run("swatch.new", json!({"name": "Ink", "color": {"c": 0, "m": 100, "y": 60, "k": 0}, "spot": true})).unwrap();
    let ink = rect(&mut app, 170.0, json!({"swatch": "Ink", "tint": 40}));
    app.run("select.set", json!({"ids": [ink, blue]})).unwrap();
    let hex = used_hex(&mut app, "Ink");
    let mut p = Panel::new(app);
    p.point(&label(&hex, 1));
    let app = p.app();
    let st = app.session.doc().unwrap();
    let fill = st.doc.node(vectorcraft_doc::NodeId(blue.as_u64().unwrap())).unwrap().appearance.fill_paint().clone();
    let Paint::Solid { swatch, tint, .. } = fill else { panic!("a solid fill, not {fill:?}") };
    assert_eq!(swatch.as_deref(), Some("Ink"));
    assert!((tint - 0.4).abs() < 1e-3, "tint {tint}");
}

/// A click recolors the selected gradient stop, linking it to a color's swatch at its tint.
#[test]
fn a_click_recolors_the_selected_gradient_stop() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
    app.run("swatch.new", json!({"name": "Ink", "color": {"c": 0, "m": 100, "y": 60, "k": 0}, "spot": true})).unwrap();
    let stops = json!([{"offset": 0, "color": "#000000", "swatch": "Ink", "tint": 40}, {"offset": 1, "color": "#0000ff"}]);
    rect(&mut app, 10.0, json!({"gradient": {"stops": stops, "start": [10, 25], "end": [40, 25]}}));
    app.select_tool("gradient");
    app.run("gradient.selectStop", json!({"index": 1})).unwrap();
    app.run("ui.colorWheel", json!({"documentColors": true})).unwrap();
    let hex = used_hex(&mut app, "Ink");
    let mut p = Panel::new(app);
    p.point(&label(&hex, 1));
    let Paint::Gradient(g) = crate::panels::current_paints(&p.app()).0 else { panic!("still a gradient") };
    let stop = &g.gradient.stops[1];
    assert_eq!((stop.color.to_hex(), stop.swatch.as_deref()), (hex, Some("Ink")));
    assert!((stop.tint - 0.4).abs() < 1e-3, "tint {}", stop.tint);
}

/// A click on a color linked to a swatch gives a selected freeform point the color alone and keeps
/// the freeform fill.
#[test]
fn a_click_gives_a_freeform_point_the_color_alone() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
    app.run("swatch.new", json!({"name": "Ink", "color": {"c": 0, "m": 100, "y": 60, "k": 0}, "spot": true})).unwrap();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 100, "height": 100})).unwrap();
    app.run("paint.editGradient", json!({"kind": "freeform"})).unwrap();
    app.run("paint.setStroke", json!({"swatch": "Ink", "tint": 40, "focus": false})).unwrap();
    app.select_tool("gradient");
    app.run("paint.freeform.selectPoint", json!({"index": 1})).unwrap();
    app.run("ui.colorWheel", json!({"documentColors": true})).unwrap();
    let hex = used_hex(&mut app, "Ink");
    let mut p = Panel::new(app);
    p.point(&label(&hex, 1));
    let Paint::Gradient(g) = crate::panels::current_paints(&p.app()).0 else { panic!("still a gradient") };
    let points = g.freeform.expect("still freeform").points;
    assert_eq!(points[1].color.to_hex(), hex);
}

/// While an interaction recolors the selection, the row keeps the colors from before it; once it
/// ends, the row lists the new color.
#[test]
fn a_drag_on_the_wheel_does_not_read_the_colors_again_until_it_ends() {
    let (app, _, _) = app();
    let mut p = Panel::new(app);
    p.app_mut().session.begin_interaction("test").unwrap();
    p.run("paint.setFill", json!({"color": "#00ff00"}));
    assert!(p.app().session.in_interaction());
    assert!(has(&p, &label("#0000ff", 1)), "the row keeps the colors from before the interaction");
    p.app_mut().session.cancel_interaction().unwrap();
    p.fx.idle(3);
    assert!(has(&p, &label("#0000ff", 1)) || has(&p, &label("#00ff00", 1)), "the row reads the colors again");
}

/// In art with more than 5,000 color uses, the row reads the colors again after an edit at most
/// once every half second of frames, and a change of selection reads them at once.
#[test]
fn in_large_art_an_edit_reads_the_colors_again_after_half_a_second() {
    let (mut app, reds, _) = app();
    app.run("select.set", json!({"ids": [reds[0]]})).unwrap();
    for _ in 0..11 {
        app.run("select.all", json!({})).unwrap();
        app.run("edit.duplicate", json!({})).unwrap();
    }
    app.run("select.none", json!({})).unwrap();
    let used = app.session.execute("color.used", &json!({})).unwrap();
    assert!(used["uses"].as_u64().unwrap() > 5_000, "{used}");
    let reds_used = used["colors"][0]["count"].as_u64().unwrap();
    let mut p = Panel::new(app);
    // A frame is 1/60 s: 31 frames pass the half second.
    p.fx.idle(31);
    assert!(has(&p, &label("#ff0000", reds_used)));
    p.run("paint.setFill", json!({"ids": [reds[0]], "color": "#00ff00"}));
    assert!(has(&p, &label("#ff0000", reds_used - 1)) && has(&p, &label("#00ff00", 1)), "the first edit is read at once");
    p.run("paint.setFill", json!({"ids": [reds[1]], "color": "#00ff00"}));
    assert!(has(&p, &label("#00ff00", 1)), "the row waits after the second edit");
    p.fx.idle(31);
    assert!(has(&p, &label("#ff0000", reds_used - 2)) && has(&p, &label("#00ff00", 2)), "the row reads the colors again");
    p.app_mut().run("paint.setFill", json!({"ids": [reds[2]], "color": "#00ff00"})).unwrap();
    p.run("select.set", json!({"ids": [reds[2]]}));
    assert!(has(&p, "Selection Colors") && has(&p, &label("#00ff00", 1)), "a new selection reads the colors at once");
    assert!(!has(&p, &label("#00ff00", 3)));
}

/// `ui.colorWheel {documentColors}` sets the row's setting, keeps the others, and refuses a value
/// that is not a bool, changing nothing.
#[test]
fn ui_color_wheel_sets_document_colors() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    assert_eq!(app.run("ui.colorWheel", json!({})).unwrap()["documentColors"], json!(false));
    app.run("ui.colorWheel", json!({"view": "box"})).unwrap();
    let r = app.run("ui.colorWheel", json!({"documentColors": true})).unwrap();
    assert_eq!((r["documentColors"].clone(), r["view"].clone()), (json!(true), json!("box")));
    assert_eq!(app.run("ui.colorWheel", json!({"documentColors": "no"})).unwrap_err(), "`documentColors` must be true or false");
    assert!(app.ui.color_wheel.document_colors);
    assert_eq!(app.run("ui.colorWheel", json!({"documentColors": null})).unwrap()["documentColors"], json!(true));
}

/// The saved setting reads leniently: a value this build does not know reads as off, and the other
/// settings and preferences are kept.
#[test]
fn the_saved_setting_reads_leniently() {
    let read = |v: Value| serde_json::from_value::<crate::UiState>(v).expect("the preferences read");
    let s = read(json!({"color_wheel": {"documentColors": true, "view": "box"}, "color_panel": {"rgbPercent": true}}));
    assert!(s.color_wheel.document_colors && s.color_panel.rgb_percent);
    let s = read(json!({"color_wheel": {"documentColors": "yes", "view": "box"}, "color_panel": {"rgbPercent": true}}));
    assert!(!s.color_wheel.document_colors && s.color_panel.rgb_percent);
    assert_eq!(s.color_wheel.view, super::color_wheel::View::Box);
}
