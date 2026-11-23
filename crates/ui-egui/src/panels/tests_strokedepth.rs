//! Headless frames of the Stroke panel's width profile library: the ≡ menu's Add to Profiles…,
//! Delete Profile and Reset Profiles, and the Profile dropdown listing saved profiles and dimmed
//! while the strokes draw with a brush.

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

/// The Stroke panel's Profile dropdown button, right of the "Profile:" label.
fn profile_button(ctx: &egui::Context, app: &mut VectorcraftApp) -> egui::Pos2 {
    let t = frame_events(ctx, app, vec![], stroke::show);
    let label = text_rect(&t, "Profile:");
    egui::pos2(label.right() + ctx.global_style().spacing.item_spacing.x + 50.0, label.center().y)
}

/// Is the Profile dropdown dimmed for the stroke the panels show?
fn dimmed(ctx: &egui::Context, app: &VectorcraftApp) -> bool {
    stroke::brush_keeps_size(app, ctx, current_stroke(app).as_ref()).is_some()
}

/// The rows the Profile dropdown lists after a click on it.
fn listed(ctx: &egui::Context, app: &mut VectorcraftApp) -> Vec<&'static str> {
    let button = profile_button(ctx, app);
    click(ctx, app, button, stroke::show);
    let t = frame_events(ctx, app, vec![], stroke::show);
    let out = WidthProfile::PRESETS.iter().map(|p| p.label).filter(|l| t.iter().any(|(s, _)| s == l)).collect();
    // Close the list.
    let escape = egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() };
    frame_events(ctx, app, vec![escape], stroke::show);
    out
}

/// A stroke drawn with a brush (here the 3 pt. Round calligraphic brush) keeps the brush's size,
/// so a width profile does not change it: the Profile dropdown is dimmed, opens no list on a
/// uniform stroke, and lists only Uniform on a stroke that has a profile (which resets it).
/// Without the brush, the full list comes back.
#[test]
fn dropdown_is_dimmed_while_the_brush_keeps_its_size() {
    let ctx = egui::Context::default();
    let mut app = app_with_line();
    let id = app.session.active().unwrap().selection.objects[0].0;
    run(&mut app, "brush.apply", json!({"ids": [id], "name": "3 pt. Round"}));
    assert!(dimmed(&ctx, &app));
    assert_eq!(listed(&ctx, &mut app), Vec::<&str>::new(), "no list on a uniform stroke");

    set_profile(&mut app, custom());
    assert_eq!(listed(&ctx, &mut app), ["Uniform"]);
    let button = profile_button(&ctx, &mut app);
    click(&ctx, &mut app, button, stroke::show);
    let t = frame_events(&ctx, &mut app, vec![], stroke::show);
    click(&ctx, &mut app, text_rect(&t, "Uniform").center(), stroke::show);
    assert_eq!(profile_points(&app), None, "Uniform resets the profile");

    run(&mut app, "brush.remove", json!({"ids": [id]}));
    assert!(!dimmed(&ctx, &app));
    assert_eq!(listed(&ctx, &mut app).len(), WidthProfile::PRESETS.len());
}

/// While the Live Paint Bucket is active, the dropdown follows the stroke the bucket paints with,
/// not a selected line that draws with a brush.
#[test]
fn the_live_paint_bucket_stroke_decides_whether_the_dropdown_is_dimmed() {
    let ctx = egui::Context::default();
    let mut app = app_with_line();
    let id = app.session.active().unwrap().selection.objects[0].0;
    run(&mut app, "brush.apply", json!({"ids": [id], "name": "3 pt. Round"}));
    assert!(dimmed(&ctx, &app));
    app.session.select_tool("livePaintBucket", Default::default()).unwrap();
    assert!(current_stroke(&app).is_some_and(|s| s.brush.is_none()), "the bucket's stroke has no brush");
    assert!(!dimmed(&ctx, &app));
    assert_eq!(listed(&ctx, &mut app).len(), WidthProfile::PRESETS.len());
}

/// With an Appearance panel stroke row active, the dropdown follows that row: a brushed row below
/// a plain top stroke dims it, and the plain row does not.
#[test]
fn the_active_stroke_row_decides_whether_the_dropdown_is_dimmed() {
    let ctx = egui::Context::default();
    let mut app = app_with_line();
    let id = app.session.active().unwrap().selection.objects[0].0;
    run(&mut app, "brush.apply", json!({"ids": [id], "name": "3 pt. Round"}));
    run(&mut app, "appearance.addStroke", json!({}));
    let items = app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().appearance.items.clone();
    let rows: Vec<(usize, bool)> = items
        .iter()
        .enumerate()
        .filter_map(|(i, it)| if let vectorcraft_doc::AppearanceItem::Stroke(s) = it { Some((i, s.brush.is_some())) } else { None })
        .collect();
    let [(lower, true), (top, false)] = rows[..] else { panic!("{rows:?}") };
    assert!(!dimmed(&ctx, &app), "the top stroke has no brush");
    run(&mut app, "appearance.setActiveItem", json!({"index": lower}));
    assert!(dimmed(&ctx, &app));
    assert_eq!(listed(&ctx, &mut app), Vec::<&str>::new());
    run(&mut app, "appearance.setActiveItem", json!({"index": top}));
    assert!(!dimmed(&ctx, &app));
}

/// A selected group with a brushed stroke of its own dims the dropdown, though the panel shows its
/// first member's plain stroke: the list edits the group's stroke.
#[test]
fn a_groups_own_brushed_stroke_dims_the_dropdown() {
    let ctx = egui::Context::default();
    let mut app = app_with_line();
    let a = app.session.active().unwrap().selection.objects[0].0;
    run(&mut app, "shape.line", json!({"x1": 10, "y1": 150, "x2": 150, "y2": 150}));
    let b = app.session.active().unwrap().selection.objects[0].0;
    let g = run(&mut app, "object.group", json!({"ids": [a, b]}))["id"].as_u64().unwrap();
    run(&mut app, "appearance.addStroke", json!({"ids": [g]}));
    app.session
        .edit("Brush", |d, _| {
            d.node_mut(vectorcraft_doc::NodeId(g)).unwrap().appearance.stroke_mut().unwrap().brush = Some("3 pt. Round".into());
            Ok(())
        })
        .unwrap();
    run(&mut app, "select.set", json!({"ids": [g]}));
    assert!(current_stroke(&app).is_some_and(|s| s.brush.is_none()), "the panel shows a member's plain stroke");
    assert!(dimmed(&ctx, &app));
    assert_eq!(listed(&ctx, &mut app), Vec::<&str>::new());
}

/// A selection change with no edit between decides whether the dropdown is dimmed: from a brushed
/// line to a plain one and back.
#[test]
fn selecting_another_object_without_an_edit_updates_the_dimming() {
    let ctx = egui::Context::default();
    let mut app = app_with_line();
    let a = app.session.active().unwrap().selection.objects[0].0;
    run(&mut app, "shape.line", json!({"x1": 10, "y1": 150, "x2": 150, "y2": 150}));
    let b = app.session.active().unwrap().selection.objects[0].0;
    run(&mut app, "brush.apply", json!({"ids": [a], "name": "3 pt. Round"}));
    run(&mut app, "select.set", json!({"ids": [a]}));
    assert!(dimmed(&ctx, &app));
    run(&mut app, "select.set", json!({"ids": [b]}));
    assert!(!dimmed(&ctx, &app), "the plain line follows a profile");
    run(&mut app, "select.set", json!({"ids": [a]}));
    assert!(dimmed(&ctx, &app));
}

/// The Flip Along and Flip Across buttons beside a dimmed dropdown do not flip a brushed stroke's
/// profile, which does not change what the stroke draws.
#[test]
fn flip_buttons_leave_a_brushed_strokes_profile_alone() {
    let ctx = egui::Context::default();
    let mut app = app_with_line();
    set_profile(&mut app, custom());
    let spacing = ctx.global_style().spacing.item_spacing.x;
    // The dropdown is 100 pt wide and the Flip Along button, 22 pt wide, follows it.
    let flip = |ctx: &egui::Context, app: &mut VectorcraftApp| {
        let button = profile_button(ctx, app);
        let at = egui::pos2(button.x + stroke::PROFILE_DROPDOWN_WIDTH / 2.0 + spacing + 11.0, button.y);
        click(ctx, app, at, stroke::show);
    };
    flip(&ctx, &mut app);
    let flipped = profile_points(&app);
    assert_ne!(flipped, Some(custom().points), "Flip Along flips a plain stroke's profile");
    let id = app.session.active().unwrap().selection.objects[0].0;
    run(&mut app, "brush.apply", json!({"ids": [id], "name": "3 pt. Round"}));
    assert!(dimmed(&ctx, &app));
    let before = profile_points(&app);
    assert!(before.is_some());
    flip(&ctx, &mut app);
    assert_eq!(profile_points(&app), before, "the buttons are off on a brushed stroke");
}

/// With several objects selected, the dropdown is dimmed only while every one of their strokes
/// draws with a brush: a profile still changes the strokes without one. A brush name the
/// document's brushes lack draws a plain stroke, which follows the profile.
#[test]
fn mixed_selection_dims_the_dropdown_only_when_every_stroke_has_a_brush() {
    let ctx = egui::Context::default();
    let mut app = app_with_line();
    let a = app.session.active().unwrap().selection.objects[0].0;
    run(&mut app, "shape.line", json!({"x1": 10, "y1": 150, "x2": 150, "y2": 150}));
    let b = app.session.active().unwrap().selection.objects[0].0;
    run(&mut app, "brush.apply", json!({"ids": [a], "name": "3 pt. Round"}));
    // The first selected object is the brushed one, whose stroke the panel shows.
    run(&mut app, "select.set", json!({"ids": [a, b]}));
    assert!(current_stroke(&app).and_then(|s| s.brush).is_some());
    assert!(!dimmed(&ctx, &app));
    assert_eq!(listed(&ctx, &mut app).len(), WidthProfile::PRESETS.len());

    run(&mut app, "brush.apply", json!({"ids": [b], "name": "3 pt. Round"}));
    run(&mut app, "select.set", json!({"ids": [a, b]}));
    assert!(dimmed(&ctx, &app));
    assert_eq!(listed(&ctx, &mut app), Vec::<&str>::new());

    // The shown stroke is uniform and the other has a profile: Uniform still removes it.
    let edit = |app: &mut VectorcraftApp, f: &dyn Fn(&mut vectorcraft_doc::Appearance)| {
        app.session
            .edit("Edit", |d, _| {
                f(&mut d.node_mut(vectorcraft_doc::NodeId(b)).unwrap().appearance);
                Ok(())
            })
            .unwrap();
    };
    edit(&mut app, &|ap| ap.stroke_mut().unwrap().profile = Some(custom()));
    assert_eq!(current_stroke(&app).and_then(|s| s.profile), None);
    assert_eq!(listed(&ctx, &mut app), ["Uniform"]);
    let button = profile_button(&ctx, &mut app);
    click(&ctx, &mut app, button, stroke::show);
    let t = frame_events(&ctx, &mut app, vec![], stroke::show);
    click(&ctx, &mut app, text_rect(&t, "Uniform").center(), stroke::show);
    let node = |app: &VectorcraftApp| app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(b)).unwrap().clone();
    assert_eq!(node(&app).appearance.stroke().unwrap().profile, None, "Uniform reset the other stroke");

    edit(&mut app, &|ap| ap.stroke_mut().unwrap().brush = Some("No Such Brush".into()));
    assert!(!dimmed(&ctx, &app));

    // An object without a stroke item takes a plain stroke from a profile.
    edit(&mut app, &|ap| ap.items.retain(|i| !matches!(i, vectorcraft_doc::AppearanceItem::Stroke(_))));
    assert!(node(&app).appearance.stroke().is_none());
    assert!(!dimmed(&ctx, &app));
    assert_eq!(listed(&ctx, &mut app).len(), WidthProfile::PRESETS.len());
}

/// The dimmed dropdown's hover text is the Width tool's status message, word for word, so the two
/// share their catalog rows.
#[test]
fn the_hover_text_is_the_width_tools_status_message() {
    let literal = format!("tl!(\"{}\")", vectorcraft_tools::distort::KEEPS_SIZE);
    assert!(include_str!("stroke.rs").contains(&literal), "panels/stroke.rs has no {literal}");
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
fn a_long_custom_arrowhead_preview_fits_in_its_box() {
    use vectorcraft_doc::Arrowhead;
    // 40 units long and 4 wide, reaching 4 units past its tip.
    let pts = [(-4.0, -2.0), (36.0, -2.0), (36.0, 2.0), (-4.0, 2.0)].map(|(x, y)| vectorcraft_geom::Point::new(x, y));
    let bar = vectorcraft_geom::PathData::single(vectorcraft_geom::SubPath::polyline(&pts, true));
    // 120 units across: smaller than the smallest head weight can draw it.
    let pts = [(0.0, -60.0), (4.0, -60.0), (4.0, 60.0), (0.0, 60.0)].map(|(x, y)| vectorcraft_geom::Point::new(x, y));
    let wall = vectorcraft_geom::PathData::single(vectorcraft_geom::SubPath::polyline(&pts, true));
    let heads = [bar, wall].map(|p| Arrowhead::Custom(std::sync::Arc::new(vectorcraft_doc::CustomArrowhead::new("Bar".into(), p))));
    for (head, start) in heads.iter().flat_map(|h| [(h, false), (h, true)]) {
        let d = stroke::arrow_doc(Some(head.clone()), start, 56.0, 22.0).unwrap();
        // Rendered well beyond the 56 × 22 pt box, at 1 pixel a point: all the ink is inside it.
        let img = vectorcraft_render::Renderer::new().render_region(&d, vectorcraft_geom::Rect::new(-100.0, -50.0, 156.0, 72.0), 1.0, false);
        let ink: Vec<(u32, u32)> = (0..256).flat_map(|x| (0..122).map(move |y| (x, y))).filter(|&(x, y)| img.pixel(x, y)[3] > 64).collect();
        assert!(ink.len() > 40, "start {start}: the line and head draw");
        let outside: Vec<&(u32, u32)> = ink.iter().filter(|&&(x, y)| !(100..156).contains(&x) || !(50..72).contains(&y)).collect();
        assert!(outside.is_empty(), "start {start}: {} pixels outside the box, e.g. {:?}", outside.len(), outside.first());
    }
}

#[test]
fn the_arrowhead_menu_renders_only_the_previews_in_view() {
    let ctx = egui::Context::default();
    let mut app = app_with_line();
    let t = frame_events(&ctx, &mut app, vec![], stroke::show);
    let label = text_rect(&t, "Arrowheads:");
    let sp = ctx.global_style().spacing.item_spacing.x;
    click(&ctx, &mut app, egui::pos2(label.right() + 2.0 * sp + 64.0 + 32.0, label.center().y), stroke::show);
    frame_events(&ctx, &mut app, vec![], stroke::show);
    frame_events(&ctx, &mut app, vec![], stroke::show);
    let previews = ctx.tex_manager().read().allocated().filter(|(_, m)| m.name.contains(":arrow:") && m.name.ends_with(":false")).count();
    // The button's and the rows' in the 360 pt list, not one for each of the menu's heads.
    let rows = vectorcraft_doc::Arrowhead::ALL.len() + 1;
    assert!(previews > 5 && previews < rows / 2 + 5, "{previews} previews for {rows} rows");
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
