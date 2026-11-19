//! The Color Wheel panel's Show Document Colors row drawn headless: its colors and their order,
//! the heading that follows the selection, the colors that fit in one row, a click and an Alt+click
//! on a color, swatch links, gradient stops and freeform points, the colors kept during a drag, the panel menu,
//! `ui.colorWheel {documentColors}` and the saved setting.

use egui::{Event, Modifiers, PointerButton, Pos2, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};
use vectorcraft_color::Paint;
use vectorcraft_engine::Session;

use super::color_wheel::show;
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

fn harness(app: VectorcraftApp) -> Harness<'static, VectorcraftApp> {
    let mut h = Harness::builder().with_size(vec2(260.0, 360.0)).build_ui_state(|ui, app: &mut VectorcraftApp| show(app, ui), app);
    h.run_steps(2);
    h
}

fn label(hex: &str, n: u64) -> String {
    format!("{hex}\n{}", if n == 1 { "Used 1 time".to_string() } else { format!("Used {n} times") })
}

fn has(h: &Harness<'static, VectorcraftApp>, label: &str) -> bool {
    h.query_by_label(label).is_some()
}

fn press(h: &mut Harness<'static, VectorcraftApp>, pos: Pos2, pressed: bool, modifiers: Modifiers) {
    h.event(Event::ModifiersChanged(modifiers));
    h.event(Event::PointerMoved(pos));
    h.event(Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers });
    h.run_steps(1);
}

fn click(h: &mut Harness<'static, VectorcraftApp>, pos: Pos2, modifiers: Modifiers) {
    press(h, pos, true, modifiers);
    press(h, pos, false, modifiers);
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    h.run_steps(1);
}

/// The hex value of the one color object `id` uses (its fill: it has no stroke).
fn fill_of(app: &mut VectorcraftApp, id: &Value) -> String {
    let r = app.session.execute("recolor.colors", &json!({"ids": [id]})).unwrap();
    assert_eq!(r["colors"].as_array().map(Vec::len), Some(1), "{r}");
    r["colors"][0]["hex"].as_str().unwrap().to_string()
}

/// With nothing selected, the row is headed Document Colors and lists the art's colors, the red
/// that three rectangles use before the blue that one uses.
#[test]
fn show_document_colors_lists_the_art_colors_most_used_first() {
    let (mut app, _, _) = app();
    app.run("select.none", json!({})).unwrap();
    let h = harness(app);
    assert!(has(&h, "Document Colors"));
    let red = h.get_by_label(&label("#ff0000", 3)).rect();
    let blue = h.get_by_label(&label("#0000ff", 1)).rect();
    assert!(red.left() < blue.left(), "red {red:?} comes before blue {blue:?}");
}

/// With something selected the row is headed Selection Colors and lists the selection's colors.
#[test]
fn the_heading_and_the_colors_follow_the_selection() {
    let (app, _, _) = app();
    let mut h = harness(app);
    assert!(has(&h, "Selection Colors"));
    assert!(has(&h, &label("#0000ff", 1)));
    assert!(!has(&h, &label("#ff0000", 3)));
    h.state_mut().run("select.none", json!({})).unwrap();
    h.run_steps(2);
    assert!(has(&h, "Document Colors") && !has(&h, "Selection Colors"));
    assert!(has(&h, &label("#ff0000", 3)));
}

/// The row shows the most used colors that fit in one row and leaves out the rest.
#[test]
fn the_row_shows_only_the_colors_that_fit() {
    let (mut app, _, _) = app();
    for i in 0..30 {
        rect(&mut app, 170.0 + i as f64, json!({"color": format!("#00{:02x}00", 40 + 6 * i)}));
    }
    app.run("select.none", json!({})).unwrap();
    let h = harness(app);
    let red = h.get_by_label(&label("#ff0000", 3)).rect();
    let shown: Vec<_> = (0..30).filter_map(|i| h.query_by_label(&label(&format!("#00{:02x}00", 40 + 6 * i), 1)).map(|n| n.rect())).collect();
    assert!(!shown.is_empty() && shown.len() < 30, "{} of the 30 greens show", shown.len());
    assert!(shown.iter().all(|r| (r.top() - red.top()).abs() < 0.5 && r.right() <= 260.0), "one row inside the panel: {shown:?}");
}

/// A row wide enough for more than 128 chips shows as many colors as fit.
#[test]
fn a_wide_row_shows_more_than_128_colors() {
    let (mut app, _, _) = app();
    let hex = |i: usize| format!("#{:02x}{:02x}80", i % 256, i / 256 + 1);
    for i in 0..160 {
        rect(&mut app, 170.0, json!({"color": hex(i)}));
    }
    app.run("select.none", json!({})).unwrap();
    let mut h = Harness::builder().with_size(vec2(3400.0, 360.0)).build_ui_state(|ui, app: &mut VectorcraftApp| show(app, ui), app);
    h.run_steps(2);
    let shown = (0..160).filter(|i| has(&h, &label(&hex(*i), 1))).count();
    assert!(shown > 128, "{shown} of the 160 colors show");
}

/// A click on a color fills the selection with it, as one undo step.
#[test]
fn a_click_on_a_color_sets_the_fill() {
    let (mut app, reds, blue) = app();
    app.run("select.set", json!({"ids": [reds[0], blue]})).unwrap();
    let undo = app.session.doc().unwrap().history.undo.len();
    let mut h = harness(app);
    let at = h.get_by_label(&label("#0000ff", 1)).rect().center();
    click(&mut h, at, Modifiers::NONE);
    let app = h.state_mut();
    assert_eq!(app.session.doc().unwrap().history.undo.len(), undo + 1);
    assert_eq!(fill_of(app, &reds[0]), "#0000ff");
    assert_eq!(fill_of(app, &blue), "#0000ff");
    assert_eq!(fill_of(app, &reds[1]), "#ff0000");
}

/// Alt+click applies the color to the inactive proxy (the stroke) and leaves the fill in front.
#[test]
fn alt_click_on_a_color_sets_the_stroke() {
    let (mut app, reds, blue) = app();
    app.run("select.set", json!({"ids": [reds[0], blue]})).unwrap();
    let mut h = harness(app);
    let at = h.get_by_label(&label("#ff0000", 1)).rect().center();
    click(&mut h, at, Modifiers::ALT);
    let app = h.state_mut();
    assert!(app.session.fill_active, "the fill stays in front");
    let st = app.session.doc().unwrap();
    let paints = |id: &Value| {
        let n = st.doc.node(vectorcraft_doc::NodeId(id.as_u64().unwrap())).unwrap();
        (n.appearance.fill_paint().color().map(|c| c.to_hex()), n.appearance.stroke_paint().color().map(|c| c.to_hex()))
    };
    assert_eq!(paints(&blue), (Some("#0000ff".into()), Some("#ff0000".into())));
}

/// A color linked to a spot swatch stays linked at its tint when a click applies it.
#[test]
fn a_click_keeps_a_swatch_link_and_its_tint() {
    let (mut app, _, blue) = app();
    app.run("swatch.new", json!({"name": "Ink", "color": {"c": 0, "m": 100, "y": 60, "k": 0}, "spot": true})).unwrap();
    let ink = rect(&mut app, 170.0, json!({"swatch": "Ink", "tint": 40}));
    app.run("select.set", json!({"ids": [ink, blue]})).unwrap();
    let hex = app.session.execute("color.used", &json!({})).unwrap()["colors"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["swatch"] == "Ink")
        .expect("the tint is listed")["hex"]
        .as_str()
        .unwrap()
        .to_string();
    let mut h = harness(app);
    let at = h.get_by_label(&label(&hex, 1)).rect().center();
    click(&mut h, at, Modifiers::NONE);
    let st = h.state().session.doc().unwrap();
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
    let hex = app.session.execute("color.used", &json!({})).unwrap()["colors"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["swatch"] == "Ink")
        .expect("the stop's color is listed")["hex"]
        .as_str()
        .unwrap()
        .to_string();
    let mut h = harness(app);
    let at = h.get_by_label(&label(&hex, 1)).rect().center();
    click(&mut h, at, Modifiers::NONE);
    let Paint::Gradient(g) = crate::panels::current_paints(h.state()).0 else { panic!("still a gradient") };
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
    let hex = app.session.execute("color.used", &json!({})).unwrap()["colors"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["swatch"] == "Ink")
        .expect("the stroke's color is listed")["hex"]
        .as_str()
        .unwrap()
        .to_string();
    let mut h = harness(app);
    let at = h.get_by_label(&label(&hex, 1)).rect().center();
    click(&mut h, at, Modifiers::NONE);
    let Paint::Gradient(g) = crate::panels::current_paints(h.state()).0 else { panic!("still a gradient") };
    let points = g.freeform.expect("still freeform").points;
    assert_eq!(points[1].color.to_hex(), hex);
}

/// While a drag on the wheel recolors the selection, the row keeps the colors from before the
/// drag; once it ends, it lists the new color.
#[test]
fn a_drag_on_the_wheel_does_not_read_the_colors_again_until_it_ends() {
    let (app, _, blue) = app();
    let revision = app.session.active().unwrap().revision;
    let mut h = harness(app);
    let ring = h.get_by_label("Hue ring").rect();
    // The left of the ring (cyan), then its bottom, in the middle of the band.
    let r = ring.width() / 2.0 - 11.0;
    let (a, b) = (ring.center() - vec2(r, 0.0), ring.center() + vec2(0.0, r));
    press(&mut h, a, true, Modifiers::NONE);
    for p in [a, b, b] {
        h.event(Event::PointerMoved(p));
        h.run_steps(1);
    }
    assert!(h.state().session.in_interaction(), "the drag is in progress");
    assert_ne!(h.state().session.active().unwrap().revision, revision, "the drag changed the document");
    assert!(has(&h, &label("#0000ff", 1)), "the row keeps the colors from before the drag");
    press(&mut h, b, false, Modifiers::NONE);
    h.run_steps(2);
    let now = fill_of(h.state_mut(), &blue);
    assert_ne!(now, "#0000ff");
    assert!(!has(&h, &label("#0000ff", 1)));
    assert!(has(&h, &label(&now, 1)), "the row lists {now} after the drag");
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
    let uses = app.session.execute("color.used", &json!({})).unwrap()["uses"].as_u64().unwrap();
    assert!(uses > 5_000, "{uses} uses");
    let reds_used = app.session.execute("color.used", &json!({})).unwrap()["colors"][0]["count"].as_u64().unwrap();
    let mut h = Harness::builder().with_size(vec2(260.0, 360.0)).with_step_dt(0.1).build_ui_state(|ui, app: &mut VectorcraftApp| show(app, ui), app);
    h.run_steps(6);
    assert!(has(&h, &label("#ff0000", reds_used)));
    // Half a second after the last read, an edit is read at once.
    h.state_mut().run("paint.setFill", json!({"ids": [reds[0]], "color": "#00ff00"})).unwrap();
    h.run_steps(1);
    assert!(has(&h, &label("#ff0000", reds_used - 1)) && has(&h, &label("#00ff00", 1)), "the first edit is read at once");
    // The next edit within half a second waits.
    h.state_mut().run("paint.setFill", json!({"ids": [reds[1]], "color": "#00ff00"})).unwrap();
    h.run_steps(1);
    assert!(has(&h, &label("#00ff00", 1)), "the row waits after the second edit");
    h.run_steps(5);
    assert!(has(&h, &label("#ff0000", reds_used - 2)) && has(&h, &label("#00ff00", 2)), "the row reads the colors again");
    h.state_mut().run("paint.setFill", json!({"ids": [reds[2]], "color": "#00ff00"})).unwrap();
    h.state_mut().run("select.set", json!({"ids": [reds[2]]})).unwrap();
    h.run_steps(1);
    assert!(has(&h, "Selection Colors") && has(&h, &label("#00ff00", 1)), "a new selection reads the colors at once");
    assert!(!has(&h, &label("#00ff00", 3)));
}

/// A row widened past the 128 colors last read keeps them while an interaction is in progress, and
/// in art with more than 5,000 color uses until half a second after the last read.
#[test]
fn a_widened_row_waits_for_the_interaction_and_the_half_second() {
    let (mut app, reds, _) = app();
    let hex = |i: usize| format!("#{:02x}{:02x}80", i % 256, i / 256 + 1);
    for i in 0..160 {
        rect(&mut app, 170.0, json!({"color": hex(i)}));
    }
    for _ in 0..5 {
        app.run("select.all", json!({})).unwrap();
        app.run("edit.duplicate", json!({})).unwrap();
    }
    app.run("select.none", json!({})).unwrap();
    let uses = app.session.execute("color.used", &json!({})).unwrap()["uses"].as_u64().unwrap();
    assert!(uses > 5_000, "{uses} uses");
    let (narrow, wide) = (vec2(260.0, 360.0), vec2(3400.0, 360.0));
    let mut h = Harness::builder().with_size(narrow).with_step_dt(0.1).build_ui_state(|ui, app: &mut VectorcraftApp| show(app, ui), app);
    h.run_steps(6);
    let shown = |h: &Harness<'static, VectorcraftApp>| (0..160).filter(|i| has(h, &label(&hex(*i), 32))).count();
    // During an interaction the wide row keeps the 128 colors read for the narrow one.
    h.state_mut().session.begin_interaction("test").unwrap();
    h.set_size(wide);
    h.run_steps(6);
    assert!(shown(&h) <= 128, "{} colors show during the interaction", shown(&h));
    h.state_mut().session.cancel_interaction().unwrap();
    h.run_steps(6);
    assert!(shown(&h) > 128, "{} colors show after the interaction", shown(&h));
    // An edit read in the narrow row, then a wide row within half a second, waits for the half second.
    h.set_size(narrow);
    h.run_steps(1);
    h.state_mut().run("paint.setFill", json!({"ids": [reds[0]], "color": "#00ff00"})).unwrap();
    h.run_steps(1);
    assert!(has(&h, &label("#ff0000", 95)), "the edit is read at once");
    h.set_size(wide);
    h.run_steps(1);
    assert!(shown(&h) <= 128, "{} colors show within half a second", shown(&h));
    h.run_steps(5);
    assert!(shown(&h) > 128, "{} colors show after half a second", shown(&h));
}

/// Without the setting the row is not drawn, and the panel menu's Show Document Colors turns it on.
#[test]
fn the_panel_menu_turns_the_row_on_and_off() {
    let off = || {
        let (mut a, _, _) = app();
        a.run("ui.colorWheel", json!({"documentColors": false})).unwrap();
        a
    };
    let mut h = harness(off());
    assert!(!has(&h, "Selection Colors"));
    // The menu in a context of its own, as the other panel menu tests draw it.
    let mut app = off();
    let ctx = egui::Context::default();
    let raw = |events| egui::RawInput { events, screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(240.0, 320.0))), ..Default::default() };
    let click_item = |app: &mut VectorcraftApp| {
        let mut out = ctx.run_ui(raw(vec![]), |ui| super::color_wheel::menu(app, ui));
        out.textures_delta.clear();
        let item = out
            .shapes
            .iter()
            .find_map(|c| match &c.shape {
                egui::Shape::Text(t) if t.galley.text().ends_with("Show Document Colors") => Some(t.visual_bounding_rect().center()),
                _ => None,
            })
            .expect("the menu shows Show Document Colors");
        let button = |pressed| Event::PointerButton { pos: item, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
        ctx.run_ui(raw(vec![Event::PointerMoved(item), button(true), button(false)]), |ui| super::color_wheel::menu(app, ui)).textures_delta.clear();
    };
    click_item(&mut app);
    assert!(app.ui.color_wheel.document_colors);
    click_item(&mut app);
    assert!(!app.ui.color_wheel.document_colors);
    h.state_mut().run("ui.colorWheel", json!({"documentColors": true})).unwrap();
    h.run_steps(2);
    assert!(has(&h, "Selection Colors"));
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
    let s = read(json!({"color_wheel": {"documentColors": true, "view": "box"}, "color_rgb_percent": true}));
    assert!(s.color_wheel.document_colors && s.color_rgb_percent);
    let s = read(json!({"color_wheel": {"documentColors": "yes", "view": "box"}, "color_rgb_percent": true}));
    assert!(!s.color_wheel.document_colors && s.color_rgb_percent);
    assert_eq!(s.color_wheel.view, super::color_wheel::View::Box);
}
