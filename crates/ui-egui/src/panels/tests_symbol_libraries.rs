//! Headless frames of symbol libraries in library panels: clicking a symbol adds it to the
//! document's Symbols, dragging one carries it to the canvas, the Window menu and the Symbols panel
//! open libraries, Save Symbols writes User Defined libraries that reopen, and Other Library… turns
//! down files it can't read.

use egui::{Event, Modifiers, PointerButton, Pos2, Rect, vec2};
use serde_json::json;
use vectorcraft_engine::Session;

use super::library_panel::{self, LibraryKind, LibraryPanel, last_open};
use super::symbols::SymbolLibraries;
use crate::VectorcraftApp;
use crate::widgets::PanelDrag;

fn app() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    app
}

fn context() -> egui::Context {
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    ctx
}

/// One frame of `draw` with `events`; the texts it painted.
fn frame(ctx: &egui::Context, events: Vec<Event>, time: f64, mut draw: impl FnMut(&mut egui::Ui)) -> Vec<String> {
    fn texts(s: &egui::Shape, out: &mut Vec<String>) {
        match s {
            egui::Shape::Text(t) => out.push(t.galley.text().to_string()),
            egui::Shape::Vec(v) => v.iter().for_each(|s| texts(s, out)),
            _ => {}
        }
    }
    let screen_rect = Some(Rect::from_min_size(Pos2::ZERO, vec2(1200.0, 800.0)));
    let input = egui::RawInput { events, time: Some(time), screen_rect, ..Default::default() };
    let mut out = ctx.run_ui(input, &mut draw);
    out.textures_delta.clear();
    let mut v = vec![];
    out.shapes.iter().for_each(|c| texts(&c.shape, &mut v));
    v
}

/// The centre of the open library panel's tile of symbol `name` (a new window takes a sizing
/// frame first).
fn tile(app: &mut VectorcraftApp, ctx: &egui::Context, name: &str, time: f64) -> Pos2 {
    for t in [time, time + 0.25] {
        frame(ctx, vec![], t, |ui| library_panel::show_window(app, ui.ctx()));
    }
    let id = library_panel::tile_id(&last_open(&app.ui).map(LibraryPanel::panel).unwrap_or_default(), name);
    ctx.read_response(id).unwrap_or_else(|| panic!("no tile for {name}")).rect.center()
}

fn click(app: &mut VectorcraftApp, ctx: &egui::Context, name: &str, modifiers: Modifiers, time: f64) {
    let at = tile(app, ctx, name, time);
    let button = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers };
    let events = vec![Event::ModifiersChanged(modifiers), Event::PointerMoved(at), button(true), button(false)];
    frame(ctx, events, time + 0.5, |ui| library_panel::show_window(app, ui.ctx()));
}

fn undo_len(app: &VectorcraftApp) -> usize {
    app.session.doc().unwrap().history.undo.len()
}

fn symbols(app: &VectorcraftApp) -> Vec<String> {
    app.session.doc().unwrap().doc.symbols.iter().map(|s| s.name.clone()).collect()
}

#[test]
fn clicking_a_library_symbol_adds_it_to_the_documents_symbols_in_one_step() {
    let mut app = app();
    let ctx = context();
    let r = app.run("window.symbolLibrary", json!({"library": "Basic Shapes"})).unwrap();
    assert_eq!((r["open"].as_str(), r["count"].as_u64()), (Some("basic-shapes"), Some(9)));
    assert_eq!(last_open(&app.ui).map(|p| (p.kind.as_str(), p.id.as_str())), Some(("symbols", "basic-shapes")));
    let undo = undo_len(&app);
    click(&mut app, &ctx, "Star", Modifiers::NONE, 0.0);
    assert_eq!(symbols(&app), ["Star"]);
    assert_eq!(undo_len(&app), undo + 1, "one undo step");
    assert_eq!(app.run("symbol.list", json!({})).unwrap()["current"], "Star", "the Symbol Sprayer's symbol");
    // Clicking it again adds nothing.
    click(&mut app, &ctx, "Star", Modifiers::NONE, 2.0);
    assert_eq!((symbols(&app).len(), undo_len(&app)), (1, undo + 1));
    // Modifier clicks only select; Add to Symbols adds the selection.
    click(&mut app, &ctx, "Hexagon", Modifiers::COMMAND, 4.0);
    assert_eq!(symbols(&app), ["Star"]);
    SymbolLibraries::add(&mut app, "basic-shapes", vec!["Hexagon".into(), "Arrow".into()]);
    assert_eq!(symbols(&app), ["Star", "Hexagon", "Arrow"]);
    // Find filters by name; list rows give the natural size.
    let lib = SymbolLibraries::get(&app, "basic-shapes").unwrap().1;
    let rows = SymbolLibraries::rows(&lib, "arr");
    assert_eq!(rows.iter().map(|r| r.name).collect::<Vec<_>>(), ["Arrow"]);
    assert_eq!(SymbolLibraries::describe(rows[0].item.unwrap()), "48 × 32 pt");
    // Dragging a symbol out of the panel carries it to the canvas.
    let at = tile(&mut app, &ctx, "Burst", 6.0);
    let press = Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE };
    frame(&ctx, vec![Event::PointerMoved(at), press], 6.5, |ui| library_panel::show_window(&mut app, ui.ctx()));
    for (i, d) in [20.0, 60.0].into_iter().enumerate() {
        frame(&ctx, vec![Event::PointerMoved(at + vec2(d, d))], 6.6 + 0.1 * i as f64, |ui| library_panel::show_window(&mut app, ui.ctx()));
    }
    let carried = egui::DragAndDrop::payload::<PanelDrag>(&ctx);
    assert_eq!(carried.as_deref(), Some(&PanelDrag::LibrarySymbol { library: "basic-shapes".into(), name: "Burst".into() }));
    assert!(app.run("window.symbolLibrary", json!({"library": "nope"})).is_err());
    app.run("window.symbolLibrary", json!({"library": null})).unwrap();
    assert!(last_open(&app.ui).is_none());
}

#[test]
fn the_window_menu_and_the_symbols_panel_open_symbol_libraries() {
    let mut app = app();
    let entries = crate::menus::menu_entries(&app);
    let under = |e: &crate::menus::MenuEntry| e.path == ["Window", "Symbol Libraries"];
    let opened: Vec<_> =
        entries.iter().filter(|e| under(e) && e.command.as_deref() == Some("window.symbolLibrary")).map(|e| e.params["library"].clone()).collect();
    assert_eq!(opened, [json!("basic-shapes")]);
    let has = |cmd: &str| entries.iter().find(|e| under(e) && e.command.as_deref() == Some(cmd)).map(|e| e.enabled);
    assert_eq!(has("window.symbolLibrary.other"), Some(true));
    assert_eq!(has("ui.saveSymbolLibrary"), Some(false), "no symbols to save yet");
    assert!(!entries.iter().any(|e| e.path == ["Window", "Symbol Libraries", "User Defined"]), "empty slots are hidden");
    // The Symbols panel's library button and menu.
    let ctx = context();
    let texts = frame(&ctx, vec![], 0.0, |ui| {
        super::symbols::show(&mut app, ui);
        super::symbols::menu(&mut app, ui);
    });
    for t in ["Open Symbol Library", "Save Symbol Library…"] {
        assert!(texts.iter().any(|x| x.ends_with(t)), "{t}: {texts:?}");
    }
}

#[test]
fn save_symbols_writes_a_user_library_that_reopens_and_adds_to_another_document() {
    let dir = std::env::temp_dir().join(format!("vc-symbol-libraries-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut app = app();
    app.session.symbol_libraries.set_user_dir(Some(dir.to_string_lossy().to_string()));
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 30, "height": 20})).unwrap();
    app.run("symbol.new", json!({"name": "Brick"})).unwrap();
    app.run("shape.ellipse", json!({"x": 60, "y": 60, "width": 20, "height": 20})).unwrap();
    app.run("symbol.new", json!({"name": "Pebble"})).unwrap();
    app.run("ui.saveSymbolLibrary", json!({"names": ["Brick"]})).unwrap();
    let d = app.ui.dialog.as_mut().expect("Save Symbol Library opens");
    assert_eq!((d.kind.as_str(), d.bool("user")), (crate::dialogs::save_symbol_library::KIND, true));
    d.fields.insert("name".into(), json!("Masonry"));
    d.fields.insert("selectedOnly".into(), json!(true));
    let ctx = context();
    frame(&ctx, vec![], 0.0, |ui| crate::dialogs::show(&mut app, ui.ctx()));
    let texts = frame(&ctx, vec![], 0.5, |ui| crate::dialogs::show(&mut app, ui.ctx()));
    assert!(texts.iter().any(|t| t == "Selected Symbols Only (1)"), "{texts:?}");
    crate::dialogs::confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none());
    let user: Vec<(String, String)> = crate::menus::menu_entries(&app)
        .into_iter()
        .filter(|e| e.path == ["Window", "Symbol Libraries", "User Defined"])
        .map(|e| (e.label, e.command.unwrap_or_default()))
        .collect();
    assert_eq!(user, [("Masonry".to_string(), "window.userSymbolLibrary1".to_string())]);
    // Reopened in a library panel, its symbol goes into another document.
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    app.run("window.userSymbolLibrary1", json!({})).unwrap();
    assert_eq!(last_open(&app.ui).map(|p| p.id.as_str()), Some("user/Masonry.vectorcraft"));
    let (name, lib) = SymbolLibraries::get(&app, "user/Masonry.vectorcraft").unwrap();
    assert_eq!((name.as_str(), lib.len()), ("Masonry", 1), "only the selected symbol");
    let undo = undo_len(&app);
    click(&mut app, &ctx, "Brick", Modifiers::NONE, 1.0);
    assert_eq!((symbols(&app), undo_len(&app)), (vec!["Brick".to_string()], undo + 1));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn other_library_opens_a_documents_symbols_and_reports_unreadable_files() {
    let dir = std::env::temp_dir().join(format!("vc-symbol-other-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut app = app();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 30, "height": 20})).unwrap();
    app.run("symbol.new", json!({"name": "Tile"})).unwrap();
    let doc = dir.join("Floor.vectorcraft");
    let b64 = app.run("document.serialize", json!({})).unwrap()["dataBase64"].as_str().unwrap().to_string();
    std::fs::write(&doc, vectorcraft_format::base64_decode(&b64).unwrap()).unwrap();
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    let r = app.run("window.symbolLibrary.other", json!({"path": doc.to_string_lossy()})).unwrap();
    assert_eq!((r["name"].as_str(), r["count"].as_u64()), (Some("Floor"), Some(1)));
    let open = last_open(&app.ui).cloned().unwrap();
    assert_eq!(open.kind, SymbolLibraries::KIND);
    let other = SymbolLibraries::list(&app).into_iter().find(|l| l.id == open.id).unwrap();
    assert_eq!(other.submenu, Some("Other Libraries"));
    // A file that isn't a document is an error, and opens no panel.
    let bad = dir.join("Broken.vectorcraft");
    std::fs::write(&bad, b"not a document").unwrap();
    app.run("window.symbolLibrary", json!({"library": null})).unwrap();
    assert!(app.run("window.symbolLibrary.other", json!({"path": bad.to_string_lossy()})).is_err());
    assert!(last_open(&app.ui).is_none());
    let _ = std::fs::remove_dir_all(dir);
}
