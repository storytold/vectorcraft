//! Print presets in the UI: the Print dialog's preset list and Save Preset…, and Edit → Print
//! Presets (New…, Edit…, Delete, Import…, Export…) with its preset editor, and `file.print` with
//! a preset's name.

use std::cell::RefCell;
use std::rc::Rc;

use serde_json::json;
use vectorcraft_engine::Session;
use vectorcraft_engine::cmd::printpresets::DEFAULT_PRESET;

use super::preset_manager::{ManagerAction, ManagerEvent, PresetManager};
use super::tests_print::{call, confirm, fields, is_open, pump, set, try_set};
use crate::{Services, VectorcraftApp};

/// A manager button, as the manager files it; the pump carries it out.
fn press(app: &mut VectorcraftApp, act: ManagerAction) {
    let dialogs = app.dialogs().expect("the test app has a window");
    dialogs.with(|h| h.get_mut::<PresetManager>().expect("the manager is open").queue(ManagerEvent::Action(act)));
    pump(app);
}

fn new_app(services: Services) -> VectorcraftApp {
    crate::testing::windowed(VectorcraftApp::new(Session::new(), services))
}

#[test]
fn the_print_dialog_saves_and_loads_presets() {
    let mut app = new_app(Default::default());
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    app.run("file.print", json!({})).unwrap();
    pump(&mut app);
    assert_eq!(fields(&app)["preset"], DEFAULT_PRESET, "never set up: [Default]");
    // Save Preset (the preset row's New…, as the framework names it) asks for a name; Enter then saves the preset,
    // not printing.
    set(&mut app, "copies", json!(4));
    set(&mut app, "media", json!("a4"));
    super::tests_print::press(&mut app, "New…");
    // The name field takes the keyboard in the frame after the press.
    pump(&mut app);
    call(&mut app, "ui.text", json!({"text": "Proofs"}));
    call(&mut app, "ui.key", json!({"key": "Enter"}));
    pump(&mut app);
    assert!(is_open(&app), "the Print dialog stays open");
    assert_eq!(fields(&app)["preset"], "Proofs");
    let saved = &app.session.prefs.print_presets[0].settings;
    assert_eq!((saved.copies, saved.media), (4, vectorcraft_pdf::Media::A4));
    // Applying a preset (the list, or an agent) loads its settings.
    set(&mut app, "preset", json!(DEFAULT_PRESET));
    pump(&mut app);
    assert_eq!((fields(&app)["copies"].clone(), fields(&app)["media"].clone()), (json!(1), json!("letter")));
    set(&mut app, "preset", json!("proofs"));
    pump(&mut app);
    set(&mut app, "choice", json!("done"));
    confirm(&mut app);
    let setup = app.session.active().unwrap().doc.print_setup.clone().unwrap();
    assert_eq!((setup["copies"].clone(), setup["media"].clone()), (json!(4), json!("a4")), "Done kept the preset's settings");
    // The next Print knows the preset its settings are.
    app.run("file.print", json!({})).unwrap();
    pump(&mut app);
    assert_eq!(fields(&app)["preset"], "Proofs");
    // An unknown preset is refused.
    assert!(try_set(&mut app, "preset", json!("Nope")).is_err());
    assert_eq!(app.session.prefs.print_presets.len(), 1);
}

#[test]
fn file_print_with_params_prints_a_preset_by_name() {
    let written: Rc<RefCell<Vec<Vec<u8>>>> = Rc::default();
    let w = written.clone();
    let write = move |_: &str, b: &[u8]| {
        w.borrow_mut().push(b.to_vec());
        Ok(())
    };
    let mut app = VectorcraftApp::new(Session::new(), Services { write: Some(Box::new(write)), ..Default::default() });
    app.run("file.new", json!({"width": 200, "height": 100, "artboards": 4})).unwrap();
    app.run("print.presets.save", json!({"name": "Sheet", "settings": {"artboards": "range", "range": "4"}})).unwrap();
    let r = app.run("file.print", json!({"path": "/tmp/sheet.pdf", "preset": "Sheet"})).unwrap();
    assert_eq!(r["pages"], 1, "{r}");
    assert_eq!(vectorcraft_pdf::info(&written.borrow()[0], None).unwrap().pages.len(), 1);
}

#[test]
fn the_manager_adds_edits_and_deletes_presets_through_the_editor() {
    // Edit → Print Presets works without a document too.
    let mut app = new_app(Default::default());
    assert!(crate::menus::menu_entries(&app).iter().any(|e| e.command.as_deref() == Some("ui.printPresetsDialog") && e.enabled));
    app.run("ui.printPresetsDialog", json!({})).unwrap();
    pump(&mut app);
    assert_eq!(fields(&app)["selected"], DEFAULT_PRESET);
    // [Default] can't be edited: New… starts a copy.
    assert!(app.run("ui.printPresetDialog", json!({"name": DEFAULT_PRESET})).is_err());
    app.run("ui.printPresetDialog", json!({"preset": DEFAULT_PRESET})).unwrap();
    pump(&mut app);
    assert_eq!(fields(&app)["name"], "Print Preset 1");
    set(&mut app, "name", json!("Tiled"));
    set(&mut app, "scaling", json!("tileFull"));
    confirm(&mut app);
    assert_eq!(fields(&app)["selected"], "Tiled", "back to the manager");
    // A new preset can't take a name in use; Edit… renames.
    app.run("ui.printPresetDialog", json!({"preset": "Tiled"})).unwrap();
    pump(&mut app);
    assert_eq!(fields(&app)["scaling"], "tileFull", "New… starts from the selected preset");
    set(&mut app, "name", json!("tiled"));
    confirm(&mut app);
    assert!(fields(&app).get("name").is_some(), "stays open on a taken name");
    app.run("ui.printPresetDialog", json!({"name": "Tiled"})).unwrap();
    pump(&mut app);
    set(&mut app, "name", json!("Posters"));
    set(&mut app, "overlap", json!(9));
    confirm(&mut app);
    let p = &app.session.prefs.print_presets;
    assert_eq!((p.len(), p[0].name.as_str(), p[0].settings.overlap), (1, "Posters", 9.0));
    // Delete selects the one above ([Default]).
    press(&mut app, ManagerAction::Delete);
    assert!(app.session.prefs.print_presets.is_empty());
    assert_eq!(fields(&app)["selected"], DEFAULT_PRESET);
    confirm(&mut app);
    assert!(!is_open(&app), "Close");
}

#[test]
fn presets_files_import_and_export() {
    let dir = std::env::temp_dir().join(format!("vc-printpresets-ui-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("mine.vcprintpresets").to_string_lossy().to_string();
    let mut app = new_app(Default::default());
    let picker = crate::tests_picks::scripted(&mut app);
    app.services.write = Some(Box::new(|p: &str, b: &[u8]| std::fs::write(p, b).map_err(|e| e.to_string())));
    app.run("print.presets.save", json!({"name": "Mine", "settings": {"copies": 2}})).unwrap();
    app.run("ui.printPresetsDialog", json!({"selected": "Mine"})).unwrap();
    pump(&mut app);
    press(&mut app, ManagerAction::Export);
    // The save panel suggests the preset's name.
    assert_eq!(picker.requests().last().and_then(|r| r.file_name.clone()).as_deref(), Some("Mine.vcprintpresets"));
    picker.answer_saved(&file, None);
    let ctx = app.ui_ctx.clone().expect("the test app has a window");
    crate::tests_picks::poll(&mut app, &ctx);
    // Opening the file imports it (also how the web's picked file arrives).
    let bytes = std::fs::read(&file).unwrap();
    crate::io::open_bytes(&mut app, &file, &bytes, Some(file.clone())).unwrap();
    let names: Vec<&str> = app.session.prefs.print_presets.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["Mine", "Mine 2"]);
    assert_eq!(app.session.prefs.print_presets[1].settings.copies, 2);
    assert!(app.ui.status.contains("print presets"), "{}", app.ui.status);
    // The built-in preset's file is named without its brackets.
    app.run("ui.printPresetsDialog", json!({"selected": DEFAULT_PRESET})).unwrap();
    pump(&mut app);
    press(&mut app, ManagerAction::Export);
    assert_eq!(picker.requests().last().and_then(|r| r.file_name.clone()).as_deref(), Some("Default.vcprintpresets"));
    picker.cancel();
    let _ = std::fs::remove_dir_all(dir);
}
