//! Headless frames of symbol libraries in library panels: clicking a symbol adds it to the
//! document's Symbols, dragging one carries it to the canvas, the Window menu and the Symbols panel
//! open libraries, Save Symbols writes User Defined libraries that reopen, and Other Library… turns
//! down files it can't read.

use egui::{Event, Modifiers, Pos2, Rect, vec2};
use serde_json::json;
use vectorcraft_engine::Session;

use super::library_panel::tests::{click, confirm_save, context, menu_library_ids};
use super::library_panel::{self, LibraryKind, LibraryPanel, last_open};
use super::symbols::SymbolLibraries;
use crate::VectorcraftApp;
use crate::i18n::LangExt as _;

fn app() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    app
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
    // List rows give the natural size.
    let lib = SymbolLibraries::get(&app, "basic-shapes").unwrap().1;
    let rows = SymbolLibraries::rows(&lib, "basic-shapes");
    let arrow = rows.iter().find(|r| r.name == "Arrow").and_then(|r| r.item).unwrap();
    assert_eq!(SymbolLibraries::describe(arrow), "48 × 32 pt");
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
    assert_eq!(
        opened,
        [
            json!("basic-shapes"),
            json!("ui-frames"),
            json!("map-markers"),
            json!("inventory-icons"),
            json!("skill-icons"),
            json!("weather-icons"),
            json!("nature-icons"),
            json!("media-controls"),
        ]
    );
    let has = |cmd: &str| entries.iter().find(|e| under(e) && e.command.as_deref() == Some(cmd)).map(|e| e.enabled);
    assert_eq!(has("window.symbolLibrary.other"), Some(true));
    assert_eq!(has("ui.saveSymbolLibrary"), Some(false), "no symbols to save yet");
    assert!(!entries.iter().any(|e| e.path == ["Window", "Symbol Libraries", "User Defined"]), "empty slots are hidden");
    // The Symbols panel's library menus list the same libraries, and each Window row opens its panel.
    let ids: Vec<String> = opened.iter().filter_map(|v| v.as_str().map(str::to_string)).collect();
    assert_eq!(menu_library_ids::<SymbolLibraries>(&app), ids);
    for id in &ids {
        app.run("window.symbolLibrary", json!({"library": id})).unwrap();
        assert_eq!(last_open(&app.ui).map(|p| p.id.as_str()), Some(id.as_str()));
    }
}

#[test]
fn save_symbols_writes_a_user_library_that_reopens_and_adds_to_another_document() {
    let dir = std::env::temp_dir().join(format!("vc-symbol-libraries-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut app = crate::testing::windowed(app());
    app.session.symbol_libraries.set_user_dir(Some(dir.to_string_lossy().to_string()));
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 30, "height": 20})).unwrap();
    app.run("symbol.new", json!({"name": "Brick"})).unwrap();
    app.run("shape.ellipse", json!({"x": 60, "y": 60, "width": 20, "height": 20})).unwrap();
    app.run("symbol.new", json!({"name": "Pebble"})).unwrap();
    app.run("ui.saveSymbolLibrary", json!({"names": ["Brick"]})).unwrap();
    confirm_save(&mut app, &[("name", json!("Masonry")), ("selectedOnly", json!(true))]);
    let ctx = context();
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

/// A built-in library lists its symbols in the UI language, and Find matches the name shown and the
/// symbol's own name. A click adds the symbol under its own name. Another library's symbols are
/// matched by their own names only.
#[test]
fn built_in_library_symbols_are_listed_in_the_ui_language() {
    let mut app = app();
    let ctx = context();
    crate::i18n::set_current(crate::i18n::Lang::from_code("fr").expect("fr registered"));
    app.run("window.symbolLibrary", json!({"library": "basic-shapes"})).unwrap();
    let panel = last_open(&app.ui).map(LibraryPanel::panel).unwrap_or_default();
    app.run("ui.libraryPanel", json!({"panel": panel, "view": "smallList"})).unwrap();
    frame(&ctx, vec![], 0.0, |ui| library_panel::show_window(&mut app, ui.ctx()));
    let texts = frame(&ctx, vec![], 0.25, |ui| library_panel::show_window(&mut app, ui.ctx()));
    assert!(texts.iter().any(|t| t == "Éclat") && texts.iter().any(|t| t == "Étoile"), "{texts:?}");
    assert!(!texts.iter().any(|t| t == "Burst"), "{texts:?}");
    click(&mut app, &ctx, "Étoile", Modifiers::NONE, 1.0);
    assert_eq!(symbols(&app), ["Star"]);
    crate::i18n::set_current(crate::i18n::Lang::EN);
}
