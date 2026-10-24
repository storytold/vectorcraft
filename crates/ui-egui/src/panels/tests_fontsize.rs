//! Headless frames of the Font Size and Leading combos (#458): a preset picked from the dropdown
//! applies in one undo step, the presets show in the type unit, Leading's Auto entry, sizes that
//! differ show blank, and the Control bar's Character controls.

use std::cell::Cell;

use serde_json::json;
use vectorcraft_doc::{CharStyle, NodeKind, Unit};

use super::tests_appearance::{app_with_rect, click, frame_events, run, text_rect};
use super::*;

type Texts = Vec<(String, egui::Rect)>;

fn shows(t: &Texts, s: &str) -> bool {
    t.iter().any(|(x, _)| x == s)
}

/// [`app_with_rect`] with 12 pt type selected instead; returns its id.
fn app_with_text() -> (VectorcraftApp, u64) {
    let mut app = app_with_rect();
    let id = run(&mut app, "text.create", json!({"x": 10, "y": 150, "text": "Hi", "size": 12}))["id"].as_u64().unwrap();
    run(&mut app, "select.set", json!({ "ids": [id] }));
    (app, id)
}

fn style(app: &VectorcraftApp, id: u64) -> CharStyle {
    match &app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().kind {
        NodeKind::Text(t) => t.first_style(),
        _ => panic!("not type"),
    }
}

fn undo_len(app: &VectorcraftApp) -> usize {
    app.session.active().unwrap().history.undo.len()
}

/// A 120 pt Font Size combo in a row; `at` keeps where it starts.
fn size_combo(at: &Cell<egui::Pos2>) -> impl FnMut(&mut VectorcraftApp, &mut Ui) + '_ {
    |app, ui| {
        at.set(ui.max_rect().min);
        ui.horizontal(|ui| character::size_field(app, ui, "size", 120.0, app.session.type_unit()));
    }
}

#[test]
fn a_picked_font_size_preset_applies_in_one_undo_step() {
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let (mut app, id) = app_with_text();
    let at = Cell::new(egui::Pos2::ZERO);
    let t = frame_events(&ctx, &mut app, vec![], size_combo(&at));
    assert!(shows(&t, "12 pt") && !shows(&t, "24 pt"), "closed: {t:?}");
    // The chevron is the combo's last 20 pt.
    click(&ctx, &mut app, at.get() + egui::vec2(110.0, 13.0), size_combo(&at));
    let t = frame_events(&ctx, &mut app, vec![], size_combo(&at));
    assert!(shows(&t, "6 pt") && shows(&t, "24 pt") && shows(&t, "72 pt"), "open: {t:?}");
    let undo = undo_len(&app);
    click(&ctx, &mut app, text_rect(&t, "24 pt").center(), size_combo(&at));
    assert_eq!(style(&app, id).size, 24.0);
    assert_eq!(undo_len(&app), undo + 1, "one undo step");
}

#[test]
fn scaled_font_size_fields_and_preset_use_document_points() {
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let (mut app, id) = app_with_text();
    run(&mut app, "object.scale", json!({"sx": 200}));
    assert!(shows(&frame_events(&ctx, &mut app, vec![], character::show), "24 pt"));
    assert!(shows(&frame_events(&ctx, &mut app, vec![], crate::chrome::control_bar), "24 pt"));
    let at = Cell::new(egui::Pos2::ZERO);
    frame_events(&ctx, &mut app, vec![], size_combo(&at));
    click(&ctx, &mut app, at.get() + egui::vec2(110.0, 13.0), size_combo(&at));
    let t = frame_events(&ctx, &mut app, vec![], size_combo(&at));
    let undo = undo_len(&app);
    click(&ctx, &mut app, text_rect(&t, "10 pt").center(), size_combo(&at));
    assert_eq!(style(&app, id).size * 2.0, 10.0);
    assert_eq!(undo_len(&app), undo + 1);
    assert!(shows(&frame_events(&ctx, &mut app, vec![], size_combo(&at)), "10 pt"));
}

#[test]
fn scaled_group_shows_character_and_control_bar_size_fields() {
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let (mut app, _) = app_with_text();
    run(&mut app, "object.group", json!({}));
    run(&mut app, "object.scale", json!({"sx": 200}));
    assert!(shows(&frame_events(&ctx, &mut app, vec![], character::show), "24 pt"));
    assert!(shows(&frame_events(&ctx, &mut app, vec![], crate::chrome::control_bar), "24 pt"));
    run(&mut app, "text.setStyle", json!({"size": 10}));
    assert!(shows(&frame_events(&ctx, &mut app, vec![], character::show), "10 pt"));
}

#[test]
fn font_size_presets_show_in_the_type_unit() {
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let (mut app, _) = app_with_text();
    app.run("prefs.set", json!({"key": "unitsType", "value": "millimeters"})).unwrap();
    let at = Cell::new(egui::Pos2::ZERO);
    frame_events(&ctx, &mut app, vec![], size_combo(&at));
    click(&ctx, &mut app, at.get() + egui::vec2(110.0, 13.0), size_combo(&at));
    let t = frame_events(&ctx, &mut app, vec![], size_combo(&at));
    assert!(shows(&t, &Unit::Millimeters.format(24.0)) && !shows(&t, "24 pt"), "{t:?}");
}

#[test]
fn leading_lists_auto_first() {
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let (mut app, id) = app_with_text();
    run(&mut app, "text.setStyle", json!({"leading": 30}));
    let t = frame_events(&ctx, &mut app, vec![], character::show);
    // The Leading cell: its 22 pt label, the spacing, then the spinner (16), field (100 - 36)
    // and chevron (20) of a 100 pt combo.
    let label = text_rect(&t, "A↕");
    let sp = ctx.global_style().spacing.item_spacing.x;
    click(&ctx, &mut app, egui::pos2(label.center().x + 11.0 + sp + 90.0, label.center().y), character::show);
    let t = frame_events(&ctx, &mut app, vec![], character::show);
    click(&ctx, &mut app, text_rect(&t, "Auto").center(), character::show);
    assert_eq!(style(&app, id).leading, None);
}

#[test]
fn sizes_that_differ_show_blank() {
    let ctx = egui::Context::default();
    let (mut app, a) = app_with_text();
    let b = run(&mut app, "text.create", json!({"x": 10, "y": 180, "text": "Ho", "size": 24}))["id"].as_u64().unwrap();
    run(&mut app, "select.set", json!({ "ids": [a, b] }));
    let t = frame_events(&ctx, &mut app, vec![], character::show);
    assert!(!shows(&t, "12 pt") && !shows(&t, "24 pt"), "{t:?}");
    run(&mut app, "select.set", json!({ "ids": [b] }));
    assert!(shows(&frame_events(&ctx, &mut app, vec![], character::show), "24 pt"));
}

#[test]
fn the_control_bar_shows_the_font_size_of_selected_type() {
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let (mut app, _) = app_with_text();
    let t = frame_events(&ctx, &mut app, vec![], crate::chrome::control_bar);
    assert!(shows(&t, "Character:") && shows(&t, "12 pt"), "{t:?}");
    run(&mut app, "select.set", json!({ "ids": [] }));
    assert!(!shows(&frame_events(&ctx, &mut app, vec![], crate::chrome::control_bar), "Character:"));
}
