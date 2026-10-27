//! Paste in the app: the Swatch Conflict dialog, pasting at the view centre, the Paste menu items
//! enabled by SVG on the system clipboard, and the Layers panel's Paste Remembers Layers.

use serde_json::{Value, json};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::NodeId;
use vectorcraft_engine::Session;
use vectorcraft_geom::Point;

use crate::{Services, VectorcraftApp, dialogs, menus};

fn run(app: &mut VectorcraftApp, id: &str, p: Value) -> Value {
    app.run(id, p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

fn rect(app: &mut VectorcraftApp) -> NodeId {
    NodeId(run(app, "shape.rectangle", json!({"x": 10, "y": 10, "width": 40, "height": 20}))["id"].as_u64().unwrap())
}

fn copy(app: &mut VectorcraftApp, id: NodeId) {
    run(app, "select.set", json!({"ids": [id.0]}));
    run(app, "edit.copy", json!({}));
}

/// Objects on the active document's first layer.
fn objects(app: &VectorcraftApp) -> Vec<NodeId> {
    app.session.active().unwrap().doc.layers[0].children().unwrap().iter().map(|n| n.id).collect()
}

fn swatch(app: &mut VectorcraftApp, name: &str, hex: &str) {
    run(app, "swatch.new", json!({"name": name, "color": hex, "global": true}));
}

/// A rectangle filled with global swatch Brand (red) and stroked with Ink (black) copied from one
/// document, and a second document (active) whose Brand is blue and Ink green.
fn conflicting() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    run(&mut app, "file.new", json!({"width": 200, "height": 200}));
    swatch(&mut app, "Brand", "#ff0000");
    swatch(&mut app, "Ink", "#000000");
    let r = rect(&mut app);
    run(&mut app, "paint.setFill", json!({"ids": [r.0], "swatch": "Brand"}));
    run(&mut app, "paint.setStroke", json!({"ids": [r.0], "swatch": "Ink"}));
    copy(&mut app, r);
    run(&mut app, "file.new", json!({"width": 200, "height": 200}));
    swatch(&mut app, "Brand", "#0000ff");
    swatch(&mut app, "Ink", "#00ff00");
    app
}

fn paints(app: &VectorcraftApp) -> (Paint, Paint) {
    let st = app.session.active().unwrap();
    let n = st.doc.node(objects(app)[0]).unwrap();
    (n.appearance.fill_paint(), n.appearance.stroke().unwrap().paint.clone())
}

fn linked(hex: &str, name: &str) -> Paint {
    Paint::Solid { color: Color::from_hex(hex).unwrap(), swatch: Some(name.into()), tint: 1.0 }
}

fn set(app: &mut VectorcraftApp, k: &str, v: Value) {
    app.ui.dialog.as_mut().unwrap().fields.insert(k.into(), v);
}

#[test]
fn the_swatch_conflict_dialog_asks_about_each_swatch_then_pastes() {
    let mut app = conflicting();
    assert_eq!(run(&mut app, "edit.paste", json!({})), json!({"dialog": dialogs::swatch_conflict::KIND}));
    assert!(objects(&app).is_empty(), "nothing is pasted before the answer");
    // It draws headlessly, naming the swatch it asks about.
    let text = crate::tests_labels::painted_text(&mut app, |app, ui| dialogs::show(app, ui.ctx()));
    for label in ["Swatch Conflict", "\u{201c}Brand\u{201d}", "Conflict 1 of 2", "Merge Swatches", "Add Swatches", "Apply to All"] {
        assert!(text.contains(label), "{label} in {text}");
    }
    // Brand: add; Ink: merge.
    set(&mut app, "choice", json!("add"));
    dialogs::confirm(&mut app).unwrap();
    assert_eq!(app.ui.dialog.as_ref().unwrap().fields["index"], 1, "the next conflict");
    assert!(objects(&app).is_empty());
    set(&mut app, "choice", json!("merge"));
    dialogs::confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none());
    assert_eq!(paints(&app), (linked("#ff0000", "Brand 2"), linked("#00ff00", "Ink")));
    // One undo step takes the object and the added swatch away.
    run(&mut app, "edit.undo", json!({}));
    assert!(objects(&app).is_empty() && app.session.active().unwrap().doc.swatch("Brand 2").is_none());
}

#[test]
fn apply_to_all_answers_the_rest_and_cancel_pastes_nothing() {
    let mut app = conflicting();
    run(&mut app, "edit.paste", json!({}));
    dialogs::cancel(&mut app);
    assert!(app.ui.dialog.is_none() && objects(&app).is_empty());
    run(&mut app, "edit.pasteInPlace", json!({}));
    set(&mut app, "choice", json!("add"));
    set(&mut app, "applyToAll", json!(true));
    dialogs::confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none());
    assert_eq!(paints(&app), (linked("#ff0000", "Brand 2"), linked("#000000", "Ink 2")));
    // An answer given with the command needs no dialog (agents).
    let v = run(&mut app, "edit.pasteInPlace", json!({"swatchConflict": "merge"}));
    assert_eq!(v["merged"], 2);
    assert!(app.ui.dialog.is_none());
}

#[test]
fn paste_goes_to_the_centre_of_the_view() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    run(&mut app, "file.new", json!({"width": 400, "height": 300}));
    let r = rect(&mut app);
    copy(&mut app, r);
    let v = app.view_mut().unwrap();
    (v.center, v.fitted) = (Point::new(250.0, 180.0), true);
    let ids: Vec<NodeId> = run(&mut app, "edit.paste", json!({}))["ids"].as_array().unwrap().iter().map(|i| NodeId(i.as_u64().unwrap())).collect();
    let b = app.session.active().unwrap().doc.bounds_of(&ids, true).unwrap();
    assert!((b.center() - Point::new(250.0, 180.0)).hypot() < 1e-6, "{b:?}");
}

const SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 10"><rect width="5" height="5"/><circle cx="15" cy="5" r="5"/></svg>"##;

/// An app whose system clipboard holds `text`, with a document, after one frame.
fn with_system_clipboard(text: &'static str) -> VectorcraftApp {
    let services = Services { clipboard_read: Some(Box::new(move || Some(text.to_string()))), ..Default::default() };
    let mut app = VectorcraftApp::new(Session::new(), services);
    run(&mut app, "file.new", json!({"width": 200, "height": 200}));
    assert!(!menus::enabled(&app, "edit.paste"), "not checked before a frame");
    let ctx = egui::Context::default();
    ctx.run_ui(Default::default(), |ui| app.logic(ui.ctx())).textures_delta.clear();
    app
}

#[test]
fn paste_menu_items_are_enabled_by_svg_on_the_system_clipboard() {
    let mut app = with_system_clipboard(SVG);
    assert!(app.session.clipboard.is_empty());
    for id in ["edit.paste", "edit.pasteInFront", "edit.pasteInBack", "edit.pasteInPlace", "edit.pasteOnAllArtboards", "edit.pasteWithoutFormatting"]
    {
        assert!(menus::enabled(&app, id), "{id}");
    }
    let entry = menus::menu_entries(&app).into_iter().find(|e| e.command.as_deref() == Some("edit.paste")).unwrap();
    assert!(entry.enabled);
    // Choosing it pastes the SVG's objects.
    menus::invoke(&mut app, "edit.paste", json!({}));
    assert_eq!(objects(&app).len(), 2);
    // Plain text is not something to paste.
    let app = with_system_clipboard("plain text");
    assert!(!menus::enabled(&app, "edit.paste"));
}

#[test]
fn the_layers_panel_menu_toggles_paste_remembers_layers() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    run(&mut app, "file.new", json!({"width": 200, "height": 200}));
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let frame = |app: &mut VectorcraftApp, events: Vec<egui::Event>| {
        let mut out = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| crate::panels::layers::menu(app, ui));
        out.textures_delta.clear();
        let mut texts = vec![];
        for c in &out.shapes {
            if let egui::Shape::Text(t) = &c.shape {
                texts.push((t.galley.text().to_string(), t.visual_bounding_rect()));
            }
        }
        texts
    };
    let item = |texts: &[(String, egui::Rect)]| texts.iter().find(|(t, _)| t.ends_with("Paste Remembers Layers")).cloned().unwrap();
    let (label, r) = item(&frame(&mut app, vec![]));
    assert!(!label.starts_with('✓'));
    let button =
        |pressed| egui::Event::PointerButton { pos: r.center(), button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    for e in [egui::Event::PointerMoved(r.center()), button(true), button(false)] {
        frame(&mut app, vec![e]);
    }
    assert!(app.session.active().unwrap().doc.paste_remembers_layers);
    assert!(item(&frame(&mut app, vec![])).0.starts_with('✓'), "checked");
}
