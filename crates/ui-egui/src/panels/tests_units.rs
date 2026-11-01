//! Headless frames of Preferences ▸ Units: the General unit in the Transform panel, Properties,
//! the Control bar, the Info panel, dialogs, the Preferences dialog itself and New Document; the
//! Type unit in the Character panel; the change showing in the next frame.

use serde_json::json;
use vectorcraft_doc::Unit;

use super::tests_appearance::{app_with_rect, frame_events, run};
use super::*;

/// Every text drawn in the second headless frame of `f` (windows size themselves in the first).
fn texts(app: &mut VectorcraftApp, mut f: impl FnMut(&mut VectorcraftApp, &mut Ui)) -> Vec<String> {
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    frame_events(&ctx, app, vec![], &mut f);
    frame_events(&ctx, app, vec![], f).into_iter().map(|(s, _)| s).collect()
}

/// Every text drawn by the open dialog in one headless frame.
fn dialog_texts(app: &mut VectorcraftApp) -> Vec<String> {
    texts(app, |app, ui| crate::dialogs::show(app, ui.ctx()))
}

fn shows(t: &[String], s: &str) -> bool {
    t.iter().any(|x| x == s)
}

/// The 100 pt square of [`app_with_rect`] in millimetres.
fn side_mm() -> String {
    Unit::Millimeters.format(100.0)
}

fn general_mm(app: &mut VectorcraftApp) {
    app.run("prefs.set", json!({"key": "unitsGeneral", "value": "Millimeters"})).unwrap();
}

#[test]
fn panels_show_millimetres_after_the_preference_changes() {
    let mut app = app_with_rect();
    assert_eq!(side_mm(), "35.2778 mm");
    assert!(shows(&texts(&mut app, transform::show), "100 pt"), "points first");
    general_mm(&mut app);
    // The next frame: no restart, nothing cached.
    let t = texts(&mut app, transform::show);
    assert!(shows(&t, &side_mm()) && !shows(&t, "100 pt"), "Transform panel: {t:?}");
    assert!(shows(&texts(&mut app, properties::show), &side_mm()), "Properties panel");
    assert!(shows(&texts(&mut app, crate::chrome::control_bar), &side_mm()), "Control bar");
    assert!(shows(&texts(&mut app, info::show), &format!("W: {}", side_mm())), "Info panel");
    // Document Setup back to points: everything follows the document.
    run(&mut app, "document.setUnits", json!({"units": "Points"}));
    assert!(shows(&texts(&mut app, transform::show), "100 pt"));
}

#[test]
fn dialog_lengths_show_in_the_general_unit() {
    let mut app = app_with_rect();
    general_mm(&mut app);
    crate::menus::invoke(&mut app, "object.move", json!({}));
    let t = dialog_texts(&mut app);
    assert!(shows(&t, "0 mm"), "Move: {t:?}");
    // A typed unit wins; the move is in points.
    app.ui.dialog.as_mut().unwrap().fields.insert("dx".into(), json!("10 mm"));
    assert!(shows(&dialog_texts(&mut app), "10 mm"));
    crate::dialogs::confirm(&mut app).unwrap();
    let st = app.session.active().unwrap();
    let b = st.doc.bounds_of(&st.selection.objects, false).unwrap();
    assert!((b.x0 - (10.0 + 10.0 * 72.0 / 25.4)).abs() < 1e-6, "{b:?}");
    // A shape tool's size dialog and an effect's distances.
    crate::dialogs::open_tool_dialog(&mut app, "rectangle", json!({"x": 0, "y": 0}));
    assert!(shows(&dialog_texts(&mut app), &side_mm()), "Rectangle");
    app.run("effect.dialog", json!({"effect": "stylize.dropShadow"})).unwrap();
    let x = vectorcraft_effects::default_params("stylize.dropShadow").unwrap()["x"].as_f64().unwrap();
    let t = dialog_texts(&mut app);
    assert!(shows(&t, &Unit::Millimeters.format(x)), "Drop Shadow: {t:?}");
    crate::dialogs::cancel(&mut app);
}

#[test]
fn type_sizes_follow_units_type() {
    let mut app = app_with_rect();
    let id = run(&mut app, "text.create", json!({"x": 10, "y": 150, "text": "Hi", "size": 24}))["id"].clone();
    run(&mut app, "select.set", json!({ "ids": [id] }));
    general_mm(&mut app);
    assert!(shows(&texts(&mut app, character::show), "24 pt"), "General leaves type alone");
    app.run("prefs.set", json!({"key": "unitsType", "value": "inches"})).unwrap();
    let t = texts(&mut app, character::show);
    assert!(shows(&t, &Unit::Inches.format(24.0)), "Character panel: {t:?}");
}

#[test]
fn the_preferences_dialog_shows_the_documents_units() {
    let mut app = app_with_rect();
    run(&mut app, "document.setUnits", json!({"units": "Millimeters"}));
    crate::prefs_dialog::open(&mut app, Some("General"));
    assert_eq!(app.ui.dialog.as_ref().unwrap().fields["unitsGeneral"], json!("millimeters"));
    let inc = app.session.prefs.keyboard_increment;
    let frame = |app: &mut VectorcraftApp| texts(app, |app, ui| crate::prefs_dialog::show(app, ui.ctx()));
    assert!(shows(&frame(&mut app), &Unit::Millimeters.format(inc)), "Keyboard Increment in mm");
    // Choosing another General unit in the dialog shows at once, and OK applies it.
    app.ui.dialog.as_mut().unwrap().fields.insert("unitsGeneral".into(), json!("inches"));
    assert!(shows(&frame(&mut app), &Unit::Inches.format(inc)));
    crate::prefs_dialog::confirm(&mut app).unwrap();
    assert_eq!(app.session.active().unwrap().doc.units, Unit::Inches);
    assert_eq!(app.session.prefs.units_general, "inches");
}

#[test]
fn new_documents_start_in_the_general_preference() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    general_mm(&mut app);
    app.run("file.newDialog", json!({})).unwrap();
    assert_eq!(app.ui.dialog.as_ref().unwrap().fields["units"], json!("Millimeters"));
    let t = dialog_texts(&mut app);
    assert!(shows(&t, &Unit::Millimeters.format(612.0)), "Letter in mm: {t:?}");
    crate::dialogs::confirm(&mut app).unwrap();
    let d = &app.session.active().unwrap().doc;
    assert_eq!((d.units, d.artboards[0].rect.width()), (Unit::Millimeters, 612.0));
}

/// A label spelled down the left ruler is tall, and a sixteenth-inch step does not always leave
/// room for a long one: at zoom 12 the labels are 54 pt apart while "-0.0625" reaches 60.5 pt, so
/// the ruler leaves that label out rather than drawing it over its neighbour. Every tick stays.
#[test]
fn a_long_vertical_label_does_not_fit_a_sixteenth_inch_step() {
    use crate::canvas::vertical_label_height;
    use vectorcraft_doc::Unit;
    // One character is one line; each further character adds a line.
    assert_eq!(vertical_label_height(1), 9.5);
    assert_eq!(vertical_label_height(2), 18.0);
    assert_eq!(vertical_label_height(7), 60.5);
    // The gap a label has to fit into, at the zoom where a sixteenth is first chosen and beyond.
    let gap = |zoom: f64| Unit::Inches.ruler_step(zoom) * Unit::Inches.points() * zoom;
    assert!((gap(12.0) - 54.0).abs() < 1e-9, "{}", gap(12.0));
    assert!(vertical_label_height(7) > gap(12.0) as f32, "a seven-character label has to be left out");
    // The labels this ruler actually writes below an inch are six characters, which do fit.
    assert!(vertical_label_height("0.0625".chars().count()) < gap(12.0) as f32);
    // Zoomed further in the gap grows and even a long label fits.
    assert!(vertical_label_height(7) < gap(20.0) as f32, "{}", gap(20.0));
}

#[test]
fn ruler_labels_have_the_steps_decimals() {
    use crate::canvas::ruler_label;
    assert_eq!(ruler_label(100.0000001, 100.0), "100");
    assert_eq!(ruler_label(0.30000000004, 0.1), "0.3");
    assert_eq!(ruler_label(1.25, 0.05), "1.25");
    assert_eq!(ruler_label(-0.0000001, 0.5), "0");
    assert_eq!(ruler_label(-2.5, 0.5), "-2.5");
    // An inch ruler's binary steps keep their exact value: rounding the step's magnitude wrote
    // 1/16 as "0.06" and 1/8 as "0.13" (#2827).
    assert_eq!(ruler_label(0.0625, 0.0625), "0.0625");
    assert_eq!(ruler_label(0.125, 0.125), "0.125");
    assert_eq!(ruler_label(0.75, 0.25), "0.75");
    assert_eq!(ruler_label(1.0, 0.0625), "1");
    assert_eq!(ruler_label(1.1875, 0.0625), "1.1875");
}

#[test]
fn new_document_cards_follow_the_chosen_units() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    general_mm(&mut app);
    app.run("file.newDialog", json!({})).unwrap();
    let d = app.ui.dialog.as_mut().unwrap();
    d.fields.insert("category".into(), json!("Art & Illustration"));
    d.fields.insert("preset".into(), json!("Postcard"));
    let t = dialog_texts(&mut app);
    assert!(shows(&t, "101.6 × 152.4 mm"), "Postcard in mm: {t:?}");
    // Choosing Pixels: the cards show pixels and Postcard stays chosen.
    app.ui.dialog.as_mut().unwrap().fields.insert("units".into(), json!("Pixels"));
    let t = dialog_texts(&mut app);
    assert!(shows(&t, "288 × 432 px") && !shows(&t, "101.6 × 152.4 mm"), "Postcard in px: {t:?}");
    assert_eq!(app.ui.dialog.as_ref().unwrap().fields["preset"], json!("Postcard"));
}
