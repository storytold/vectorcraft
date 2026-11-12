//! Headless frames of the Stroke panel's width profile library: the ≡ menu's Add to Profiles…,
//! Delete Profile and Reset Profiles, and the Profile dropdown listing saved profiles.

use serde_json::json;
use vectorcraft_doc::WidthProfile;

use super::tests_appearance::{click, frame_events, run, text_rect};
use super::*;

fn app_with_line() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    run(&mut app, "file.new", json!({"width": 200, "height": 200}));
    run(&mut app, "shape.line", json!({"x1": 10, "y1": 100, "x2": 150, "y2": 100}));
    app
}

fn custom() -> WidthProfile {
    WidthProfile { points: vec![(0.0, 0.3, 0.3), (0.6, 1.4, 1.4), (1.0, 0.0, 0.0)] }
}

/// Give the selected line `p` as its variable width.
fn set_profile(app: &mut VectorcraftApp, p: WidthProfile) {
    app.session
        .edit("Width", |d, sel| {
            let id = sel.objects[0];
            d.node_mut(id).unwrap().appearance.stroke_mut().unwrap().profile = Some(p);
            Ok(())
        })
        .unwrap();
}

fn panel_and_menu(app: &mut VectorcraftApp, ui: &mut Ui) {
    stroke::show(app, ui);
    stroke::menu(app, ui);
}

fn profile_points(app: &VectorcraftApp) -> Option<Vec<(f64, f64, f64)>> {
    current_stroke(app).and_then(|s| s.profile).map(|p| p.points)
}

#[test]
fn menu_saves_deletes_and_resets_profiles() {
    let ctx = egui::Context::default();
    let mut app = app_with_line();
    set_profile(&mut app, custom());
    let t = frame_events(&ctx, &mut app, vec![], panel_and_menu);
    click(&ctx, &mut app, text_rect(&t, "   Add to Profiles…").center(), panel_and_menu);
    // A name dialog (OK runs stroke.widthProfile.add).
    let d = app.ui.dialog.clone().expect("name dialog");
    assert_eq!((d.str("__command"), d.str("name")), ("stroke.widthProfile.add".to_string(), "Width Profile 1".to_string()));
    app.ui.dialog.as_mut().unwrap().fields.insert("name".into(), json!("Mine"));
    crate::dialogs::confirm(&mut app).unwrap();
    assert_eq!(app.session.prefs.width_profiles[0].name, "Mine");
    // Saved profiles don't trip the Preferences dialog, which only edits preferences.
    crate::prefs_dialog::open(&mut app, None);
    crate::prefs_dialog::confirm(&mut app).unwrap();
    assert_eq!(app.session.prefs.width_profiles.len(), 1);

    // Delete Profile removes the selected stroke's saved profile; the stroke keeps its widths.
    let t = frame_events(&ctx, &mut app, vec![], panel_and_menu);
    click(&ctx, &mut app, text_rect(&t, "   Delete Profile").center(), panel_and_menu);
    assert!(app.session.prefs.width_profiles.is_empty());
    assert_eq!(profile_points(&app), Some(custom().points));

    run(&mut app, "stroke.widthProfile.add", json!({"name": "A"}));
    let t = frame_events(&ctx, &mut app, vec![], panel_and_menu);
    click(&ctx, &mut app, text_rect(&t, "   Reset Profiles").center(), panel_and_menu);
    assert!(app.session.prefs.width_profiles.is_empty());
}

#[test]
fn dropdown_lists_saved_profiles_and_applies_them() {
    let ctx = egui::Context::default();
    let mut app = app_with_line();
    set_profile(&mut app, custom());
    run(&mut app, "stroke.widthProfile.add", json!({"name": "Mine"}));
    run(&mut app, "stroke.set", json!({"profile": "uniform"}));
    let t = frame_events(&ctx, &mut app, vec![], stroke::show);
    // The dropdown sits right of the "Profile:" label.
    let label = text_rect(&t, "Profile:");
    let button = egui::pos2(label.right() + ctx.global_style().spacing.item_spacing.x + 50.0, label.center().y);
    click(&ctx, &mut app, button, stroke::show);
    let t = frame_events(&ctx, &mut app, vec![], stroke::show);
    for p in WidthProfile::PRESETS {
        text_rect(&t, p.label);
    }
    click(&ctx, &mut app, text_rect(&t, "Mine").center(), stroke::show);
    assert_eq!(profile_points(&app), Some(custom().points));
}

#[test]
fn every_arrowhead_preview_draws_its_head() {
    use vectorcraft_doc::Arrowhead;
    // Rendered on transparent: the head adds ink to its half of the preview.
    let ink = |a: Option<Arrowhead>, start: bool| {
        let d = stroke::arrow_doc(a, start, 56.0, 22.0).unwrap();
        let img = vectorcraft_render::Renderer::new().render_region(&d, vectorcraft_geom::Rect::new(0.0, 0.0, 56.0, 22.0), 2.0, false);
        let xs = if start { 0..56 } else { 56..112 };
        xs.flat_map(|x| (0..44).map(move |y| (x, y))).filter(|&(x, y)| img.pixel(x, y)[3] > 64).count()
    };
    let plain = (ink(None, false), ink(None, true));
    for a in Arrowhead::ALL {
        assert!(ink(Some(a.clone()), false) > plain.0 + 20, "{a:?} at the end");
        assert!(ink(Some(a.clone()), true) > plain.1 + 20, "{a:?} at the start");
    }
    // So does a custom head: a 4 × 4 square behind its tip.
    let pts = [(0.0, -2.0), (4.0, -2.0), (4.0, 2.0), (0.0, 2.0)].map(|(x, y)| vectorcraft_geom::Point::new(x, y));
    let square = vectorcraft_geom::PathData::single(vectorcraft_geom::SubPath::polyline(&pts, true));
    let custom = Arrowhead::Custom(std::sync::Arc::new(vectorcraft_doc::CustomArrowhead::new("Square".into(), square)));
    assert!(ink(Some(custom.clone()), false) > plain.0 + 20);
    assert!(ink(Some(custom), true) > plain.1 + 20);
}

#[test]
fn arrowhead_dropdown_offers_the_new_heads() {
    let ctx = egui::Context::default();
    let mut app = app_with_line();
    let t = frame_events(&ctx, &mut app, vec![], stroke::show);
    // The end-arrowhead dropdown is the second 64 pt button right of the "Arrowheads:" label.
    let label = text_rect(&t, "Arrowheads:");
    let sp = ctx.global_style().spacing.item_spacing.x;
    click(&ctx, &mut app, egui::pos2(label.right() + 2.0 * sp + 64.0 + 32.0, label.center().y), stroke::show);
    let t = frame_events(&ctx, &mut app, vec![], stroke::show);
    click(&ctx, &mut app, text_rect(&t, "Barbed").center(), stroke::show);
    assert_eq!(current_stroke(&app).and_then(|s| s.end_arrow), Some(vectorcraft_doc::Arrowhead::Barbed));
}
