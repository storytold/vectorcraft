//! Headless frames of the Stroke panel's width profile library: the ≡ menu's Add to Profiles…,
//! Delete Profile and Reset Profiles, and the Profile select listing saved profiles and disabled
//! while the strokes draw with a brush.
// PLAN: the name dialog is read through assumed `DialogHost::open_dialogs()` entries with an `id` (hub-w1).

use craft::ui::runtime::commands::CommandStates;
use egui::{Key, Modifiers};
use serde_json::json;
use vectorcraft_doc::WidthProfile;

use crate::tests::panel_harness::Panel;
// PLAN: assumed path of object-panels-w8's `Panel` harness (op.A15).
use super::tests_appearance::run;
use super::*;

fn app_with_line() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    run(&mut app, "file.new", json!({"width": 200, "height": 200}));
    run(&mut app, "shape.line", json!({"x1": 10, "y1": 100, "x2": 150, "y2": 100}));
    app
}

fn panel() -> Panel {
    Panel::new(app_with_line(), stroke::show)
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

fn profile_points(app: &VectorcraftApp) -> Option<Vec<(f64, f64, f64)>> {
    current_stroke(app).and_then(|s| s.profile).map(|p| p.points)
}

/// Runs the ≡ menu row of command `id` the way the menu does: only when its state is enabled.
fn menu_row(app: &mut VectorcraftApp, id: &str) {
    let mut states = CommandStates::new();
    stroke::menu_states(app, &mut states);
    let rows = stroke::menu_rows(app);
    let command = rows
        .iter()
        .find_map(|r| match r {
            craft::ui::menu::PanelRow::Command(c) if c.id == id => Some(c.clone()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no menu row {id}"));
    assert!(states.get(&command).is_some_and(|s| s.is_enabled()), "{id} is disabled");
    app.run(&command.id, command.params).unwrap();
}

#[test]
fn menu_saves_deletes_and_resets_profiles() {
    let panel = panel();
    set_profile(&mut panel.app(), custom());
    menu_row(&mut panel.app(), "ui.paramDialog");
    // A name dialog (OK runs stroke.widthProfile.add).
    let mut app = panel.app();
    let id = crate::tests::picks::open_dialogs(&app).first().map(|d| d.id).expect("name dialog");
    crate::tests::picks::answer_dialog(&mut app, id, &[("name", json!("Mine"))], true);
    assert_eq!(app.session.prefs.width_profiles[0].name, "Mine");
    // Saved profiles don't trip the Preferences dialog, which only edits preferences.
    crate::prefs_dialog::open(&mut app, None);
    crate::prefs_dialog::confirm(&mut app).unwrap();
    assert_eq!(app.session.prefs.width_profiles.len(), 1);

    // Delete Profile removes the selected stroke's saved profile; the stroke keeps its widths.
    menu_row(&mut app, "stroke.widthProfile.delete");
    assert!(app.session.prefs.width_profiles.is_empty());
    assert_eq!(profile_points(&app), Some(custom().points));

    run(&mut app, "stroke.widthProfile.add", json!({"name": "A"}));
    menu_row(&mut app, "stroke.widthProfile.reset");
    assert!(app.session.prefs.width_profiles.is_empty());
}

#[test]
fn dropdown_lists_saved_profiles_and_applies_them() {
    let mut panel = panel();
    set_profile(&mut panel.app(), custom());
    run(&mut panel.app(), "stroke.widthProfile.add", json!({"name": "Mine"}));
    run(&mut panel.app(), "stroke.set", json!({"profile": "uniform"}));
    panel.frame();
    panel.click("Profile");
    for p in WidthProfile::PRESETS {
        assert!(panel.shows(p.label), "{} is listed", p.label);
    }
    panel.click("Mine");
    assert_eq!(profile_points(&panel.app()), Some(custom().points));
}

/// Is the Profile select disabled for the stroke the panels show?
fn dimmed(panel: &Panel) -> bool {
    let mut app = panel.app();
    let st = current_stroke(&app);
    stroke::brush_keeps_size(&mut app, st.as_ref()).is_some()
}

/// The rows the Profile select lists after a click on it.
fn listed(panel: &mut Panel) -> Vec<&'static str> {
    panel.frame();
    if panel.enabled("Profile") {
        panel.click("Profile");
    }
    let out = WidthProfile::PRESETS.iter().map(|p| p.label).filter(|l| panel.shows(l)).collect();
    panel.key(Key::Escape, Modifiers::NONE);
    out
}

/// While the Live Paint Bucket is active, the select follows the stroke the bucket paints with,
/// not a selected line that draws with a brush.
#[test]
fn the_live_paint_bucket_stroke_decides_whether_the_dropdown_is_dimmed() {
    let mut panel = panel();
    let id = panel.app().session.active().unwrap().selection.objects[0].0;
    run(&mut panel.app(), "brush.apply", json!({"ids": [id], "name": "3 pt. Round"}));
    assert!(dimmed(&panel));
    panel.app().session.select_tool("livePaintBucket", Default::default()).unwrap();
    assert!(current_stroke(&panel.app()).is_some_and(|s| s.brush.is_none()), "the bucket's stroke has no brush");
    assert!(!dimmed(&panel));
    assert_eq!(listed(&mut panel).len(), WidthProfile::PRESETS.len());
}

/// With an Appearance panel stroke row active, the select follows that row: a brushed row below
/// a plain top stroke dims it, and the plain row does not.
#[test]
fn the_active_stroke_row_decides_whether_the_dropdown_is_dimmed() {
    let mut panel = panel();
    let id = panel.app().session.active().unwrap().selection.objects[0].0;
    run(&mut panel.app(), "brush.apply", json!({"ids": [id], "name": "3 pt. Round"}));
    run(&mut panel.app(), "appearance.addStroke", json!({}));
    let items = panel.app().session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().appearance.items.clone();
    let rows: Vec<(usize, bool)> = items
        .iter()
        .enumerate()
        .filter_map(|(i, it)| if let vectorcraft_doc::AppearanceItem::Stroke(s) = it { Some((i, s.brush.is_some())) } else { None })
        .collect();
    let [(lower, true), (top, false)] = rows[..] else { panic!("{rows:?}") };
    assert!(!dimmed(&panel), "the top stroke has no brush");
    run(&mut panel.app(), "appearance.setActiveItem", json!({"index": lower}));
    assert!(dimmed(&panel));
    assert_eq!(listed(&mut panel), Vec::<&str>::new());
    run(&mut panel.app(), "appearance.setActiveItem", json!({"index": top}));
    assert!(!dimmed(&panel));
}

/// A selected group with a brushed stroke of its own dims the select, though the panel shows its
/// first member's plain stroke: the list edits the group's stroke.
#[test]
fn a_groups_own_brushed_stroke_dims_the_dropdown() {
    let mut panel = panel();
    let a = panel.app().session.active().unwrap().selection.objects[0].0;
    run(&mut panel.app(), "shape.line", json!({"x1": 10, "y1": 150, "x2": 150, "y2": 150}));
    let b = panel.app().session.active().unwrap().selection.objects[0].0;
    let g = run(&mut panel.app(), "object.group", json!({"ids": [a, b]}))["id"].as_u64().unwrap();
    run(&mut panel.app(), "appearance.addStroke", json!({"ids": [g]}));
    panel
        .app()
        .session
        .edit("Brush", |d, _| {
            d.node_mut(vectorcraft_doc::NodeId(g)).unwrap().appearance.stroke_mut().unwrap().brush = Some("3 pt. Round".into());
            Ok(())
        })
        .unwrap();
    run(&mut panel.app(), "select.set", json!({"ids": [g]}));
    assert!(current_stroke(&panel.app()).is_some_and(|s| s.brush.is_none()), "the panel shows a member's plain stroke");
    assert!(dimmed(&panel));
    assert_eq!(listed(&mut panel), Vec::<&str>::new());
}

/// A selection change with no edit between decides whether the select is dimmed: from a brushed
/// line to a plain one and back.
#[test]
fn selecting_another_object_without_an_edit_updates_the_dimming() {
    let panel = panel();
    let a = panel.app().session.active().unwrap().selection.objects[0].0;
    run(&mut panel.app(), "shape.line", json!({"x1": 10, "y1": 150, "x2": 150, "y2": 150}));
    let b = panel.app().session.active().unwrap().selection.objects[0].0;
    run(&mut panel.app(), "brush.apply", json!({"ids": [a], "name": "3 pt. Round"}));
    run(&mut panel.app(), "select.set", json!({"ids": [a]}));
    assert!(dimmed(&panel));
    run(&mut panel.app(), "select.set", json!({"ids": [b]}));
    assert!(!dimmed(&panel), "the plain line follows a profile");
    run(&mut panel.app(), "select.set", json!({"ids": [a]}));
    assert!(dimmed(&panel));
}

/// The Flip Along and Flip Across buttons beside a dimmed select do not flip a brushed stroke's
/// profile, which does not change what the stroke draws.
#[test]
fn flip_buttons_leave_a_brushed_strokes_profile_alone() {
    let mut panel = panel();
    set_profile(&mut panel.app(), custom());
    panel.frame();
    panel.click("Flip Along");
    let flipped = profile_points(&panel.app());
    assert_ne!(flipped, Some(custom().points), "Flip Along flips a plain stroke's profile");
    let id = panel.app().session.active().unwrap().selection.objects[0].0;
    run(&mut panel.app(), "brush.apply", json!({"ids": [id], "name": "3 pt. Round"}));
    panel.frame();
    assert!(dimmed(&panel));
    assert!(!panel.enabled("Flip Along") && !panel.enabled("Flip Across"), "the buttons are off on a brushed stroke");
    let before = profile_points(&panel.app());
    assert!(before.is_some());
}

/// With several objects selected, the select is dimmed only while every one of their strokes
/// draws with a brush: a profile still changes the strokes without one. A brush name the
/// document's brushes lack draws a plain stroke, which follows the profile.
#[test]
fn mixed_selection_dims_the_dropdown_only_when_every_stroke_has_a_brush() {
    let mut panel = panel();
    let a = panel.app().session.active().unwrap().selection.objects[0].0;
    run(&mut panel.app(), "shape.line", json!({"x1": 10, "y1": 150, "x2": 150, "y2": 150}));
    let b = panel.app().session.active().unwrap().selection.objects[0].0;
    run(&mut panel.app(), "brush.apply", json!({"ids": [a], "name": "3 pt. Round"}));
    // The first selected object is the brushed one, whose stroke the panel shows.
    run(&mut panel.app(), "select.set", json!({"ids": [a, b]}));
    assert!(current_stroke(&panel.app()).and_then(|s| s.brush).is_some());
    assert!(!dimmed(&panel));
    assert_eq!(listed(&mut panel).len(), WidthProfile::PRESETS.len());

    run(&mut panel.app(), "brush.apply", json!({"ids": [b], "name": "3 pt. Round"}));
    run(&mut panel.app(), "select.set", json!({"ids": [a, b]}));
    assert!(dimmed(&panel));
    assert_eq!(listed(&mut panel), Vec::<&str>::new());

    // The shown stroke is uniform and the other has a profile: Uniform still removes it.
    let edit = |panel: &Panel, f: &dyn Fn(&mut vectorcraft_doc::Appearance)| {
        panel
            .app()
            .session
            .edit("Edit", |d, _| {
                f(&mut d.node_mut(vectorcraft_doc::NodeId(b)).unwrap().appearance);
                Ok(())
            })
            .unwrap();
    };
    edit(&panel, &|ap| ap.stroke_mut().unwrap().profile = Some(custom()));
    assert_eq!(current_stroke(&panel.app()).and_then(|s| s.profile), None);
    assert_eq!(listed(&mut panel), ["Uniform"]);
    panel.click("Profile");
    panel.click("Uniform");
    let node = |panel: &Panel| panel.app().session.active().unwrap().doc.node(vectorcraft_doc::NodeId(b)).unwrap().clone();
    assert_eq!(node(&panel).appearance.stroke().unwrap().profile, None, "Uniform reset the other stroke");

    edit(&panel, &|ap| ap.stroke_mut().unwrap().brush = Some("No Such Brush".into()));
    assert!(!dimmed(&panel));

    // An object without a stroke item takes a plain stroke from a profile.
    edit(&panel, &|ap| ap.items.retain(|i| !matches!(i, vectorcraft_doc::AppearanceItem::Stroke(_))));
    assert!(node(&panel).appearance.stroke().is_none());
    assert!(!dimmed(&panel));
    assert_eq!(listed(&mut panel).len(), WidthProfile::PRESETS.len());
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
