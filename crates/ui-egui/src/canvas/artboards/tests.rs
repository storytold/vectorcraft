use craft::ui::canvas::{PageBorder, PageFill, PasteboardColor};
use serde_json::json;
use vectorcraft_engine::Session;

use super::*;
use crate::menus;

fn app_with(boards: usize) -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 200, "artboards": boards})).unwrap();
    app
}

fn frames_of(app: &VectorcraftApp, outline: bool) -> Vec<PageFrame> {
    let st = app.session.active().unwrap();
    frames(app, st, app.view().unwrap(), outline)
}

fn use_tool(app: &mut VectorcraftApp, id: &str) {
    let info = app.view_info();
    app.session.select_tool(id, info).unwrap();
}

#[test]
fn the_artboard_tool_names_its_active_artboard_once() {
    let mut app = app_with(2);
    let names: Vec<_> = frames_of(&app, false).into_iter().map(|f| f.name).collect();
    assert_eq!(names.len(), 2);
    assert!(names.iter().enumerate().all(|(i, n)| n.as_deref().is_some_and(|n| n.starts_with(&format!("{:02} - ", i + 1)))));
    let mut single = app_with(1);
    use_tool(&mut single, "selection");
    assert!(frames_of(&single, false).iter().all(|f| f.name.is_none()));
    use_tool(&mut single, "artboard");
    assert!(frames_of(&single, false).iter().all(|f| f.name.is_some()));
    use_tool(&mut app, "artboard");
    assert_eq!(frames_of(&app, false).iter().filter(|f| f.name.is_some()).count(), 2);
}

#[test]
fn shift_clicking_a_name_toggles_selection_and_a_plain_click_activates() {
    let mut app = app_with(2);
    use_tool(&mut app, "artboard");
    select_named_artboard(&mut app, 1, false);
    assert_eq!(app.session.tool_options()["active"], json!(1));
    select_named_artboard(&mut app, 0, true);
    assert!(app.session.tool_options()["selected"].as_array().is_some_and(|a| a.contains(&json!(0))));
}

#[test]
fn outline_has_a_white_canvas_line_edges_and_no_paper() {
    let app = app_with(2);
    let st = app.session.active().unwrap();
    assert_eq!(pasteboard_color(&app, st, true), PasteboardColor::Custom(DataColor::new(Color32::WHITE)));
    assert_eq!(border(true), PageBorder::Line);
    assert!(frames_of(&app, true).iter().all(|f| f.fill.is_none()));
    assert_eq!(pasteboard_color(&app, st, false), PasteboardColor::Default);
    assert_eq!(border(false), PageBorder::DropShadow);
    assert!(frames_of(&app, false).iter().all(|f| f.fill.is_some()));
}

/// User Interface › Canvas Color paints the pasteboard in Preview; Outline keeps its white one.
#[test]
fn the_canvas_color_preference_paints_the_pasteboard() {
    let mut app = app_with(1);
    app.run("prefs.set", json!({"key": "canvasColor", "value": "darkGray"})).unwrap();
    let st = app.session.active().unwrap();
    assert_eq!(pasteboard_color(&app, st, false), PasteboardColor::Custom(DataColor::new(Color32::from_gray(0x5a))));
    assert_eq!(pasteboard_color(&app, st, true), PasteboardColor::Custom(DataColor::new(Color32::WHITE)));
}

#[test]
fn hidden_artboards_leave_a_paper_canvas_and_the_artboard_tool_brings_them_back() {
    let mut app = app_with(2);
    app.ui.view.artboards = false;
    assert!(frames_of(&app, false).is_empty());
    let st = app.session.active().unwrap();
    assert_eq!(pasteboard_color(&app, st, false), PasteboardColor::Custom(DataColor::new(paper_color(&st.doc.setup))));
    use_tool(&mut app, "artboard");
    assert_eq!(frames_of(&app, false).len(), 2);
}

#[test]
fn the_fade_follows_the_tool_presentation_mode_and_drag_state() {
    let mut app = app_with(1);
    app.session.prefs.fade_region_outside_artboard = true;
    assert!(!fades_outside_artboards(&app));
    use_tool(&mut app, "artboard");
    assert!(fades_outside_artboards(&app));
    app.ui.screen_mode = 3;
    assert!(!fades_outside_artboards(&app));
    app.ui.screen_mode = 0;
    app.session.prefs.artboard_update_while_dragging = false;
    assert_eq!(fades_outside_artboards(&app), !app.session.tool_transforming());
}

#[test]
fn the_fade_covers_the_pasteboard_and_not_the_boards() {
    let app = app_with(2);
    let frames = frames_of(&app, false);
    // No canvas has shown the view: it is shown in an 800 × 600 point area.
    let view = app.view().unwrap().canvas(app.ui_scaling(), egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0)));
    let quads = fade_quads(&view, &frames);
    let covers = |p: Point| {
        let s = view.to_screen(p);
        quads.iter().any(|q| {
            let (lo, hi) = (
                q.iter().fold(egui::Pos2::new(f32::MAX, f32::MAX), |a, p| a.min(*p)),
                q.iter().fold(egui::Pos2::new(f32::MIN, f32::MIN), |a, p| a.max(*p)),
            );
            s.x > lo.x && s.x < hi.x && s.y > lo.y && s.y < hi.y
        })
    };
    for f in &frames {
        assert!(!covers(Point::new(f.rect.center().x, f.rect.center().y)));
    }
    // The fade is drawn over the visible pasteboard only: a point just inside the canvas's corner.
    assert!(covers(view.to_doc(view.rect.min + egui::vec2(1.0, 1.0))));
}

#[test]
fn a_rename_commit_renames_with_the_trimmed_name_and_empty_or_same_sends_nothing() {
    let mut app = app_with(1);
    assert!(frames_of(&app, false).iter().all(|f| f.renames));
    commit_artboard_name(&mut app, 0, "  Cover  ");
    assert_eq!(app.session.active().unwrap().doc.artboards[0].name, "Cover");
    commit_artboard_name(&mut app, 0, "   ");
    commit_artboard_name(&mut app, 0, "Cover");
    assert_eq!(app.session.active().unwrap().doc.artboards[0].name, "Cover");
}

#[test]
fn a_locked_artboards_name_does_not_rename() {
    let mut app = app_with(1);
    app.run("artboard.setProps", json!({"index": 0, "locked": true})).unwrap();
    assert!(frames_of(&app, false).iter().all(|f| !f.renames));
}

#[test]
fn paper_is_the_artboard_color_times_the_simulated_paper() {
    let mut app = app_with(1);
    let st = app.session.active().unwrap();
    assert_eq!(paper_color(&st.doc.setup), Color32::WHITE);
    let mut ab = st.doc.artboards[0].clone();
    ab.color = Some(vectorcraft_color::Color::rgb(0.2, 0.4, 0.6));
    let paper = Color32::from_rgb(255, 238, 204);
    assert_eq!(artboard_paper(&ab, paper), Color32::from_rgb(51, 95, 122));
    assert_eq!(artboard_paper(&ab, Color32::WHITE), Color32::from_rgb(51, 102, 153));
    // The frame carries the same fill.
    app.run("view.transparencyGrid", json!({})).unwrap();
    assert!(frames_of(&app, false).iter().all(|f| matches!(f.fill, Some(PageFill::Checker(_)))));
}

#[test]
fn marks_follow_the_artboard_options_at_any_zoom() {
    let mut app = app_with(1);
    let st = app.session.active().unwrap();
    assert!(mark_paths(st, 1.0).is_empty() && bleed_paths(st).is_empty());
    app.run("artboard.setProps", json!({"index": 0, "showCrossHairs": true, "showCenterMark": true, "showVideoSafeAreas": true})).unwrap();
    let st = app.session.active().unwrap();
    assert_eq!(mark_paths(st, 1.0).len(), 2 + 2 + 2);
    assert_eq!(mark_paths(st, 4.0).len(), 6);
}

#[test]
fn the_transparency_grid_follows_each_document_and_the_view_menu_label() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    assert!(!shows_transparency_grid(app.session.active().unwrap()));
    assert_eq!(menus::dynamic_label(&app, "view.transparencyGrid", ""), "Show Transparency Grid");
    assert_eq!(app.run("view.transparencyGrid", json!({})).unwrap(), json!({"on": true}));
    assert!(shows_transparency_grid(app.session.active().unwrap()));
    assert!(frames_of(&app, false).iter().all(|f| matches!(f.fill, Some(PageFill::Checker(_)))));
    assert_eq!(menus::dynamic_label(&app, "view.transparencyGrid", ""), "Hide Transparency Grid");
    app.run("document.activate", json!({"index": 0})).unwrap();
    assert!(!shows_transparency_grid(app.session.active().unwrap()));
    assert_eq!(menus::dynamic_label(&app, "view.transparencyGrid", ""), "Show Transparency Grid");
    app.run("document.activate", json!({"index": 1})).unwrap();
    assert!(shows_transparency_grid(app.session.active().unwrap()));
}
