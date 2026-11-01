//! Headless frames of graphic style libraries in the library panel: clicking a style adds and
//! applies it, the Window menu and the Graphic Styles panel open libraries, and Save Graphic Style
//! Library writes `.vcstyles` files that open as libraries.

use egui::{Event, Modifiers, PointerButton, Pos2, Rect, vec2};
use serde_json::{Value, json};
use vectorcraft_doc::style_libs::STYLE_LIBRARIES;
use vectorcraft_engine::Session;

use super::graphic_styles::GraphicStyleLibraries;
use super::library_panel::{self, LibraryKind, LibraryPanel, last_open};
use crate::VectorcraftApp;

fn app_with(services: crate::Services) -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), services);
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    app
}

fn app() -> VectorcraftApp {
    app_with(Default::default())
}

fn context() -> egui::Context {
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    ctx
}

/// One frame of `draw` with `events` and `modifiers` held; the texts it painted.
fn frame(ctx: &egui::Context, events: Vec<Event>, modifiers: Modifiers, time: f64, mut draw: impl FnMut(&mut egui::Ui)) -> Vec<String> {
    fn texts(s: &egui::Shape, out: &mut Vec<String>) {
        match s {
            egui::Shape::Text(t) => out.push(t.galley.text().to_string()),
            egui::Shape::Vec(v) => v.iter().for_each(|s| texts(s, out)),
            _ => {}
        }
    }
    let screen_rect = Some(Rect::from_min_size(Pos2::ZERO, vec2(1200.0, 800.0)));
    let events = std::iter::once(Event::ModifiersChanged(modifiers)).chain(events).collect();
    let input = egui::RawInput { events, time: Some(time), screen_rect, ..Default::default() };
    let mut out = ctx.run_ui(input, &mut draw);
    out.textures_delta.clear();
    let mut v = vec![];
    out.shapes.iter().for_each(|c| texts(&c.shape, &mut v));
    v
}

/// Click the library panel's tile of style `name` with `modifiers` (a new window takes a sizing
/// frame first).
fn click(app: &mut VectorcraftApp, ctx: &egui::Context, name: &str, modifiers: Modifiers, time: f64) {
    for t in [time, time + 0.25] {
        frame(ctx, vec![], Modifiers::NONE, t, |ui| library_panel::show_window(app, ui.ctx()));
    }
    let id = library_panel::tile_id(&last_open(&app.ui).map(LibraryPanel::panel).unwrap_or_default(), name);
    let at = ctx.read_response(id).unwrap_or_else(|| panic!("no tile for {name}")).rect.center();
    let button = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers };
    frame(ctx, vec![Event::PointerMoved(at), button(true), button(false)], modifiers, time + 0.5, |ui| library_panel::show_window(app, ui.ctx()));
}

fn undo_len(app: &VectorcraftApp) -> usize {
    app.session.doc().unwrap().history.undo.len()
}

#[test]
fn clicking_a_library_style_adds_and_applies_it_in_one_step() {
    let mut app = app();
    let ctx = context();
    let rect = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 80, "height": 40})).unwrap()["id"].as_u64().unwrap();
    let r = app.run("window.graphicStyleLibrary", json!({"library": "Shadows and Glows"})).unwrap();
    assert_eq!((r["open"].as_str(), r["name"].as_str()), (Some("shadows-glows"), Some("Shadows and Glows")));
    assert_eq!(last_open(&app.ui).map(|p| (p.kind.as_str(), p.id.as_str())), Some(("graphicStyles", "shadows-glows")));
    let undo = undo_len(&app);
    click(&mut app, &ctx, "Halo", Modifiers::NONE, 0.0);
    let (g, linked) = app.session.selection_graphic_style().expect("applied to the selection");
    assert_eq!((g.name.as_str(), linked), ("Halo", true));
    assert_eq!(undo_len(&app), undo + 1, "added and applied as one undo step");
    let tile = ctx.read_response(library_panel::tile_id("library:graphicStyles:shadows-glows", "Halo")).unwrap().rect;
    assert_eq!(tile.width(), 30.0, "styles open in large thumbnails");
    // Alt-click adds the style's appearance on top, unlinked.
    let fills = |app: &VectorcraftApp| app.session.doc().unwrap().doc.node(vectorcraft_doc::NodeId(rect)).unwrap().appearance.items.len();
    let before = fills(&app);
    click(&mut app, &ctx, "Ember", Modifiers::ALT, 2.0);
    assert!(app.session.doc().unwrap().doc.graphic_style("Ember").is_some());
    assert_eq!(fills(&app), before + 1);
    assert!(app.session.selection_graphic_style().is_none_or(|(_, linked)| !linked));
    // Modifier clicks only select; Add to Graphic Styles adds the selection.
    click(&mut app, &ctx, "Feathered", Modifiers::COMMAND, 4.0);
    assert!(app.session.doc().unwrap().doc.graphic_style("Feathered").is_none());
    GraphicStyleLibraries::add(&mut app, "shadows-glows", vec!["Feathered".into(), "Haze".into()]);
    let d = &app.session.doc().unwrap().doc;
    assert!(d.graphic_style("Feathered").is_some() && d.graphic_style("Haze").is_some());
    // Find filters by name; list rows describe the styles; looks are computed once per library.
    let lib = GraphicStyleLibraries::get(&app, "shadows-glows").unwrap().1;
    let rows = GraphicStyleLibraries::rows(&lib, "ember");
    assert_eq!(rows.iter().map(|r| r.name).collect::<Vec<_>>(), ["Ember"]);
    assert_eq!(GraphicStyleLibraries::describe(rows[0].item.unwrap()), "1 fill, Inner Glow, Outer Glow");
    assert!(std::sync::Arc::ptr_eq(&lib, &GraphicStyleLibraries::get(&app, "shadows-glows").unwrap().1));
    assert!(app.run("window.graphicStyleLibrary", json!({"library": "nope"})).is_err());
    app.run("window.graphicStyleLibrary", json!({"library": null})).unwrap();
    assert!(last_open(&app.ui).is_none());
}

#[test]
fn the_window_menu_and_the_panel_open_every_library() {
    let mut app = app();
    let entries = crate::menus::menu_entries(&app);
    let opened: Vec<Value> = entries
        .iter()
        .filter(|e| e.path == ["Window", "Graphic Style Libraries"] && e.command.as_deref() == Some("window.graphicStyleLibrary"))
        .map(|e| e.params["library"].clone())
        .collect();
    assert_eq!(opened, STYLE_LIBRARIES.iter().map(|b| json!(b.id)).collect::<Vec<_>>());
    let has = |cmd: &str| entries.iter().any(|e| e.path == ["Window", "Graphic Style Libraries"] && e.command.as_deref() == Some(cmd) && e.enabled);
    assert!(has("window.graphicStyleLibrary.other") && has("ui.saveGraphicStyleLibrary"));
    assert!(!entries.iter().any(|e| e.path == ["Window", "Graphic Style Libraries", "User Defined"]), "empty slots are hidden");
    // The Graphic Styles panel's library button and menu.
    let ctx = context();
    let texts = frame(&ctx, vec![], Modifiers::NONE, 0.0, |ui| {
        super::graphic_styles::show(&mut app, ui);
        super::graphic_styles::menu(&mut app, ui);
    });
    for t in ["Open Graphic Style Library", "Save Graphic Style Library…"] {
        assert!(texts.iter().any(|x| x.ends_with(t)), "{t}: {texts:?}");
    }
    let ids: Vec<String> = GraphicStyleLibraries::list(&app).into_iter().map(|l| l.id).collect();
    assert_eq!(ids, STYLE_LIBRARIES.iter().map(|b| b.id).collect::<Vec<_>>());
}

#[test]
fn save_dialog_writes_user_libraries_that_the_window_menu_lists() {
    let dir = std::env::temp_dir().join(format!("vc-style-libraries-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut app = app();
    app.session.style_libraries.set_user_dir(Some(dir.to_string_lossy().to_string()));
    let user_items = |app: &VectorcraftApp| -> Vec<(String, String)> {
        crate::menus::menu_entries(app)
            .into_iter()
            .filter(|e| e.path == ["Window", "Graphic Style Libraries", "User Defined"])
            .map(|e| (e.label, e.command.unwrap_or_default()))
            .collect()
    };
    app.run("ui.saveGraphicStyleLibrary", json!({"names": ["Sunshine"]})).unwrap();
    let d = app.ui.dialog.as_mut().expect("Save Graphic Style Library opens");
    assert_eq!((d.kind.as_str(), d.bool("user")), (crate::dialogs::save_style_library::KIND, true));
    d.fields.insert("name".into(), json!("Warm"));
    d.fields.insert("selectedOnly".into(), json!(true));
    let ctx = context();
    frame(&ctx, vec![], Modifiers::NONE, 0.0, |ui| crate::dialogs::show(&mut app, ui.ctx()));
    let texts = frame(&ctx, vec![], Modifiers::NONE, 0.5, |ui| crate::dialogs::show(&mut app, ui.ctx()));
    assert!(texts.iter().any(|t| t == "Selected Styles Only (1)"), "{texts:?}");
    crate::dialogs::confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none());
    assert_eq!(user_items(&app), [("Warm".to_string(), "window.userGraphicStyleLibrary1".to_string())]);
    app.run("window.userGraphicStyleLibrary1", json!({})).unwrap();
    assert_eq!(last_open(&app.ui).map(|p| p.id.as_str()), Some("user/Warm.vcstyles"));
    let (name, lib) = GraphicStyleLibraries::get(&app, "user/Warm.vcstyles").unwrap();
    assert_eq!((name.as_str(), lib.len()), ("Warm", 1), "only the selected style");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn saving_to_a_file_and_opening_a_vcstyles_file() {
    let written = std::rc::Rc::new(std::cell::RefCell::new(vec![]));
    let w = written.clone();
    let services = crate::Services {
        pick_save: Some(Box::new(|p: &crate::FilePick| Some(format!("/tmp/{}", p.name)))),
        write: Some(Box::new(move |p: &str, b: &[u8]| {
            w.borrow_mut().push((p.to_string(), b.to_vec()));
            Ok(())
        })),
        ..Default::default()
    };
    let mut app = app_with(services);
    app.run("ui.saveGraphicStyleLibrary", json!({})).unwrap();
    assert!(!app.ui.dialog.as_ref().unwrap().bool("__user"), "no user folder: a file");
    crate::dialogs::confirm(&mut app).unwrap();
    let (path, bytes) = written.borrow()[0].clone();
    assert!(path.ends_with(".vcstyles"), "{path}");
    // Opening it opens it in the library panel, not as a document.
    crate::io::open_bytes(&mut app, "Saved.vcstyles", &bytes, None).unwrap();
    let open = last_open(&app.ui).cloned().unwrap();
    assert_eq!(open.kind, GraphicStyleLibraries::KIND);
    assert_eq!(app.session.documents().len(), 1);
    let other = GraphicStyleLibraries::list(&app).into_iter().find(|l| l.id == open.id).unwrap();
    assert_eq!(other.submenu, Some("Other Libraries"));
    assert_eq!(GraphicStyleLibraries::get(&app, &open.id).unwrap().1.len(), app.session.doc().unwrap().doc.graphic_styles.len());
}
