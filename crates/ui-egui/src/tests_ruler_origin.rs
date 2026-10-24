//! #1212: real ruler gestures and position fields share the document's zero point.

use egui::{Event, Key, PointerButton, Pos2, Rect, Ui, pos2, vec2};
use serde_json::json;
use vectorcraft_doc::{NodeId, Unit};
use vectorcraft_engine::Session;
use vectorcraft_geom::Point;

use crate::{VectorcraftApp, canvas, chrome, panels};

fn app() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.ui.view.rulers = true;
    app.ui.control_bar = true;
    app
}

fn context() -> egui::Context {
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    ctx
}

fn frame(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<Event>, draw: fn(&mut VectorcraftApp, &mut Ui)) -> Vec<(String, Rect)> {
    let raw = egui::RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1600.0, 900.0))),
        time: Some(ctx.input(|i| i.time) + 0.05),
        events,
        ..Default::default()
    };
    let mut out = ctx.run_ui(raw, |ui| draw(app, ui));
    out.textures_delta.clear();
    fn walk(s: &egui::Shape, v: &mut Vec<(String, Rect)>) {
        match s {
            egui::Shape::Text(t) => v.push((t.galley.text().to_string(), Rect::from_min_size(t.pos, t.galley.size()))),
            egui::Shape::Vec(s) => s.iter().for_each(|s| walk(s, v)),
            _ => {}
        }
    }
    let mut texts = vec![];
    out.shapes.iter().for_each(|c| walk(&c.shape, &mut texts));
    texts
}

fn press(at: Pos2, pressed: bool) -> Event {
    Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() }
}

fn key(key: Key) -> Event {
    Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() }
}

fn start_drag(app: &mut VectorcraftApp, ctx: &egui::Context, to: Pos2) {
    let at = pos2(8.0, 8.0);
    frame(app, ctx, vec![Event::PointerMoved(at)], canvas::show);
    frame(app, ctx, vec![press(at, true)], canvas::show);
    frame(app, ctx, vec![Event::PointerMoved(to)], canvas::show);
}

#[test]
fn ruler_origin_drag_uses_the_rotated_zoomed_view_and_commits_only_on_release() {
    let mut app = app();
    app.select_tool("rectangle");
    let ctx = context();
    frame(&mut app, &ctx, vec![], canvas::show);
    let v = app.view_mut().unwrap();
    v.center = Point::new(220.0, 130.0);
    v.zoom = 2.3;
    v.rotation = 35.0;
    frame(&mut app, &ctx, vec![], canvas::show);
    let before = app.session.doc().unwrap().doc.clone();
    let undo = app.session.doc().unwrap().history.undo.len();
    let goal = Point::new(125.25, 90.5);
    let to = canvas::Xf::new(app.canvas_rect.unwrap(), app.view().unwrap()).to_screen(goal);
    start_drag(&mut app, &ctx, to);
    assert_eq!(&*app.session.doc().unwrap().doc, &*before, "preview does not edit the document");
    frame(&mut app, &ctx, vec![press(to, false)], canvas::show);
    let st = app.session.doc().unwrap();
    assert!(st.doc.ruler_origin_for(0).distance(goal) < 1e-4, "{:?}", st.doc.ruler_origin_for(0));
    assert_eq!(st.history.undo.len(), undo + 1);
    assert_eq!(st.doc.art_bounds(), None, "the drawing tool never receives the corner drag");
    assert!(st.doc.guides.is_empty());
    let origin = st.doc.ruler_origin_for(0);
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(&*app.session.doc().unwrap().doc, &*before);
    app.run("edit.redo", json!({})).unwrap();
    assert_eq!(app.ruler_origin(), origin);
}

#[test]
fn ruler_origin_double_click_resets_to_the_active_artboard() {
    let mut app = app();
    app.run("view.globalRulers", json!({"global": true})).unwrap();
    app.run("artboard.new", json!({"x": 700, "y": -250, "width": 200, "height": 150})).unwrap();
    app.run("document.setRulerOrigin", json!({"x": 125, "y": 90})).unwrap();
    let ctx = context();
    frame(&mut app, &ctx, vec![], canvas::show);
    app.view_mut().unwrap().artboard = 1;
    let at = pos2(8.0, 8.0);
    let undo = app.session.doc().unwrap().history.undo.len();
    for _ in 0..2 {
        frame(&mut app, &ctx, vec![Event::PointerMoved(at)], canvas::show);
        frame(&mut app, &ctx, vec![press(at, true)], canvas::show);
        frame(&mut app, &ctx, vec![press(at, false)], canvas::show);
    }
    assert_eq!(app.ruler_origin(), Point::new(700.0, -250.0));
    assert_eq!(app.session.doc().unwrap().history.undo.len(), undo + 1);
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(app.ruler_origin(), Point::new(125.0, 90.0));
}

#[test]
fn ruler_origin_cancelled_drags_do_not_modify_either_document() {
    #[derive(Debug, PartialEq)]
    enum Cancel {
        Outside,
        Escape,
        Focus,
        Document,
    }
    for cancel in [Cancel::Outside, Cancel::Escape, Cancel::Focus, Cancel::Document] {
        let mut app = app();
        app.run("document.setRulerOrigin", json!({"x": 25, "y": 30})).unwrap();
        let ctx = context();
        frame(&mut app, &ctx, vec![], canvas::show);
        let to = app.canvas_rect.unwrap().center();
        let before = app.session.doc().unwrap().doc.clone();
        let undo = app.session.doc().unwrap().history.undo.len();
        start_drag(&mut app, &ctx, to);
        let release = match cancel {
            Cancel::Outside => pos2(4.0, 4.0),
            Cancel::Escape => {
                frame(&mut app, &ctx, vec![key(Key::Escape)], canvas::show);
                to
            }
            Cancel::Focus => {
                let mut out = ctx.run_ui(egui::RawInput { focused: false, ..Default::default() }, |ui| canvas::show(&mut app, ui));
                out.textures_delta.clear();
                to
            }
            Cancel::Document => {
                app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
                to
            }
        };
        frame(&mut app, &ctx, vec![Event::PointerMoved(release), press(release, false)], canvas::show);
        if cancel == Cancel::Document {
            assert_eq!(app.ruler_origin(), Point::ZERO);
            app.run("document.activate", json!({"index": 0})).unwrap();
            assert_eq!(&*app.session.doc().unwrap().doc, &*before);
        } else {
            assert_eq!(&*app.session.doc().unwrap().doc, &*before, "{cancel:?}");
            assert_eq!(app.session.doc().unwrap().history.undo.len(), undo, "{cancel:?}");
        }
    }
}

#[test]
fn ruler_origin_position_fields_show_relative_values_and_edit_absolute_positions() {
    for draw in [panels::properties::show as fn(&mut VectorcraftApp, &mut Ui), panels::transform::show, chrome::control_bar] {
        for (label, arrow) in [("X:", (1.0, 0.0)), ("Y:", (0.0, 1.0))] {
            let mut app = app();
            let id = NodeId(app.run("shape.rectangle", json!({"x": 100, "y": 50, "width": 80, "height": 40})).unwrap()["id"].as_u64().unwrap());
            app.run("document.setRulerOrigin", json!({"x": 120, "y": 50})).unwrap();
            let ctx = context();
            frame(&mut app, &ctx, vec![], draw);
            let texts = frame(&mut app, &ctx, vec![], draw);
            assert!(texts.iter().filter(|(t, _)| t == &Unit::Points.format(20.0)).count() >= 2, "{texts:?}");
            let label = texts.iter().find(|(t, _)| t == label).unwrap().1;
            let at = pos2(label.right() + 40.0, label.center().y);
            frame(&mut app, &ctx, vec![Event::PointerMoved(at), press(at, true)], draw);
            frame(&mut app, &ctx, vec![press(at, false)], draw);
            frame(&mut app, &ctx, vec![key(Key::ArrowUp)], draw);
            let bounds = app.session.doc().unwrap().doc.node(id).unwrap().geometric_bounds().unwrap();
            assert_eq!((bounds.x0, bounds.y0), (100.0 + arrow.0, 50.0 + arrow.1));
            assert_eq!((bounds.width(), bounds.height()), (80.0, 40.0));
            app.run("edit.undo", json!({})).unwrap();
            assert_eq!(app.session.doc().unwrap().doc.node(id).unwrap().geometric_bounds().unwrap().x0, 100.0);
        }
    }
}

#[test]
fn ruler_origin_artboard_fields_preserve_the_absolute_reference_point() {
    for draw in [panels::properties::show as fn(&mut VectorcraftApp, &mut Ui), panels::transform::show] {
        for label in ["X:", "Y:"] {
            let mut app = app();
            app.run("document.setRulerOrigin", json!({"x": 120, "y": 50})).unwrap();
            app.select_tool("artboard");
            let ctx = context();
            frame(&mut app, &ctx, vec![], draw);
            let texts = frame(&mut app, &ctx, vec![], draw);
            let label_rect = texts.iter().find(|(t, _)| t == label).unwrap().1;
            let at = pos2(label_rect.right() + 40.0, label_rect.center().y);
            frame(&mut app, &ctx, vec![Event::PointerMoved(at), press(at, true)], draw);
            frame(&mut app, &ctx, vec![press(at, false)], draw);
            frame(&mut app, &ctx, vec![key(Key::ArrowUp)], draw);
            let ab = app.session.doc().unwrap().doc.artboards[0].rect;
            assert_eq!((ab.x0, ab.y0), if label == "X:" { (1.0, 0.0) } else { (0.0, 1.0) });
            assert_eq!((ab.width(), ab.height()), (400.0, 300.0));
        }
    }
}

// View › Rulers › Change to Global / Artboard Rulers (Adobe's public help:
// https://helpx.adobe.com/illustrator/desktop/measure-and-align/grids-and-guides/about-rulers.html):
// artboard rulers, a new document's, count from the active artboard's top-left corner; global
// rulers from one zero point for the whole document.

/// A document with a second artboard at (700, -250) and a rectangle on it at (720, -230).
fn two_boards() -> (VectorcraftApp, NodeId) {
    let mut app = app();
    app.run("artboard.new", json!({"x": 700, "y": -250, "width": 200, "height": 150})).unwrap();
    let id = NodeId(app.run("shape.rectangle", json!({"x": 720, "y": -230, "width": 80, "height": 40})).unwrap()["id"].as_u64().unwrap());
    (app, id)
}

#[test]
fn artboard_rulers_position_fields_count_from_the_active_artboard() {
    for draw in [panels::properties::show as fn(&mut VectorcraftApp, &mut Ui), panels::transform::show, chrome::control_bar] {
        let (mut app, id) = two_boards();
        let ctx = context();
        frame(&mut app, &ctx, vec![], draw);
        app.view_mut().unwrap().artboard = 1;
        assert_eq!(app.ruler_origin(), Point::new(700.0, -250.0));
        let texts = frame(&mut app, &ctx, vec![], draw);
        // The reference point is the centre (760, -210): 60 and 40 from artboard 2's corner.
        for v in [60.0, 40.0] {
            assert!(texts.iter().any(|(t, _)| t == &Unit::Points.format(v)), "X and Y count from artboard 2: {texts:?}");
        }
        // Typing a position puts the object there on the active artboard's rulers.
        let label = texts.iter().find(|(t, _)| t == "X:").unwrap().1;
        let at = pos2(label.right() + 40.0, label.center().y);
        frame(&mut app, &ctx, vec![Event::PointerMoved(at), press(at, true)], draw);
        frame(&mut app, &ctx, vec![press(at, false)], draw);
        frame(&mut app, &ctx, vec![key(Key::ArrowUp)], draw);
        let bounds = app.session.doc().unwrap().doc.node(id).unwrap().geometric_bounds().unwrap();
        assert_eq!((bounds.x0, bounds.y0), (721.0, -230.0));
        // Global rulers: the same object reads from the document's zero point.
        app.run("view.globalRulers", json!({"global": true})).unwrap();
        assert_eq!(app.ruler_origin(), Point::ZERO);
        frame(&mut app, &ctx, vec![], draw);
        let texts = frame(&mut app, &ctx, vec![], draw);
        assert!(texts.iter().any(|(t, _)| t == &Unit::Points.format(761.0)), "{texts:?}");
        assert!(!texts.iter().any(|(t, _)| t == &Unit::Points.format(61.0)), "{texts:?}");
    }
}

#[test]
fn ruler_origin_drag_with_artboard_rulers_sets_the_active_artboards_origin() {
    let (mut app, _) = two_boards();
    app.select_tool("rectangle");
    let ctx = context();
    frame(&mut app, &ctx, vec![], canvas::show);
    app.view_mut().unwrap().artboard = 1;
    let v = app.view_mut().unwrap();
    v.center = Point::new(800.0, -175.0);
    frame(&mut app, &ctx, vec![], canvas::show);
    let goal = Point::new(760.0, -200.0);
    let to = canvas::Xf::new(app.canvas_rect.unwrap(), app.view().unwrap()).to_screen(goal);
    start_drag(&mut app, &ctx, to);
    frame(&mut app, &ctx, vec![press(to, false)], canvas::show);
    let doc = app.session.doc().unwrap().doc.clone();
    assert!(doc.ruler_origin_for(1).distance(goal) < 1e-4, "{:?}", doc.ruler_origin_for(1));
    assert_eq!(doc.ruler_origin_for(0), Point::ZERO, "the first artboard keeps its own");
    assert_eq!(doc.ruler_origin, Point::ZERO, "the global zero point is apart");
    assert!(app.ruler_origin().distance(goal) < 1e-4);
    app.view_mut().unwrap().artboard = 0;
    assert_eq!(app.ruler_origin(), Point::ZERO);
}

#[test]
fn change_to_global_rulers_menu_item_toggles_and_names_the_other_mode() {
    let (mut app, _) = two_boards();
    let item = crate::menus::menu_tree().iter().find(|(title, _)| *title == "View").map(|(_, items)| format!("{items:?}")).unwrap();
    assert!(item.contains("view.globalRulers"), "View › Rulers lists it: {item}");
    assert_eq!(crate::menus::dynamic_label(&app, "view.globalRulers", "Change to Global Rulers"), "Change to Global Rulers");
    app.run("view.globalRulers", json!({})).unwrap();
    assert_eq!(app.session.doc().unwrap().doc.rulers, vectorcraft_doc::RulerMode::Global);
    assert_eq!(crate::menus::dynamic_label(&app, "view.globalRulers", "Change to Global Rulers"), "Change to Artboard Rulers");
    app.run("view.globalRulers", json!({})).unwrap();
    assert_eq!(app.session.doc().unwrap().doc.rulers, vectorcraft_doc::RulerMode::Artboard);
    let spec = vectorcraft_engine::command_specs().iter().find(|c| c.id == "view.globalRulers").unwrap();
    assert_eq!(spec.shortcut, Some("Cmd+Alt+R"));
}

/// The top ruler's labels and where each starts on screen.
fn top_labels(app: &mut VectorcraftApp, ctx: &egui::Context) -> Vec<(String, f32)> {
    frame(app, ctx, vec![], canvas::show).into_iter().filter(|(_, r)| r.min.y < 4.0 && r.min.x > 16.0).map(|(t, r)| (t, r.min.x)).collect()
}

/// Where the top ruler's label `text` starts, if it shows.
fn label_x(labels: &[(String, f32)], text: &str) -> Option<f32> {
    labels.iter().find(|(t, _)| t == text).map(|(_, x)| *x)
}

/// #2849: View › Rulers › Show Video Rulers shows the rulers counting the active artboard in video
/// pixels from its top-left corner, whatever the ruler origin, a pixel as wide as the artboard's
/// Video Ruler Pixel Aspect Ratio: at 0.5 the 400 pt artboard reads 800 at its right edge. The
/// item then reads Hide Video Rulers, which puts the rulers back in points from the ruler origin;
/// they follow the active artboard, and hiding the rulers names the item Show Video Rulers again.
#[test]
fn show_video_rulers_counts_the_active_artboard_in_pixels() {
    let (mut app, _) = two_boards();
    let ctx = context();
    let tree = format!("{:?}", crate::menus::menu_tree().iter().find(|(title, _)| *title == "View").unwrap().1);
    assert!(tree.contains("\"Show Video Rulers\", \"view.videoRulers\"") && !tree.contains("Todo(\"Show Video Rulers\""), "{tree}");
    app.run("artboard.setProps", json!({"index": 0, "videoRulerPixelAspectRatio": 0.5})).unwrap();
    app.run("document.setRulerOrigin", json!({"x": 100, "y": 50, "artboard": 0})).unwrap();
    app.ui.view.rulers = false;
    let label = |app: &VectorcraftApp| crate::menus::dynamic_label(app, "view.videoRulers", "Show Video Rulers");
    assert_eq!(label(&app), "Show Video Rulers");
    assert_eq!(app.run("view.videoRulers", json!({})).unwrap(), json!(true));
    assert!(app.ui.view.rulers && app.ui.view.video_rulers, "the rulers show");
    assert_eq!(label(&app), "Hide Video Rulers");
    let at_100 = |app: &mut VectorcraftApp, center: Point| {
        let v = app.view_mut().unwrap();
        (v.zoom, v.center, v.fitted, v.rotation) = (1.0, center, true, 0.0);
    };
    at_100(&mut app, Point::new(200.0, 150.0));
    // How far label `text` of `labels` starts right of `zero`, to within a hundredth of a point.
    let from = |labels: &[(String, f32)], text: &str, zero: f32, want: f32| label_x(labels, text).is_some_and(|x| (x - zero - want).abs() < 0.01);
    let video = top_labels(&mut app, &ctx);
    let zero = label_x(&video, "0").unwrap();
    assert!(from(&video, "800", zero, 400.0), "800 pixels at the right edge: {video:?}");
    assert!(from(&video, "100", zero, 50.0), "{video:?}");
    // Hide Video Rulers: points from the ruler origin, 100 pt in from the artboard's left edge.
    assert_eq!(app.run("view.videoRulers", json!({})).unwrap(), json!(false));
    assert!(app.ui.view.rulers && !app.ui.view.video_rulers, "the rulers stay");
    assert_eq!(label(&app), "Show Video Rulers");
    let points = top_labels(&mut app, &ctx);
    assert!(from(&points, "0", zero, 100.0), "{points:?}");
    assert!(from(&points, "300", zero, 400.0), "{points:?}");
    // The second artboard (square pixels, 200 pt wide at x = 700) once it is the active one.
    app.run("view.videoRulers", json!({})).unwrap();
    app.view_mut().unwrap().artboard = 1;
    at_100(&mut app, Point::new(800.0, -175.0));
    let second = top_labels(&mut app, &ctx);
    let zero = label_x(&second, "0").unwrap();
    assert!(from(&second, "200", zero, 200.0), "{second:?}");
    // Hidden with Show Rulers, the item offers to show them again.
    app.run("view.rulers", json!({})).unwrap();
    assert_eq!(label(&app), "Show Video Rulers");
    assert!(top_labels(&mut app, &ctx).is_empty());
}
