//! Headless frames of brush libraries in library panels: clicking a brush adds it to the
//! document's Brushes (and applies it to the selected paths), the Window menu and the Brushes panel open libraries, Save Brushes writes User Defined libraries
//! that reopen, and Other Library… turns down files it can't read.

use egui::{Event, Modifiers, Pos2, Rect, vec2};
use serde_json::json;
use vectorcraft_engine::Session;

use super::brushes::BrushLibraries;
use super::library_panel::tests::{click, confirm_save, context, menu_library_ids};
use super::library_panel::{self, LibraryKind, LibraryPanel, last_open};
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

fn has(app: &VectorcraftApp, name: &str) -> bool {
    vectorcraft_brush::find(&app.session.doc().unwrap().doc, name).is_some()
}

fn count(app: &VectorcraftApp) -> usize {
    vectorcraft_brush::library(&app.session.doc().unwrap().doc).len()
}

#[test]
fn clicking_a_library_brush_adds_it_to_the_documents_brushes_in_one_step() {
    let mut app = app();
    let ctx = context();
    let r = app.run("window.brushLibrary", json!({"library": "Art Ribbons"})).unwrap();
    assert_eq!((r["open"].as_str(), r["count"].as_u64()), (Some("art-ribbons"), Some(5)));
    assert_eq!(last_open(&app.ui).map(|p| (p.kind.as_str(), p.id.as_str())), Some(("brushes", "art-ribbons")));
    let (undo, brushes) = (undo_len(&app), count(&app));
    click(&mut app, &ctx, "Wave", Modifiers::NONE, 0.0);
    assert!(has(&app, "Wave"));
    assert_eq!((count(&app), undo_len(&app)), (brushes + 1, undo + 1), "one brush, one undo step");
    assert_eq!(app.run("brush.list", json!({})).unwrap()["current"], "Wave", "the Paintbrush's brush");
    // Clicking it again adds nothing.
    click(&mut app, &ctx, "Wave", Modifiers::NONE, 2.0);
    assert_eq!((count(&app), undo_len(&app)), (brushes + 1, undo + 1));
    // With a path selected, a click also applies the brush to it, in the same step.
    let path = app.run("shape.line", json!({"x1": 10, "y1": 10, "x2": 150, "y2": 60})).unwrap()["id"].as_u64().unwrap();
    let undo = undo_len(&app);
    click(&mut app, &ctx, "Striped Ribbon", Modifiers::NONE, 4.0);
    assert_eq!(undo_len(&app), undo + 1);
    let st = app.session.doc().unwrap();
    let n = st.doc.node(vectorcraft_doc::NodeId(path)).unwrap();
    assert_eq!(n.appearance.stroke().and_then(|s| s.brush.clone()).as_deref(), Some("Striped Ribbon"));
    assert!(st.doc.pattern("Pinstripes").is_some(), "with the pattern its art is filled with");
    // Modifier clicks only select; Add to Brushes adds the selection.
    app.run("select.none", json!({})).unwrap();
    click(&mut app, &ctx, "Teardrop", Modifiers::COMMAND, 6.0);
    assert!(!has(&app, "Teardrop"));
    BrushLibraries::add(&mut app, "art-ribbons", vec!["Teardrop".into(), "Double Line".into()]);
    assert!(has(&app, "Teardrop") && has(&app, "Double Line"));
    // List rows give the type.
    let lib = BrushLibraries::get(&app, "art-ribbons").unwrap().1;
    let rows = BrushLibraries::rows(&lib, "art-ribbons");
    let teardrop = rows.iter().find(|r| r.name == "Teardrop").and_then(|r| r.item).expect("Teardrop is listed");
    assert_eq!(BrushLibraries::describe(teardrop), "Art");
    assert!(app.run("window.brushLibrary", json!({"library": "nope"})).is_err());
    app.run("window.brushLibrary", json!({"library": null})).unwrap();
    assert!(last_open(&app.ui).is_none());
}

/// A built-in library lists its brushes in the UI language, and Find matches the name shown and
/// the brush's own name. The brush keeps its own name for its tile, a click and the document's
/// Brushes. A library loaded from a document lists its brushes as they are named.
#[test]
fn built_in_library_brushes_are_listed_in_the_ui_language() {
    let dir = std::env::temp_dir().join(format!("vc-brush-names-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut app = app();
    let ctx = context();
    crate::i18n::set_current(crate::i18n::Lang::from_code("fr").expect("fr registered"));
    let list = |app: &mut VectorcraftApp, t: f64| {
        let panel = last_open(&app.ui).map(LibraryPanel::panel).unwrap_or_default();
        app.run("ui.libraryPanel", json!({"panel": panel, "view": "smallList"})).unwrap();
        frame(&ctx, vec![], t, |ui| library_panel::show_window(app, ui.ctx()));
        frame(&ctx, vec![], t + 0.25, |ui| library_panel::show_window(app, ui.ctx()))
    };
    app.run("window.brushLibrary", json!({"library": "ink-and-wash"})).unwrap();
    let texts = list(&mut app, 0.0);
    assert!(texts.iter().any(|t| t == "Encre effilée"), "{texts:?}");
    assert!(!texts.iter().any(|t| t == "Tapered Ink"), "{texts:?}");
    click(&mut app, &ctx, "Encre effilée", Modifiers::NONE, 1.0);
    assert!(has(&app, "Tapered Ink"));
    assert_eq!(app.run("brush.list", json!({})).unwrap()["current"], "Tapered Ink");
    // The document's brushes, opened as a library, keep their names.
    let doc = dir.join("Inks.vectorcraft");
    let b64 = app.run("document.serialize", json!({})).unwrap()["dataBase64"].as_str().unwrap().to_string();
    std::fs::write(&doc, vectorcraft_format::base64_decode(&b64).unwrap()).unwrap();
    app.run("window.brushLibrary.other", json!({"path": doc.to_string_lossy()})).unwrap();
    let texts = list(&mut app, 2.0);
    assert!(texts.iter().any(|t| t == "Tapered Ink"), "{texts:?}");
    assert!(!texts.iter().any(|t| t == "Encre effilée"), "{texts:?}");
    crate::i18n::set_current(crate::i18n::Lang::EN);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_window_menu_and_the_brushes_panel_open_brush_libraries() {
    let mut app = app();
    let entries = crate::menus::menu_entries(&app);
    let under = |e: &crate::menus::MenuEntry| e.path == ["Window", "Brush Libraries"];
    let opened: Vec<_> =
        entries.iter().filter(|e| under(e) && e.command.as_deref() == Some("window.brushLibrary")).map(|e| e.params["library"].clone()).collect();
    assert_eq!(
        opened,
        [
            json!("art-ribbons"),
            json!("ink-and-wash"),
            json!("calligraphic-nibs"),
            json!("pressure-sensitive"),
            json!("bristle-brushes"),
            json!("paint"),
            json!("pattern-borders"),
            json!("decorative"),
            json!("ornamental"),
            json!("scatter-shapes"),
            json!("stipple"),
            json!("game-trails"),
            json!("energy-beams"),
            json!("dry-media"),
            json!("tech-circuit"),
            json!("engraving"),
            json!("arrows"),
            json!("image-brush"),
            json!("artistic"),
            json!("vector-packs")
        ]
    );
    let dry = entries.iter().find(|e| under(e) && e.params["library"] == "dry-media").map(|e| e.label.as_str());
    assert_eq!(dry, Some("Dry Media"));
    let enabled = |cmd: &str| entries.iter().find(|e| under(e) && e.command.as_deref() == Some(cmd)).map(|e| e.enabled);
    assert_eq!(enabled("window.brushLibrary.other"), Some(true));
    assert_eq!(enabled("ui.saveBrushLibrary"), Some(true), "a new document has the default brushes");
    assert!(!entries.iter().any(|e| e.path == ["Window", "Brush Libraries", "User Defined"]), "empty slots are hidden");
    // The Brushes panel's library menus list the same libraries, and each Window row opens its panel.
    let ctx = context();
    let mut menu_ids = menu_library_ids::<BrushLibraries>(&app);
    let mut window_ids: Vec<String> = opened.iter().filter_map(|v| v.as_str().map(str::to_string)).collect();
    menu_ids.sort();
    window_ids.sort();
    assert_eq!(menu_ids, window_ids);
    for id in &window_ids {
        app.run("window.brushLibrary", json!({"library": id})).unwrap();
        assert_eq!(last_open(&app.ui).map(|p| p.id.as_str()), Some(id.as_str()));
    }
    app.run("window.brushLibrary", json!({"library": null})).unwrap();
    // Dry Media opens in a library panel, and a click on Chalk Grain adds it in one step.
    let r = app.run("window.brushLibrary", json!({"library": "Dry Media"})).unwrap();
    assert_eq!((r["open"].as_str(), r["count"].as_u64()), (Some("dry-media"), Some(4)));
    let undo = undo_len(&app);
    click(&mut app, &ctx, "Chalk Grain", Modifiers::NONE, 1.0);
    assert!(has(&app, "Chalk Grain") && !has(&app, "Wax Crayon"));
    assert_eq!(undo_len(&app), undo + 1);
    let r = app.run("window.brushLibrary", json!({"library": "Tech Circuit"})).unwrap();
    assert_eq!((r["open"].as_str(), r["count"].as_u64()), (Some("tech-circuit"), Some(3)));
    let undo = undo_len(&app);
    click(&mut app, &ctx, "Circuit Trace", Modifiers::NONE, 2.0);
    assert!(has(&app, "Circuit Trace") && !has(&app, "Bus Lines"));
    assert_eq!(undo_len(&app), undo + 1);
    let r = app.run("window.brushLibrary", json!({"library": "Artistic Brushes"})).unwrap();
    assert_eq!((r["open"].as_str(), r["count"].as_u64()), (Some("artistic"), Some(4)));
    let previous = crate::i18n::current();
    crate::i18n::set_current(crate::i18n::Lang::from_code("de").unwrap());
    assert_eq!(crate::menus::display_label(&app, "window.brushLibrary", "Artistic Brushes"), "Künstlerische Pinsel");
    crate::i18n::set_current(previous);
    let undo = undo_len(&app);
    click(&mut app, &ctx, "Orbit Lines", Modifiers::NONE, 3.0);
    assert!(has(&app, "Orbit Lines") && !has(&app, "Facet Sweep"));
    assert_eq!(undo_len(&app), undo + 1);
    let r = app.run("window.brushLibrary", json!({"library": "Vector Packs"})).unwrap();
    assert_eq!((r["open"].as_str(), r["count"].as_u64()), (Some("vector-packs"), Some(4)));
    let undo = undo_len(&app);
    click(&mut app, &ctx, "Compass Rose", Modifiers::NONE, 4.0);
    assert!(has(&app, "Compass Rose") && !has(&app, "Facet Diamond"));
    assert_eq!(undo_len(&app), undo + 1);
    let r = app.run("window.brushLibrary", json!({"library": "Ornamental"})).unwrap();
    assert_eq!((r["open"].as_str(), r["count"].as_u64()), (Some("ornamental"), Some(4)));
    let path = app.run("shape.line", json!({"x1": 10, "y1": 100, "x2": 180, "y2": 100})).unwrap()["id"].as_u64().unwrap();
    let undo = undo_len(&app);
    click(&mut app, &ctx, "Scrollwork", Modifiers::NONE, 5.0);
    assert!(has(&app, "Scrollwork") && !has(&app, "Twin Curl"));
    assert_eq!(undo_len(&app), undo + 1);
    assert_eq!(
        app.session.doc().unwrap().doc.node(vectorcraft_doc::NodeId(path)).unwrap().appearance.stroke().unwrap().brush.as_deref(),
        Some("Scrollwork")
    );
    // Without brushes there is nothing to save.
    let all: Vec<String> = vectorcraft_brush::library(&app.session.doc().unwrap().doc).into_iter().map(|b| b.name).collect();
    app.run("brush.delete", json!({ "names": all })).unwrap();
    let entries = crate::menus::menu_entries(&app);
    assert!(entries.iter().any(|e| under(e) && e.command.as_deref() == Some("ui.saveBrushLibrary") && !e.enabled));
}

#[test]
fn save_brushes_writes_a_user_library_that_reopens_and_adds_to_another_document() {
    let dir = std::env::temp_dir().join(format!("vc-brush-libraries-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut app = crate::testing::windowed(app());
    app.session.brush_libraries.set_user_dir(Some(dir.to_string_lossy().to_string()));
    app.run("brush.new", json!({"type": "calligraphic", "name": "Quill", "params": {"angle": 60, "roundness": 30, "size": 5}})).unwrap();
    app.run("ui.saveBrushLibrary", json!({"names": ["Quill"]})).unwrap();
    confirm_save(&mut app, &[("name", json!("Pens")), ("selectedOnly", json!(true))]);
    let ctx = context();
    let user: Vec<(String, String)> = crate::menus::menu_entries(&app)
        .into_iter()
        .filter(|e| e.path == ["Window", "Brush Libraries", "User Defined"])
        .map(|e| (e.label, e.command.unwrap_or_default()))
        .collect();
    assert_eq!(user, [("Pens".to_string(), "window.userBrushLibrary1".to_string())]);
    // Reopened in a library panel, its brush goes into another document.
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    app.run("window.userBrushLibrary1", json!({})).unwrap();
    assert_eq!(last_open(&app.ui).map(|p| p.id.as_str()), Some("user/Pens.vectorcraft"));
    let (name, lib) = BrushLibraries::get(&app, "user/Pens.vectorcraft").unwrap();
    assert_eq!((name.as_str(), lib.len()), ("Pens", 1), "only the selected brush");
    let undo = undo_len(&app);
    click(&mut app, &ctx, "Quill", Modifiers::NONE, 1.0);
    assert!(has(&app, "Quill"));
    assert_eq!(undo_len(&app), undo + 1);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn other_library_opens_a_documents_brushes_and_reports_unreadable_files() {
    let dir = std::env::temp_dir().join(format!("vc-brush-other-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut app = app();
    app.run("brush.new", json!({"type": "bristle", "name": "Mop"})).unwrap();
    let brushes = count(&app);
    let doc = dir.join("Studio.vectorcraft");
    let b64 = app.run("document.serialize", json!({})).unwrap()["dataBase64"].as_str().unwrap().to_string();
    std::fs::write(&doc, vectorcraft_format::base64_decode(&b64).unwrap()).unwrap();
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    let r = app.run("window.brushLibrary.other", json!({"path": doc.to_string_lossy()})).unwrap();
    assert_eq!((r["name"].as_str(), r["count"].as_u64()), (Some("Studio"), Some(brushes as u64)));
    let open = last_open(&app.ui).cloned().unwrap();
    assert_eq!(open.kind, BrushLibraries::KIND);
    let other = BrushLibraries::list(&app).into_iter().find(|l| l.id == open.id).unwrap();
    assert_eq!(other.submenu, Some("Other Libraries"));
    // A file that isn't a document is an error, and opens no panel.
    let bad = dir.join("Broken.vectorcraft");
    std::fs::write(&bad, b"not a document").unwrap();
    app.run("window.brushLibrary", json!({"library": null})).unwrap();
    assert!(app.run("window.brushLibrary.other", json!({"path": bad.to_string_lossy()})).is_err());
    assert!(last_open(&app.ui).is_none());
    let _ = std::fs::remove_dir_all(dir);
}

/// Brushes worked out (serialised and hashed) since the last call.
fn keyed() -> usize {
    super::brushes::counters::KEYED.with(|k| k.replace(0))
}

/// A library panel works out a library's brushes once: frames after the first, and the panel
/// closed and opened again, serialise and hash none, however many art brushes the library has.
/// A library loaded from a document (Other Library…) keeps the brushes it was loaded with when
/// that document is edited. Symbol library panels key their thumbnails once too.
#[test]
fn library_panels_serialise_and_hash_their_brushes_once() {
    const MANY: usize = 300;
    let dir = std::env::temp_dir().join(format!("vc-brush-many-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut app = app();
    let ctx = context();
    // A document of MANY art brushes whose art is filled with a pattern, as a library.
    app.run("brush.addFromLibrary", json!({"library": "art-ribbons", "name": "Striped Ribbon"})).unwrap();
    let striped = vectorcraft_brush::find(&app.session.doc().unwrap().doc, "Striped Ribbon").unwrap();
    app.session
        .edit("Brushes", |d, _| {
            let many: Vec<_> = (0..MANY).map(|i| vectorcraft_brush::Brush { name: format!("Ribbon {i}"), ..striped.clone() }).collect();
            vectorcraft_brush::store(d, &many);
            Ok(())
        })
        .unwrap();
    let path = dir.join("Many.vectorcraft");
    let b64 = app.run("document.serialize", json!({})).unwrap()["dataBase64"].as_str().unwrap().to_string();
    std::fs::write(&path, vectorcraft_format::base64_decode(&b64).unwrap()).unwrap();
    let r = app.run("window.brushLibrary.other", json!({"path": path.to_string_lossy()})).unwrap();
    assert_eq!(r["count"].as_u64(), Some(MANY as u64));
    let id = last_open(&app.ui).unwrap().id.clone();
    let show = |app: &mut VectorcraftApp, t: f64| {
        frame(&ctx, vec![], t, |ui| library_panel::show_window(app, ui.ctx()));
    };
    keyed();
    show(&mut app, 0.0);
    assert_eq!(keyed(), MANY, "the first frame works each brush out");
    for i in 1..4 {
        show(&mut app, i as f64);
    }
    assert_eq!(keyed(), 0, "later frames serialise and hash none");
    // Closed and opened again.
    app.run("window.brushLibrary", json!({"library": null})).unwrap();
    show(&mut app, 5.0);
    app.run("window.brushLibrary", json!({"library": id})).unwrap();
    show(&mut app, 6.0);
    show(&mut app, 7.0);
    assert_eq!(keyed(), 0, "reopened, the library's brushes are still known");
    // The document it was loaded from edited: the library doesn't change.
    app.session
        .edit("Brushes", |d, _| {
            vectorcraft_brush::store(d, &[]);
            Ok(())
        })
        .unwrap();
    show(&mut app, 8.0);
    assert_eq!((keyed(), BrushLibraries::get(&app, &id).unwrap().1.len()), (0, MANY));
    app.run("window.brushLibrary", json!({"library": null})).unwrap();
    // A symbol library.
    use super::symbols::counters::KEYED;
    app.run("window.symbolLibrary", json!({"library": "basic-shapes"})).unwrap();
    KEYED.with(|k| k.set(0));
    show(&mut app, 9.0);
    assert!(KEYED.with(|k| k.replace(0)) > 0);
    show(&mut app, 10.0);
    show(&mut app, 11.0);
    assert_eq!(KEYED.with(|k| k.get()), 0, "symbol thumbnails are keyed once");
    let _ = std::fs::remove_dir_all(dir);
}
