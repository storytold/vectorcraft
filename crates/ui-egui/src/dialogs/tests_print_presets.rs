//! Print presets in the UI: the Print dialog's preset list and Save Preset…, and Edit → Print
//! Presets (New…, Edit…, Delete, Import…, Export…) with its preset editor, and `file.print` with
//! a preset's name.

use std::cell::RefCell;
use std::rc::Rc;

use serde_json::{Value, json};
use vectorcraft_engine::Session;
use vectorcraft_engine::cmd::printpresets::DEFAULT_PRESET;

use super::print::{KIND, PRESET_KIND, SECTIONS};
use super::print_presets::{self, Action};
use crate::{Services, VectorcraftApp, theme};

fn frame(app: &mut VectorcraftApp) {
    let ctx = egui::Context::default();
    theme::install_fonts(&ctx);
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| super::show(app, ui.ctx()));
    out.textures_delta.clear();
}

/// Press a manager button on the preset `current`, as the body does.
fn press(app: &mut VectorcraftApp, act: Action, current: &str) {
    let mut d = app.ui.dialog.take().unwrap();
    print_presets::run(app, &mut d, act, current).unwrap();
    app.ui.dialog = Some(d);
    frame(app);
}

fn kind(app: &VectorcraftApp) -> Option<&str> {
    app.ui.dialog.as_ref().map(|d| d.kind.as_str())
}

fn field(app: &VectorcraftApp, k: &str) -> Value {
    app.ui.dialog.as_ref().and_then(|d| d.fields.get(k).cloned()).unwrap_or_default()
}

fn set(app: &mut VectorcraftApp, k: &str, v: Value) {
    app.ui.dialog.as_mut().expect("a dialog is open").fields.insert(k.into(), v);
}

#[test]
fn the_print_dialog_saves_and_loads_presets() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    app.run("file.print", json!({})).unwrap();
    assert_eq!(field(&app, "preset"), DEFAULT_PRESET, "never set up: [Default]");
    // Save Preset… asks for a name; OK (and Enter) then saves the preset, not printing.
    set(&mut app, "copies", json!(4));
    set(&mut app, "media", json!("a4"));
    set(&mut app, "__savePresetAs", json!("Proofs"));
    frame(&mut app);
    let r = super::confirm(&mut app).unwrap();
    assert_eq!(r["name"], "Proofs");
    assert_eq!(kind(&app), Some(KIND), "the Print dialog stays open");
    assert_eq!(field(&app, "preset"), "Proofs");
    assert!(field(&app, "__savePresetAs").is_null());
    let saved = &app.session.prefs.print_presets[0].settings;
    assert_eq!((saved.copies, saved.media), (4, vectorcraft_pdf::Media::A4));
    // Setting `preset` (the list, or an agent) loads that preset's settings.
    set(&mut app, "preset", json!(DEFAULT_PRESET));
    frame(&mut app);
    assert_eq!((field(&app, "copies"), field(&app, "media")), (json!(1), json!("letter")));
    set(&mut app, "preset", json!("proofs"));
    set(&mut app, "discard", json!(true));
    super::confirm(&mut app).unwrap();
    let setup = app.session.active().unwrap().doc.print_setup.clone().unwrap();
    assert_eq!((setup["copies"].clone(), setup["media"].clone()), (json!(4), json!("a4")), "Done kept the preset's settings");
    // The next Print knows the preset its settings are.
    app.run("file.print", json!({})).unwrap();
    assert_eq!(field(&app, "preset"), "Proofs");
    frame(&mut app);
    // An unknown preset is refused; [Default]'s name can't be taken.
    set(&mut app, "preset", json!("Nope"));
    set(&mut app, "discard", json!(true));
    assert!(super::confirm(&mut app).is_err());
    assert!(field(&app, "discard").is_null(), "Done's press doesn't stick to the dialog");
    set(&mut app, "__savePresetAs", json!("[Default]"));
    assert!(super::confirm(&mut app).is_err());
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
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    assert!(crate::menus::menu_entries(&app).iter().any(|e| e.command.as_deref() == Some("ui.printPresetsDialog") && e.enabled));
    app.run("ui.printPresetsDialog", json!({})).unwrap();
    frame(&mut app);
    assert_eq!((kind(&app), field(&app, "selected")), (Some(print_presets::KIND), json!(DEFAULT_PRESET)));
    // [Default] can't be edited: New… starts a copy.
    assert!(app.run("ui.printPresetDialog", json!({"name": DEFAULT_PRESET})).is_err());
    app.run("ui.printPresetDialog", json!({"preset": DEFAULT_PRESET})).unwrap();
    assert_eq!((kind(&app), field(&app, "name")), (Some(PRESET_KIND), json!("Print Preset 1")));
    for s in SECTIONS {
        set(&mut app, "__section", json!(s));
        frame(&mut app);
        assert_eq!(kind(&app), Some(PRESET_KIND), "{s}");
    }
    set(&mut app, "name", json!("Tiled"));
    set(&mut app, "scaling", json!("tileFull"));
    super::confirm(&mut app).unwrap();
    assert_eq!((kind(&app), field(&app, "selected")), (Some(print_presets::KIND), json!("Tiled")), "back to the manager");
    frame(&mut app);
    // A new preset can't take a name in use; Edit… renames.
    app.run("ui.printPresetDialog", json!({"preset": "Tiled"})).unwrap();
    assert_eq!(field(&app, "scaling"), "tileFull", "New… starts from the selected preset");
    set(&mut app, "name", json!("tiled"));
    assert!(super::confirm(&mut app).is_err());
    assert_eq!(kind(&app), Some(PRESET_KIND), "stays open on a taken name");
    app.run("ui.printPresetDialog", json!({"name": "Tiled"})).unwrap();
    set(&mut app, "name", json!("Posters"));
    set(&mut app, "overlap", json!(9));
    super::confirm(&mut app).unwrap();
    let p = &app.session.prefs.print_presets;
    assert_eq!((p.len(), p[0].name.as_str(), p[0].settings.overlap), (1, "Posters", 9.0));
    // Delete selects the one above ([Default]).
    press(&mut app, Action::Delete, "Posters");
    assert!(app.session.prefs.print_presets.is_empty());
    assert_eq!(field(&app, "selected"), DEFAULT_PRESET);
    super::confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none(), "Close");
}

#[test]
fn presets_files_import_and_export() {
    let dir = std::env::temp_dir().join(format!("vc-printpresets-ui-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("mine.vcprintpresets").to_string_lossy().to_string();
    let picked = file.clone();
    let services = crate::Services {
        pick_save: Some(Box::new(move |_: &crate::FilePick| Some(picked.clone()))),
        write: Some(Box::new(|p: &str, b: &[u8]| std::fs::write(p, b).map_err(|e| e.to_string()))),
        ..Default::default()
    };
    let mut app = VectorcraftApp::new(Session::new(), services);
    app.run("print.presets.save", json!({"name": "Mine", "settings": {"copies": 2}})).unwrap();
    app.run("ui.printPresetsDialog", json!({"selected": "Mine"})).unwrap();
    press(&mut app, Action::Export, "Mine");
    // Opening the file imports it (also how the web's picked file arrives).
    let bytes = std::fs::read(&file).unwrap();
    crate::io::open_bytes(&mut app, &file, &bytes, Some(file.clone())).unwrap();
    let names: Vec<&str> = app.session.prefs.print_presets.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["Mine", "Mine 2"]);
    assert_eq!(app.session.prefs.print_presets[1].settings.copies, 2);
    assert!(app.ui.status.contains("print presets"), "{}", app.ui.status);
    let _ = std::fs::remove_dir_all(dir);
}
