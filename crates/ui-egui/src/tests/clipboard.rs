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

static TEST_CLIP_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn copying_text_carries_formatting_across_documents() {
    let _guard = TEST_CLIP_LOCK.lock().unwrap();
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    // Document 1
    run(&mut app, "file.new", json!({"width": 200, "height": 200}));
    let id1 = run(&mut app, "text.create", json!({"x": 20, "y": 40, "text": "Styled Text", "size": 36, "color": "#ff0000"}))["id"].as_u64().unwrap();
    let v = app.view_info();
    app.session.select_tool("type", v).unwrap();
    app.session.set_tool_option("textSelection", &json!({"id": id1, "start": 0, "end": 6}));
    crate::panels::character::intercept_text_command(&mut app, "edit.copy");

    // Document 2
    run(&mut app, "file.new", json!({"width": 200, "height": 200}));
    let id2 = run(&mut app, "text.create", json!({"x": 20, "y": 40, "text": "Other", "size": 12, "color": "#000000"}))["id"].as_u64().unwrap();
    app.session.select_tool("type", v).unwrap();
    app.session.set_tool_option("textSelection", &json!({"id": id2, "start": 5, "end": 5}));
    crate::panels::character::intercept_text_command(&mut app, "edit.paste");
    let t2 = app.session.doc().unwrap().doc.node(vectorcraft_doc::NodeId(id2)).unwrap();
    let vectorcraft_doc::NodeKind::Text(t2) = &t2.kind else { panic!() };
    assert_eq!(t2.plain_text(), "OtherStyled");
    let pasted_run = &t2.runs[1];
    assert_eq!(pasted_run.style.size, 36.0, "pasted text keeps 36 pt size in document 2");
    assert_eq!(
        pasted_run.style.fill,
        vectorcraft_doc::color::Paint::solid(vectorcraft_doc::color::Color::rgb8(255, 0, 0)),
        "pasted text keeps red fill"
    );
}

#[test]
fn paste_without_formatting_takes_surrounding_style() {
    let _guard = TEST_CLIP_LOCK.lock().unwrap();
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    run(&mut app, "file.new", json!({"width": 200, "height": 200}));
    let id1 = run(&mut app, "text.create", json!({"x": 20, "y": 40, "text": "Big Red", "size": 48, "color": "#ff0000"}))["id"].as_u64().unwrap();
    let v = app.view_info();
    app.session.select_tool("type", v).unwrap();
    app.session.set_tool_option("textSelection", &json!({"id": id1, "start": 0, "end": 7}));
    crate::panels::character::intercept_text_command(&mut app, "edit.copy");

    // Paste without formatting into another document with 10 pt text
    run(&mut app, "file.new", json!({"width": 200, "height": 200}));
    let id2 = run(&mut app, "text.create", json!({"x": 20, "y": 40, "text": "Small: ", "size": 10, "color": "#000000"}))["id"].as_u64().unwrap();
    app.session.select_tool("type", v).unwrap();
    app.session.set_tool_option("textSelection", &json!({"id": id2, "start": 7, "end": 7}));
    crate::panels::character::intercept_text_command(&mut app, "edit.pasteWithoutFormatting");
    let t2 = app.session.doc().unwrap().doc.node(vectorcraft_doc::NodeId(id2)).unwrap();
    let vectorcraft_doc::NodeKind::Text(t2) = &t2.kind else { panic!() };
    assert_eq!(t2.plain_text(), "Small: Big Red");
    assert!(t2.runs.iter().all(|r| r.style.size == 10.0), "all runs take 10 pt size: {:?}", t2.runs);
}

#[test]
fn external_plain_text_matching_an_old_copy_uses_destination_style() {
    let _guard = TEST_CLIP_LOCK.lock().unwrap();
    let (mut app, board) = super::sysclip::app();
    let source = run(&mut app, "text.create", json!({"x": 20, "y": 40, "text": "Hello", "size": 36, "color": "#ff0000"}))["id"].as_u64().unwrap();
    let v = app.view_info();
    app.session.select_tool("type", v).unwrap();
    app.session.set_tool_option("textSelection", &json!({"id": source, "start": 0, "end": 5}));
    crate::panels::character::intercept_text_command(&mut app, "edit.copy");
    crate::background::wait_all(&mut app);

    let own_destination =
        run(&mut app, "text.create", json!({"x": 20, "y": 60, "text": "Own: ", "size": 12, "color": "#000000"}))["id"].as_u64().unwrap();
    app.session.select_tool("type", v).unwrap();
    app.session.set_tool_option("textSelection", &json!({"id": own_destination, "start": 5, "end": 5}));
    crate::panels::character::intercept_text_command(&mut app, "edit.paste");
    let node = app.session.doc().unwrap().doc.node(NodeId(own_destination)).unwrap();
    let vectorcraft_doc::NodeKind::Text(t) = &node.kind else { panic!() };
    assert_eq!(t.plain_text(), "Own: Hello");
    assert_eq!(t.runs.last().unwrap().style.size, 36.0, "our copy keeps its style");

    // Another app later copies the same characters as plain text. Their style is unrelated.
    super::sysclip::copy_elsewhere(&board, vec![(vectorcraft_engine::cmd::clipboard::TEXT, b"Hello".to_vec())]);
    let destination =
        run(&mut app, "text.create", json!({"x": 20, "y": 80, "text": "Before ", "size": 12, "color": "#000000"}))["id"].as_u64().unwrap();
    app.session.select_tool("type", v).unwrap();
    app.session.set_tool_option("textSelection", &json!({"id": destination, "start": 7, "end": 7}));
    super::sysclip::frame(&mut app, vec![egui::Event::Paste("Hello".into())]);
    assert_eq!(app.system_clipboard_text().as_deref(), Some("Hello"));
    app.session.select_tool("selection", v).unwrap();
    app.session.select_tool("type", v).unwrap();
    app.session.set_tool_option("textSelection", &json!({"id": destination, "start": 12, "end": 12}));
    assert!(app.session.tool_wants_text());
    assert!(crate::panels::character::intercept_text_command(&mut app, "edit.paste").is_some());

    let node = app.session.doc().unwrap().doc.node(NodeId(destination)).unwrap();
    let vectorcraft_doc::NodeKind::Text(t) = &node.kind else { panic!() };
    assert_eq!(t.plain_text(), "Before HelloHello");
    assert!(t.runs.iter().all(|run| run.style.size == 12.0 && run.style.fill == Paint::solid(Color::rgb8(0, 0, 0))));
}

#[test]
fn type_paste_keeps_a_copy_the_system_clipboard_refused_until_another_app_copies() {
    let _guard = TEST_CLIP_LOCK.lock().unwrap();
    let (mut app, board) = super::sysclip::app();
    super::sysclip::copy_elsewhere(&board, vec![(vectorcraft_engine::cmd::clipboard::TEXT, b"Old".to_vec())]);
    super::sysclip::lock(&board, true);
    let source = run(&mut app, "text.create", json!({"x": 20, "y": 40, "text": "Hello", "size": 36, "color": "#ff0000"}))["id"].as_u64().unwrap();
    let v = app.view_info();
    app.session.select_tool("type", v).unwrap();
    app.session.set_tool_option("textSelection", &json!({"id": source, "start": 0, "end": 5}));
    crate::panels::character::intercept_text_command(&mut app, "edit.copy");
    crate::background::wait_all(&mut app);
    super::sysclip::lock(&board, false);
    assert_eq!(app.system_clipboard_text().as_deref(), Some("Old"), "the write was refused");

    let destination =
        run(&mut app, "text.create", json!({"x": 20, "y": 60, "text": "Own: ", "size": 12, "color": "#000000"}))["id"].as_u64().unwrap();
    app.session.select_tool("type", v).unwrap();
    app.session.set_tool_option("textSelection", &json!({"id": destination, "start": 5, "end": 5}));
    crate::panels::character::intercept_text_command(&mut app, "edit.paste");
    let node = app.session.doc().unwrap().doc.node(NodeId(destination)).unwrap();
    let vectorcraft_doc::NodeKind::Text(t) = &node.kind else { panic!() };
    assert_eq!(t.plain_text(), "Own: Hello");
    assert_eq!(t.runs.last().unwrap().style.size, 36.0, "our copy keeps its style");

    // A paste event carries the text the clipboard kept.
    app.session.select_tool("selection", v).unwrap();
    app.session.select_tool("type", v).unwrap();
    app.session.set_tool_option("textSelection", &json!({"id": destination, "start": 10, "end": 10}));
    super::sysclip::frame(&mut app, vec![egui::Event::Paste("Old".into())]);
    let node = app.session.doc().unwrap().doc.node(NodeId(destination)).unwrap();
    let vectorcraft_doc::NodeKind::Text(t) = &node.kind else { panic!() };
    assert_eq!(t.plain_text(), "Own: HelloHello");

    // Another app copies, even the same text: Paste takes it, in the destination's style.
    super::sysclip::copy_elsewhere(&board, vec![(vectorcraft_engine::cmd::clipboard::TEXT, b"Old".to_vec())]);
    app.session.select_tool("selection", v).unwrap();
    app.session.select_tool("type", v).unwrap();
    app.session.set_tool_option("textSelection", &json!({"id": destination, "start": 5, "end": 5}));
    crate::panels::character::intercept_text_command(&mut app, "edit.paste");
    let node = app.session.doc().unwrap().doc.node(NodeId(destination)).unwrap();
    let vectorcraft_doc::NodeKind::Text(t) = &node.kind else { panic!() };
    assert_eq!(t.plain_text(), "Own: OldHelloHello");
    let old = t.runs.iter().find(|run| run.text.contains("Old")).unwrap();
    assert_eq!(old.style.size, 12.0, "another app's text takes the destination's style");
}

/// An app whose fake clipboard has no change count and holds `held`; "Hello" (36 pt) copied in the
/// Type tool with `refuse` set up by `before_copy`; and a paste at `at` into "Own: " (12 pt).
fn uncounted_refused_copy(
    held: (&'static str, Vec<u8>),
    before_copy: impl FnOnce(&std::rc::Rc<std::cell::RefCell<super::sysclip::Board>>),
) -> (VectorcraftApp, std::rc::Rc<std::cell::RefCell<super::sysclip::Board>>, impl Fn(&mut VectorcraftApp, usize) -> String) {
    let (mut app, board) = super::sysclip::app();
    super::sysclip::without_change_count(&board);
    super::sysclip::copy_elsewhere(&board, vec![held]);
    before_copy(&board);
    let source = run(&mut app, "text.create", json!({"x": 20, "y": 40, "text": "Hello", "size": 36, "color": "#ff0000"}))["id"].as_u64().unwrap();
    let v = app.view_info();
    app.session.select_tool("type", v).unwrap();
    app.session.set_tool_option("textSelection", &json!({"id": source, "start": 0, "end": 5}));
    crate::panels::character::intercept_text_command(&mut app, "edit.copy");
    crate::background::wait_all(&mut app);

    let destination =
        run(&mut app, "text.create", json!({"x": 20, "y": 60, "text": "Own: ", "size": 12, "color": "#000000"}))["id"].as_u64().unwrap();
    let paste_at = move |app: &mut VectorcraftApp, at: usize| {
        app.session.select_tool("selection", v).unwrap();
        app.session.select_tool("type", v).unwrap();
        app.session.set_tool_option("textSelection", &json!({"id": destination, "start": at, "end": at}));
        crate::panels::character::intercept_text_command(app, "edit.paste");
        let node = app.session.doc().unwrap().doc.node(NodeId(destination)).unwrap();
        let vectorcraft_doc::NodeKind::Text(t) = &node.kind else { panic!() };
        t.plain_text()
    };
    (app, board, paste_at)
}

#[test]
fn type_paste_without_a_change_count_keeps_a_refused_copy_while_the_text_is_unchanged() {
    use vectorcraft_engine::cmd::clipboard::TEXT;
    let _guard = TEST_CLIP_LOCK.lock().unwrap();
    let (mut app, board, paste_at) = uncounted_refused_copy((TEXT, b"Old".to_vec()), |b| super::sysclip::refuse_writes(b, true));
    assert_eq!(paste_at(&mut app, 5), "Own: Hello");
    super::sysclip::copy_elsewhere(&board, vec![(TEXT, b"New".to_vec())]);
    assert_eq!(paste_at(&mut app, 5), "Own: NewHello");
}

#[test]
fn type_paste_without_a_change_count_drops_a_refused_copy_it_cannot_track() {
    use vectorcraft_engine::cmd::clipboard::{PNG, TEXT};
    let _guard = TEST_CLIP_LOCK.lock().unwrap();
    // The clipboard can't be read when the write fails: its text read later may be another app's.
    let (mut app, board, paste_at) = uncounted_refused_copy((TEXT, b"Old".to_vec()), |b| super::sysclip::lock(b, true));
    assert_eq!(paste_at(&mut app, 5), "Own: ");
    super::sysclip::lock(&board, false);
    assert_eq!(paste_at(&mut app, 5), "Own: Old");

    // The clipboard holds a picture, and another app copies a different picture: no text pastes.
    let (mut app, board, paste_at) = uncounted_refused_copy((PNG, super::sysclip::blue_png()), |b| {
        super::sysclip::has_text_only(b);
        super::sysclip::refuse_writes(b, true);
    });
    super::sysclip::copy_elsewhere(&board, vec![(PNG, super::sysclip::blue_png().into_iter().rev().collect())]);
    assert_eq!(paste_at(&mut app, 5), "Own: ");
}

#[test]
fn a_paste_event_waiting_on_a_refused_copy_pastes_that_copy() {
    let _guard = TEST_CLIP_LOCK.lock().unwrap();
    // Another copy's formats hold the worker, so this Copy's formats wait.
    let (mut app, board, hold) = super::sysclip::app_with_busy_worker("");
    super::sysclip::copy_elsewhere(&board, vec![(vectorcraft_engine::cmd::clipboard::TEXT, b"Old".to_vec())]);
    let source = run(&mut app, "text.create", json!({"x": 20, "y": 40, "text": "Hello", "size": 36, "color": "#ff0000"}))["id"].as_u64().unwrap();
    let destination =
        run(&mut app, "text.create", json!({"x": 20, "y": 60, "text": "Own: ", "size": 12, "color": "#000000"}))["id"].as_u64().unwrap();
    let v = app.view_info();
    app.session.select_tool("type", v).unwrap();
    app.session.set_tool_option("textSelection", &json!({"id": source, "start": 0, "end": 5}));
    crate::panels::character::intercept_text_command(&mut app, "edit.copy");
    assert!(app.clipboard_publishing.is_pending());

    // The paste comes while the formats wait, and the write then fails with the clipboard open.
    super::sysclip::lock(&board, true);
    drop(hold);
    app.session.select_tool("selection", v).unwrap();
    app.session.select_tool("type", v).unwrap();
    app.session.set_tool_option("textSelection", &json!({"id": destination, "start": 5, "end": 5}));
    super::sysclip::frame(&mut app, vec![egui::Event::Paste("Old".into())]);
    let node = app.session.doc().unwrap().doc.node(NodeId(destination)).unwrap();
    let vectorcraft_doc::NodeKind::Text(t) = &node.kind else { panic!() };
    assert_eq!(t.plain_text(), "Own: Hello");
}

#[test]
fn a_copy_elsewhere_before_a_refused_write_wins_the_type_paste() {
    let _guard = TEST_CLIP_LOCK.lock().unwrap();
    let (mut app, board, hold) = super::sysclip::app_with_busy_worker("");
    let source = run(&mut app, "text.create", json!({"x": 20, "y": 40, "text": "Hello", "size": 36, "color": "#ff0000"}))["id"].as_u64().unwrap();
    let destination =
        run(&mut app, "text.create", json!({"x": 20, "y": 60, "text": "Own: ", "size": 12, "color": "#000000"}))["id"].as_u64().unwrap();
    let v = app.view_info();
    app.session.select_tool("type", v).unwrap();
    app.session.set_tool_option("textSelection", &json!({"id": source, "start": 0, "end": 5}));
    crate::panels::character::intercept_text_command(&mut app, "edit.copy");
    assert!(app.clipboard_publishing.is_pending());

    // Another app copies while the formats wait, and keeps the clipboard open for the write.
    super::sysclip::copy_elsewhere(&board, vec![(vectorcraft_engine::cmd::clipboard::TEXT, b"New".to_vec())]);
    super::sysclip::lock(&board, true);
    drop(hold);
    crate::background::wait_all(&mut app);
    super::sysclip::lock(&board, false);

    app.session.select_tool("selection", v).unwrap();
    app.session.select_tool("type", v).unwrap();
    app.session.set_tool_option("textSelection", &json!({"id": destination, "start": 5, "end": 5}));
    crate::panels::character::intercept_text_command(&mut app, "edit.paste");
    let node = app.session.doc().unwrap().doc.node(NodeId(destination)).unwrap();
    let vectorcraft_doc::NodeKind::Text(t) = &node.kind else { panic!() };
    assert_eq!(t.plain_text(), "Own: New");
}

#[test]
fn type_paste_keeps_a_copy_whose_write_failed_after_emptying_the_clipboard() {
    let _guard = TEST_CLIP_LOCK.lock().unwrap();
    assert_eq!(paste_after_a_write_that_failed_after_emptying(false), "Own: Hello");
    // The formats other than text were written before the failure.
    assert_eq!(paste_after_a_write_that_failed_after_emptying(true), "Own: Hello");
}

/// Copy "Hello" in the Type tool with a write that empties the clipboard and fails (`keep_others`:
/// after writing the formats other than text), then paste into "Own: ".
fn paste_after_a_write_that_failed_after_emptying(keep_others: bool) -> String {
    let (mut app, board) = super::sysclip::app();
    super::sysclip::copy_elsewhere(&board, vec![(vectorcraft_engine::cmd::clipboard::TEXT, b"Old".to_vec())]);
    super::sysclip::fail_writes_after_emptying(&board, keep_others);
    let source = run(&mut app, "text.create", json!({"x": 20, "y": 40, "text": "Hello", "size": 36, "color": "#ff0000"}))["id"].as_u64().unwrap();
    let destination =
        run(&mut app, "text.create", json!({"x": 20, "y": 60, "text": "Own: ", "size": 12, "color": "#000000"}))["id"].as_u64().unwrap();
    let v = app.view_info();
    app.session.select_tool("type", v).unwrap();
    app.session.set_tool_option("textSelection", &json!({"id": source, "start": 0, "end": 5}));
    crate::panels::character::intercept_text_command(&mut app, "edit.copy");
    crate::background::wait_all(&mut app);

    app.session.select_tool("selection", v).unwrap();
    app.session.select_tool("type", v).unwrap();
    app.session.set_tool_option("textSelection", &json!({"id": destination, "start": 5, "end": 5}));
    crate::panels::character::intercept_text_command(&mut app, "edit.paste");
    let node = app.session.doc().unwrap().doc.node(NodeId(destination)).unwrap();
    let vectorcraft_doc::NodeKind::Text(t) = &node.kind else { panic!() };
    t.plain_text()
}

#[test]
fn a_copy_elsewhere_during_a_refused_write_wins_the_type_paste() {
    let _guard = TEST_CLIP_LOCK.lock().unwrap();
    use vectorcraft_engine::cmd::clipboard::{PNG, TEXT};
    assert_eq!(paste_after_a_copy_during_a_refused_write(TEXT, b"Race".to_vec(), false), "Own: Race");
    // Its other formats may match this copy's; its text still differs.
    assert_eq!(paste_after_a_copy_during_a_refused_write(TEXT, b"Race".to_vec(), true), "Own: Race");
    // A picture has no text to paste into type.
    assert_eq!(paste_after_a_copy_during_a_refused_write(PNG, super::sysclip::blue_png(), false), "Own: ");
}

/// Copy in the Type tool while another app copies `data` of type `mime` (with `our_formats`, also
/// the copy's formats other than text) and the write fails, then paste into "Own: ".
fn paste_after_a_copy_during_a_refused_write(mime: &'static str, data: Vec<u8>, our_formats: bool) -> String {
    let (mut app, board) = super::sysclip::app();
    super::sysclip::copy_elsewhere(&board, vec![(vectorcraft_engine::cmd::clipboard::TEXT, b"Old".to_vec())]);
    super::sysclip::copy_during_next_write(&board, mime, data, our_formats);
    let source = run(&mut app, "text.create", json!({"x": 20, "y": 40, "text": "Hello", "size": 36, "color": "#ff0000"}))["id"].as_u64().unwrap();
    let destination =
        run(&mut app, "text.create", json!({"x": 20, "y": 60, "text": "Own: ", "size": 12, "color": "#000000"}))["id"].as_u64().unwrap();
    let v = app.view_info();
    app.session.select_tool("type", v).unwrap();
    app.session.set_tool_option("textSelection", &json!({"id": source, "start": 0, "end": 5}));
    crate::panels::character::intercept_text_command(&mut app, "edit.copy");
    crate::background::wait_all(&mut app);

    app.session.select_tool("selection", v).unwrap();
    app.session.select_tool("type", v).unwrap();
    app.session.set_tool_option("textSelection", &json!({"id": destination, "start": 5, "end": 5}));
    crate::panels::character::intercept_text_command(&mut app, "edit.paste");
    let node = app.session.doc().unwrap().doc.node(NodeId(destination)).unwrap();
    let vectorcraft_doc::NodeKind::Text(t) = &node.kind else { panic!() };
    t.plain_text()
}

/// https://helpx.adobe.com/illustrator/using/format-text.html
#[test]
fn copying_single_character_carries_formatting_across_documents() {
    let _guard = TEST_CLIP_LOCK.lock().unwrap();
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    run(&mut app, "file.new", json!({"width": 200, "height": 200}));
    let id1 = run(&mut app, "text.create", json!({"x": 20, "y": 40, "text": "X", "size": 36, "color": "#ff0000"}))["id"].as_u64().unwrap();
    let v = app.view_info();
    app.session.select_tool("type", v).unwrap();
    app.session.set_tool_option("textSelection", &json!({"id": id1, "start": 0, "end": 1}));
    crate::panels::character::intercept_text_command(&mut app, "edit.copy");

    // Document 2
    run(&mut app, "file.new", json!({"width": 200, "height": 200}));
    let id2 = run(&mut app, "text.create", json!({"x": 20, "y": 40, "text": "A", "size": 12, "color": "#000000"}))["id"].as_u64().unwrap();
    app.session.select_tool("type", v).unwrap();
    app.session.set_tool_option("textSelection", &json!({"id": id2, "start": 1, "end": 1}));
    crate::panels::character::intercept_text_command(&mut app, "edit.paste");
    let t2 = app.session.doc().unwrap().doc.node(vectorcraft_doc::NodeId(id2)).unwrap();
    let vectorcraft_doc::NodeKind::Text(t2) = &t2.kind else { panic!() };
    assert_eq!(t2.plain_text(), "AX");
    assert_eq!(t2.runs[1].style.size, 36.0, "single character pasted carries styled size");
}

#[test]
fn copying_text_and_pasting_on_canvas_in_another_document() {
    let _guard = TEST_CLIP_LOCK.lock().unwrap();
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    run(&mut app, "file.new", json!({"width": 200, "height": 200}));
    let id1 = run(&mut app, "text.create", json!({"x": 20, "y": 40, "text": "Hello", "size": 28, "color": "#00ff00"}))["id"].as_u64().unwrap();
    let v = app.view_info();
    app.session.select_tool("type", v).unwrap();
    app.session.set_tool_option("textSelection", &json!({"id": id1, "start": 0, "end": 5}));
    crate::panels::character::intercept_text_command(&mut app, "edit.copy");

    // Document 2: paste onto canvas with selection tool
    run(&mut app, "file.new", json!({"width": 200, "height": 200}));
    app.run("edit.paste", json!({})).unwrap();
    let pasted_nodes = objects(&app);
    assert_eq!(pasted_nodes.len(), 1);
    let node = app.session.doc().unwrap().doc.node(pasted_nodes[0]).unwrap();
    let vectorcraft_doc::NodeKind::Text(t) = &node.kind else { panic!("not text") };
    assert_eq!(t.plain_text(), "Hello");
    assert_eq!(t.runs[0].style.size, 28.0);
}

/// Copy of a styled Type range publishes rich formats to the desktop clipboard. After leaving
/// text editing, canvas Paste must use that copy rather than import its plain-text representation.
#[test]
fn type_range_copy_stays_styled_when_pasted_as_art() {
    let _guard = TEST_CLIP_LOCK.lock().unwrap();
    let (mut app, board) = super::sysclip::app();
    super::sysclip::copy_elsewhere(&board, vec![(vectorcraft_engine::cmd::clipboard::TEXT, b"Older plain text".to_vec())]);
    let source =
        run(&mut app, "text.create", json!({"x": 20, "y": 40, "text": "Hello", "font": "Source Serif 4", "size": 28, "color": "#00ff00"}))["id"]
            .as_u64()
            .unwrap();
    run(&mut app, "charStyle.new", json!({"name": "Emphasis", "attrs": {"size": 28.0}}));
    run(&mut app, "charStyle.apply", json!({"name": "Emphasis", "id": source}));
    let view = app.view_info();
    app.session.select_tool("type", view).unwrap();
    app.session.set_tool_option("textSelection", &json!({"id": source, "start": 0, "end": 5}));
    crate::panels::character::intercept_text_command(&mut app, "edit.copy");
    crate::background::wait_all(&mut app);
    assert!(super::sysclip::written(&board).contains(&vectorcraft_engine::cmd::clipboard::SVG), "desktop Copy publishes rich art");
    assert!(app.session.clipboard.char_styles.iter().any(|style| style.name == "Emphasis"), "rich Copy includes the referenced character style");

    app.session.select_tool("selection", view).unwrap();
    run(&mut app, "edit.paste", json!({}));
    let id = app.session.active().unwrap().selection.objects[0];
    let node = app.session.active().unwrap().doc.node(id).unwrap();
    let vectorcraft_doc::NodeKind::Text(t) = &node.kind else { panic!("Paste did not create text art") };
    assert_eq!(t.plain_text(), "Hello");
    assert_eq!(t.runs[0].style.font_family, "Source Serif 4");
    assert_eq!(t.runs[0].style.size, 28.0);
    assert_eq!(t.runs[0].style.fill, Paint::solid(Color::rgb8(0, 255, 0)));
    assert_eq!(t.runs[0].style.style_name.as_deref(), Some("Emphasis"));
}
