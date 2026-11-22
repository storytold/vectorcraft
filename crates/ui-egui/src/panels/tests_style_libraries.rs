//! Headless frames of graphic style libraries in the library panel: clicking a style adds and
//! applies it, the Window menu and the Graphic Styles panel open libraries, and Save Graphic Style
//! Library writes `.vcstyles` files that open as libraries.

use egui::{Event, Modifiers, Pos2, Rect, vec2};
use serde_json::{Value, json};
use vectorcraft_doc::style_libs::STYLE_LIBRARIES;
use vectorcraft_engine::Session;

use super::graphic_styles::GraphicStyleLibraries;
use super::library_panel::tests::{click, confirm_save, context, menu_library_ids, save_to_tmp};
use super::library_panel::{self, LibraryKind, last_open};
use crate::VectorcraftApp;
use crate::i18n::LangExt as _;

fn app_with(services: crate::Services) -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), services);
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    app
}

fn app() -> VectorcraftApp {
    app_with(Default::default())
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
    let rows = GraphicStyleLibraries::rows(&lib, "shadows-glows");
    let ember = rows.iter().find(|r| r.name == "Ember").and_then(|r| r.item).unwrap();
    assert_eq!(GraphicStyleLibraries::describe(ember), "1 fill, Inner Glow, Outer Glow");
    assert!(std::sync::Arc::ptr_eq(&lib, &GraphicStyleLibraries::get(&app, "shadows-glows").unwrap().1));
    assert!(app.run("window.graphicStyleLibrary", json!({"library": "nope"})).is_err());
    app.run("window.graphicStyleLibrary", json!({"library": null})).unwrap();
    assert!(last_open(&app.ui).is_none());
}

#[test]
fn paper_cutouts_opens_from_the_window_menu_and_applies_a_layered_style() {
    let mut app = app();
    let entry = crate::menus::menu_entries(&app)
        .into_iter()
        .find(|e| e.path == ["Window", "Graphic Style Libraries"] && e.label == "Paper Cutouts")
        .expect("a Window menu item");
    let rect = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 80, "height": 40})).unwrap()["id"].as_u64().unwrap();
    app.run(entry.command.as_deref().unwrap(), entry.params.clone()).unwrap();
    assert_eq!(last_open(&app.ui).map(|p| p.id.as_str()), Some("paper-cutouts"));
    let undo = undo_len(&app);
    click(&mut app, &context(), "Coral Layers", Modifiers::NONE, 0.0);
    assert_eq!(undo_len(&app), undo + 1);
    let appearance = &app.session.doc().unwrap().doc.node(vectorcraft_doc::NodeId(rect)).unwrap().appearance;
    assert_eq!(appearance.items.len(), 3);
    assert_eq!(appearance.effects[0].id, "stylize.dropShadow");
}

/// A built-in library's list rows and Find field use the UI language; a user library's styles keep
/// their names, and clicks still name the stored style.
#[test]
fn built_in_style_names_show_in_the_ui_language() {
    let mut app = app();
    let ctx = context();
    app.run("window.graphicStyleLibrary", json!({"library": "paper-cutouts"})).unwrap();
    app.run("ui.libraryPanel", json!({"panel": "library:graphicStyles:paper-cutouts", "view": "largeList"})).unwrap();
    crate::i18n::set_current(crate::i18n::Lang::from_code("fr").unwrap());
    let mut texts = vec![];
    for t in [0.0, 0.25] {
        texts = frame(&ctx, vec![], Modifiers::NONE, t, |ui| library_panel::show_window(&mut app, ui.ctx()));
    }
    let user = GraphicStyleLibraries::label("user/Mine", "Coral Layers").to_string();
    crate::i18n::set_current(crate::i18n::Lang::EN);
    assert!(texts.iter().any(|t| t == "Couches corail"), "{texts:?}");
    assert!(!texts.iter().any(|t| t == "Coral Layers"), "{texts:?}");

    assert_eq!(user, "Coral Layers");
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
    // The Graphic Styles panel's library menus list the same libraries, and each Window row opens its panel.
    let ids: Vec<String> = GraphicStyleLibraries::list(&app).into_iter().map(|l| l.id).collect();
    assert_eq!(ids, STYLE_LIBRARIES.iter().map(|b| b.id).collect::<Vec<_>>());
    assert_eq!(menu_library_ids::<GraphicStyleLibraries>(&app), ids);
    for id in &ids {
        app.run("window.graphicStyleLibrary", json!({"library": id})).unwrap();
        assert_eq!(last_open(&app.ui).map(|p| p.id.as_str()), Some(id.as_str()));
    }
}

#[test]
fn save_dialog_writes_user_libraries_that_the_window_menu_lists() {
    let dir = std::env::temp_dir().join(format!("vc-style-libraries-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut app = crate::testing::windowed(app());
    app.session.style_libraries.set_user_dir(Some(dir.to_string_lossy().to_string()));
    let user_items = |app: &VectorcraftApp| -> Vec<(String, String)> {
        crate::menus::menu_entries(app)
            .into_iter()
            .filter(|e| e.path == ["Window", "Graphic Style Libraries", "User Defined"])
            .map(|e| (e.label, e.command.unwrap_or_default()))
            .collect()
    };
    app.run("ui.saveGraphicStyleLibrary", json!({"names": ["Sunshine"]})).unwrap();
    confirm_save(&mut app, &[("name", json!("Warm")), ("selectedOnly", json!(true))]);
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
        write: Some(Box::new(move |p: &str, b: &[u8]| {
            w.borrow_mut().push((p.to_string(), b.to_vec()));
            Ok(())
        })),
        ..Default::default()
    };
    let mut app = app_with(services);
    let picker = crate::tests::picks::scripted(&mut app);
    app.run("ui.saveGraphicStyleLibrary", json!({})).unwrap();
    confirm_save(&mut app, &[]);
    save_to_tmp(&mut app, &picker);
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
