//! The colour adjustment effect dialogs (Effect → Color Adjustments) and Object → Vector Halftone
//! in the menus.

use serde_json::json;
use vectorcraft_engine::Session;

use crate::{VectorcraftApp, dialogs, menus, tests_labels::painted_text};

fn app() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap();
    app
}

fn dialog_text(app: &mut VectorcraftApp) -> String {
    painted_text(app, |app, ui| dialogs::show(app, ui.ctx()))
}

#[test]
fn the_effect_menu_lists_the_adjustments_and_the_object_menu_the_halftone() {
    let tree = menus::menu_tree();
    let items = |menu: &str| format!("{:?}", tree.iter().find(|(t, _)| *t == menu).unwrap().1);
    let effect = items("Effect");
    for label in ["Color Adjustments", "Brightness/Contrast…", "Curves…", "Hue/Saturation…", "Levels…", "Shift to Color…", "Temperature/Tint…"]
    {
        assert!(effect.contains(label), "{label}");
    }
    assert!(items("Object").contains("Vector Halftone…"));
}

#[test]
fn film_grain_dialog_previews_cancels_and_edits_as_one_undo_step() {
    let mut app = app();
    app.run("effect.dialog", json!({"effect": "artistic.filmGrain"})).unwrap();
    let text = dialog_text(&mut app);
    for label in ["Film Grain", "Grain", "Highlight Area", "Intensity", "Preview"] {
        assert!(text.lines().any(|line| line == label), "{label} in {text}");
    }
    assert!(app.session.in_interaction());
    dialogs::cancel(&mut app);
    let effects = |app: &VectorcraftApp| {
        let st = app.session.active().unwrap();
        st.doc.node(st.selection.objects[0]).unwrap().appearance.effects.clone()
    };
    assert!(effects(&app).is_empty(), "Cancel removes the preview");
    app.run("effect.dialog", json!({"effect": "artistic.filmGrain"})).unwrap();
    dialog_text(&mut app);
    app.ui.dialog.as_mut().unwrap().fields.insert("grain".into(), json!(12));
    dialog_text(&mut app);
    let before = app.session.doc().unwrap().history.undo.len();
    dialogs::confirm(&mut app).unwrap();
    assert_eq!(app.session.doc().unwrap().history.undo.len(), before + 1);
    assert_eq!(effects(&app)[0].params["grain"], 12);
    app.run("effect.dialog", json!({"effect": "artistic.filmGrain", "index": 0})).unwrap();
    assert_eq!(app.ui.dialog.as_ref().unwrap().f64("grain", 0.0), 12.0);
    app.ui.dialog.as_mut().unwrap().fields.insert("grain".into(), json!(7));
    dialog_text(&mut app);
    dialogs::confirm(&mut app).unwrap();
    assert_eq!(effects(&app).len(), 1, "editing doesn't add a second effect");
    assert_eq!(effects(&app)[0].params["grain"], 7);
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(effects(&app)[0].params["grain"], 12);
}

#[test]
fn an_adjustment_dialog_has_sliders_and_previews_live() {
    let mut app = app();
    app.run("effect.dialog", json!({"effect": "adjust.hueSaturation"})).unwrap();
    let text = dialog_text(&mut app);
    for s in ["Hue/Saturation", "Hue", "Saturation", "Lightness", "Colorize", "Preview"] {
        assert!(text.lines().any(|l| l == s), "{s} in {text}");
    }
    assert!(app.session.in_interaction(), "the preview runs");
    // A value set from outside (ui.dialog.set) previews too.
    app.ui.dialog.as_mut().unwrap().fields.insert("hue".into(), json!(90));
    dialog_text(&mut app);
    let before = app.session.doc().unwrap().history.undo.len();
    dialogs::confirm(&mut app).unwrap();
    assert!(!app.session.in_interaction());
    assert_eq!(app.session.doc().unwrap().history.undo.len(), before + 1, "one undo step");
    let st = app.session.active().unwrap();
    let n = st.doc.node(st.selection.objects[0]).unwrap();
    assert_eq!(n.appearance.effects[0].id, "adjust.hueSaturation");
    assert_eq!(n.appearance.effects[0].params["hue"], 90.0);
}

#[test]
fn levels_and_curves_show_their_channel_and_values() {
    let mut app = app();
    app.run("effect.dialog", json!({"effect": "adjust.levels"})).unwrap();
    let text = dialog_text(&mut app);
    for s in ["Input Black", "Output White", "Channel", "RGB", "Gamma"] {
        assert!(text.lines().any(|l| l == s), "{s} in {text}");
    }
    app.ui.dialog = None;
    let _ = app.session.cancel_interaction();
    app.run("effect.dialog", json!({"effect": "adjust.curves"})).unwrap();
    let text = dialog_text(&mut app);
    assert!(text.lines().any(|l| l == "Points") && text.contains("0,0 128,128 255,255"), "{text}");
}

#[test]
fn dragging_on_the_curves_graph_adds_and_moves_a_point() {
    use egui::{Event, PointerButton, Pos2, Rect, vec2};
    let mut app = app();
    app.run("effect.dialog", json!({"effect": "adjust.curves"})).unwrap();
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let screen_rect = Some(Rect::from_min_size(Pos2::ZERO, vec2(1400.0, 1000.0)));
    let frame = |app: &mut VectorcraftApp, events: Vec<Event>| {
        let input = egui::RawInput { events, screen_rect, ..Default::default() };
        ctx.run_ui(input, |ui| dialogs::show(app, ui.ctx())).textures_delta.clear();
    };
    for _ in 0..3 {
        frame(&mut app, vec![]);
    }
    let graph =
        ctx.viewport(|vp| vp.prev_pass.widgets.layers().flat_map(|(_, w)| w.iter()).find(|w| w.rect.size() == vec2(200.0, 200.0)).map(|w| w.rect));
    let graph = graph.expect("the curve graph");
    // Press above the identity line a quarter of the way in and drag up: a new point, raised.
    let at = |x: f32, y: f32| Pos2::new(graph.left() + x * 200.0, graph.bottom() - y * 200.0);
    let (from, to) = (at(0.25, 0.35), at(0.25, 0.6));
    let button = |pos, pressed| Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Default::default() };
    frame(&mut app, vec![Event::PointerMoved(from), button(from, true)]);
    for k in 1..=5 {
        frame(&mut app, vec![Event::PointerMoved(from + (to - from) * (k as f32 / 5.0))]);
    }
    frame(&mut app, vec![button(to, false)]);
    let points = app.ui.dialog.as_ref().unwrap().str("points");
    let pts: Vec<(f32, f32)> = vectorcraft_effects::curve_points(Some(&json!(points)));
    assert_eq!(pts.len(), 4, "{points}");
    let (x, y) = pts[1];
    assert!((x - 0.25).abs() < 0.01 && (y - 0.6).abs() < 0.01, "{points}");
}

#[test]
fn the_halftone_menu_item_opens_a_previewing_dialog() {
    let mut app = app();
    app.run("paint.setFill", json!({"color": "#000000"})).unwrap();
    let id = app.session.active().unwrap().selection.objects[0];
    menus::invoke(&mut app, "object.vectorHalftone", json!({}));
    assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some(dialogs::halftone::KIND));
    // Agents open it the same way.
    app.ui.dialog = None;
    app.run("ui.menuDialog", json!({"command": "object.vectorHalftone"})).unwrap();
    assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some(dialogs::halftone::KIND));
    let text = dialog_text(&mut app);
    for s in ["Vector Halftone", "Shape", "Circle", "Frequency", "Angle", "Screens", "Mono", "Color", "Clip to Art", "Keep Original"] {
        assert!(text.lines().any(|l| l == s), "{s} in {text}");
    }
    assert!(app.session.in_interaction(), "the preview runs");
    assert!(app.session.active().unwrap().doc.node(id).is_none(), "the preview shows the halftone in place of the art");
    app.ui.dialog.as_mut().unwrap().fields.insert("mode".into(), json!("cmyk"));
    assert!(!dialog_text(&mut app).lines().any(|l| l == "Color"), "CMYK screens have their own inks");
    let undo = app.session.doc().unwrap().history.undo.len();
    dialogs::confirm(&mut app).unwrap();
    assert!(!app.session.in_interaction());
    assert_eq!(app.session.doc().unwrap().history.undo.len(), undo + 1);
    let st = app.session.active().unwrap();
    assert_eq!(st.doc.node(st.selection.objects[0]).unwrap().name.as_deref(), Some("Vector Halftone"));
}
