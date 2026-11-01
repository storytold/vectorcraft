use std::path::PathBuf;

use serde_json::json;
use vectorcraft_engine::Session;

use super::SAVE_COPY;
use crate::{VectorcraftApp, menus};

/// A new, empty folder of its own for test `name`.
fn folder(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("vc-ui-document-tools-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn app() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app
}

/// Click the Document Tools item `label`.
fn click(app: &mut VectorcraftApp, label: &str) {
    let entries = menus::menu_entries(app);
    let item = entries.iter().find(|e| e.path == ["Object", "Plug-ins", "Document Tools"] && e.label == label).expect(label);
    let (id, p) = menus::click_target(&item.label, item.command.as_deref().unwrap(), &item.params);
    menus::invoke(app, &id, p);
}

fn files(dir: &PathBuf) -> Vec<String> {
    std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect()
}

#[test]
fn save_a_timestamped_copy_writes_beside_the_document() {
    let dir = folder("beside");
    let mut app = app();
    app.run("document.save", json!({"path": dir.join("card.vectorcraft")})).unwrap();
    assert_eq!(menus::menu_entries(&app).iter().find(|e| e.label == "Save a Timestamped Copy").and_then(|e| e.command.as_deref()), Some(SAVE_COPY));
    click(&mut app, "Save a Timestamped Copy");
    let names = files(&dir);
    assert_eq!(names.len(), 2, "{names:?}");
    assert!(names.iter().any(|n| n.starts_with("card_20") && n.ends_with(".vectorcraft")), "{names:?}");
    assert!(app.ui.status.starts_with("Saved "), "{}", app.ui.status);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_unsaved_document_asks_for_a_folder() {
    let dir = folder("picked");
    let mut app = app();
    let picked = dir.to_string_lossy().into_owned();
    app.services.pick_folder = Some(Box::new(move || Some(picked.clone())));
    click(&mut app, "Save a Timestamped Copy");
    assert_eq!(files(&dir).len(), 1);
    assert!(app.session.doc().unwrap().path.is_none(), "the document stays unsaved");
    // No folder picked: nothing written.
    app.services.pick_folder = Some(Box::new(|| None));
    assert_eq!(app.run(SAVE_COPY, json!({})).unwrap_err(), "cancelled");
    assert_eq!(files(&dir).len(), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn new_symbols_from_layers_and_convert_layers_to_symbols() {
    let mut app = app();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 30})).unwrap();
    app.run("layer.new", json!({"name": "Badges"})).unwrap();
    app.run("shape.ellipse", json!({"x": 100, "y": 10, "width": 40, "height": 40})).unwrap();
    let symbols = |app: &VectorcraftApp| app.session.doc().unwrap().doc.symbols.iter().map(|s| s.name.clone()).collect::<Vec<_>>();
    let before = symbols(&app).len();
    click(&mut app, "New Symbols from Layers");
    assert_eq!(symbols(&app).len(), before + 2);
    assert!(symbols(&app).contains(&"Badges".to_string()));
    app.run("edit.undo", json!({})).unwrap();
    click(&mut app, "Convert Layers to Symbols");
    let doc = &app.session.doc().unwrap().doc;
    assert!(
        doc.layers
            .iter()
            .all(|l| matches!(l.children().unwrap()[..], [ref one] if matches!(one.kind, vectorcraft_doc::NodeKind::SymbolInstance { .. })))
    );
}
