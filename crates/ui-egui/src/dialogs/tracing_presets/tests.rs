//! Edit → Tracing Presets: the menu item, New… and Edit… through the preset editor, Delete,
//! Import… (with the question about saved presets of the same names) and Export….

use serde_json::{Value, json};
use vectorcraft_engine::Session;

use super::*;

fn frame(app: &mut VectorcraftApp) {
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| super::super::show(app, ui.ctx()));
    out.textures_delta.clear();
}

/// Press a manager button on the preset `current`, as the body does.
fn press(app: &mut VectorcraftApp, act: Action, current: &str) -> Result<(), String> {
    let mut d = app.ui.dialog.take().expect("a dialog is open");
    let r = run(app, &mut d, act, current);
    app.ui.dialog = Some(d);
    frame(app);
    r
}

/// The texts the dialog paints on a 1200 × 900 window, once it has taken its sizing frame and
/// faded in (each frame is a quarter second on).
fn frame_texts(app: &mut VectorcraftApp) -> Vec<String> {
    fn walk(s: &egui::Shape, out: &mut Vec<String>) {
        match s {
            egui::Shape::Text(t) => out.push(t.galley.text().to_string()),
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
            _ => {}
        }
    }
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 900.0));
    let mut texts = vec![];
    for k in 0..4 {
        let raw = egui::RawInput { screen_rect: Some(screen), time: Some(f64::from(k) * 0.25), ..Default::default() };
        let mut out = ctx.run_ui(raw, |ui| super::super::show(app, ui.ctx()));
        out.textures_delta.clear();
        texts.clear();
        out.shapes.iter().for_each(|c| walk(&c.shape, &mut texts));
    }
    texts
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

/// The tracing settings of the preset `name` as `imageTrace.presets` lists them.
fn listed(app: &mut VectorcraftApp, name: &str) -> Value {
    let all = app.session.execute("imageTrace.presets", &json!({})).unwrap();
    all["presets"].as_array().unwrap().iter().find(|p| p["name"] == name).map(|p| p["params"].clone()).unwrap_or_default()
}

fn saved_names(app: &VectorcraftApp) -> Vec<&str> {
    app.session.prefs.trace_presets.iter().map(|p| p.name.as_str()).collect()
}

#[test]
fn edit_menu_opens_the_manager_without_a_document() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    let entries = crate::menus::menu_entries(&app);
    let item = entries.iter().find(|e| e.command.as_deref() == Some("ui.tracingPresetsDialog")).expect("Edit → Tracing Presets…");
    assert!(item.path == ["Edit"] && item.label == "Tracing Presets…" && item.enabled);
    app.run("ui.tracingPresetsDialog", json!({})).unwrap();
    frame(&mut app);
    assert_eq!((kind(&app), field(&app, "selected")), (Some(KIND), json!("Default")));
    app.run("ui.tracingPresetsDialog", json!({"selected": "[6 Colors]"})).unwrap();
    frame(&mut app);
    assert_eq!(field(&app, "selected"), json!("[6 Colors]"));
    // Built-in presets can't be edited or deleted.
    assert!(press(&mut app, Action::Edit, "6 Colors").is_err());
    assert!(press(&mut app, Action::Delete, "6 Colors").is_err());
    assert!(app.session.prefs.trace_presets.is_empty());
    super::super::confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none(), "Close");
}

#[test]
fn new_and_edit_save_through_the_editor_and_cancel_saves_nothing() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    let revision = app.session.trace_presets_revision();
    app.run("ui.tracingPresetsDialog", json!({"selected": "6 Colors"})).unwrap();
    frame(&mut app);
    // New… starts from the selected built-in preset; Cancel comes back with nothing saved.
    press(&mut app, Action::New, "6 Colors").unwrap();
    assert_eq!((kind(&app), field(&app, "name")), (Some(PRESET_KIND), json!("Tracing Preset 1")));
    let six = listed(&mut app, "6 Colors");
    assert_eq!(field(&app, "params"), six);
    set(&mut app, "name", json!("Poster"));
    super::super::cancel(&mut app);
    assert_eq!((kind(&app), field(&app, "selected")), (Some(KIND), json!("6 Colors")), "back to the manager");
    assert!(app.session.prefs.trace_presets.is_empty());
    assert_eq!(app.session.trace_presets_revision(), revision);
    // OK saves the preset with the editor's settings and selects it.
    press(&mut app, Action::New, "6 Colors").unwrap();
    set(&mut app, "name", json!(" Poster "));
    let mut params = field(&app, "params");
    params["noise"] = json!(40);
    set(&mut app, "params", params);
    frame(&mut app);
    super::super::confirm(&mut app).unwrap();
    assert_eq!((kind(&app), field(&app, "selected")), (Some(KIND), json!("Poster")));
    assert_eq!(saved_names(&app), ["Poster"]);
    let poster = listed(&mut app, "Poster");
    assert_eq!((poster["noise"].clone(), poster["colors"].clone()), (json!(40), six["colors"].clone()));
    // The Image Trace panel's list (also the Control bar's and Properties') has it.
    let ctx = egui::Context::default();
    let all = crate::panels::image_trace::presets(&mut app, &ctx);
    assert!(all.iter().any(|p| p.0 == "Poster" && !p.3));
    // Edit… changes the saved preset; Cancel leaves it as it was.
    frame(&mut app);
    press(&mut app, Action::Edit, "Poster").unwrap();
    assert_eq!((kind(&app), field(&app, "name"), field(&app, "params")), (Some(PRESET_KIND), json!("Poster"), poster.clone()));
    let mut params = field(&app, "params");
    params["noise"] = json!(3);
    set(&mut app, "params", params);
    super::super::cancel(&mut app);
    assert_eq!(listed(&mut app, "Poster"), poster);
    press(&mut app, Action::Edit, "Poster").unwrap();
    let mut params = field(&app, "params");
    params["noise"] = json!(3);
    params["mode"] = json!("grayscale");
    set(&mut app, "params", params);
    set(&mut app, "name", json!("Posters"));
    frame(&mut app);
    super::super::confirm(&mut app).unwrap();
    assert_eq!((kind(&app), field(&app, "selected")), (Some(KIND), json!("Posters")));
    assert_eq!(saved_names(&app), ["Posters"]);
    let edited = listed(&mut app, "Posters");
    assert_eq!((edited["noise"].clone(), edited["mode"].clone()), (json!(3), json!("grayscale")));
    // New… from a saved preset starts from its settings.
    press(&mut app, Action::New, "Posters").unwrap();
    assert_eq!((field(&app, "name"), field(&app, "params")), (json!("Tracing Preset 1"), edited));
}

#[test]
fn the_editor_shows_document_library_s_library_and_colors() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("ui.tracingPresetsDialog", json!({})).unwrap();
    frame(&mut app);
    press(&mut app, Action::New, "Default").unwrap();
    let mut params = field(&app, "params");
    params["mode"] = json!("color");
    params["palette"] = json!("documentLibrary");
    params["colorGroup"] = json!("Brand Colors");
    set(&mut app, "params", params.clone());
    let texts = frame_texts(&mut app);
    for t in ["Library:", "Colors:", "Brand Colors"] {
        assert!(texts.iter().any(|s| s == t), "{t}: {texts:?}");
    }
    // Another library's colors are all its swatches: no Colors menu.
    params["library"] = json!("lib:Other");
    params["colorGroup"] = Value::Null;
    set(&mut app, "params", params.clone());
    let texts = frame_texts(&mut app);
    assert!(texts.iter().any(|s| s == "Library:") && !texts.iter().any(|s| s == "Colors:"), "{texts:?}");
    // Other palettes show neither.
    params["palette"] = json!("limited");
    set(&mut app, "params", params);
    let texts = frame_texts(&mut app);
    assert!(!texts.iter().any(|s| s == "Library:" || s == "Colors:"), "{texts:?}");
}

#[test]
fn a_taken_or_protected_name_keeps_the_editor_open() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("imageTrace.presets.save", json!({"name": "Ink", "params": {"threshold": 90}})).unwrap();
    app.run("imageTrace.presets.save", json!({"name": "Poster", "params": {"noise": 40}})).unwrap();
    let saved = app.session.prefs.trace_presets.clone();
    app.run("ui.tracingPresetsDialog", json!({"selected": "Ink"})).unwrap();
    frame(&mut app);
    press(&mut app, Action::New, "Ink").unwrap();
    for name in ["Default", "[6 Colors]", "custom", "poster"] {
        set(&mut app, "name", json!(name));
        assert!(super::super::confirm(&mut app).is_err(), "{name}");
        assert_eq!((kind(&app), field(&app, "name")), (Some(PRESET_KIND), json!(name)), "{name}: stays open");
    }
    super::super::cancel(&mut app);
    press(&mut app, Action::Edit, "Ink").unwrap();
    for name in ["Poster", "Default", "Custom"] {
        set(&mut app, "name", json!(name));
        assert!(super::super::confirm(&mut app).is_err(), "{name}");
        assert_eq!(kind(&app), Some(PRESET_KIND), "{name}: stays open");
    }
    assert_eq!(app.session.prefs.trace_presets, saved, "nothing changed");
}

#[test]
fn delete_selects_the_next_saved_preset_then_default() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    for name in ["A", "B", "C"] {
        app.run("imageTrace.presets.save", json!({ "name": name })).unwrap();
    }
    app.run("ui.tracingPresetsDialog", json!({"selected": "B"})).unwrap();
    frame(&mut app);
    press(&mut app, Action::Delete, "B").unwrap();
    assert_eq!((saved_names(&app), field(&app, "selected")), (vec!["A", "C"], json!("C")));
    press(&mut app, Action::Delete, "C").unwrap();
    assert_eq!((saved_names(&app), field(&app, "selected")), (vec!["A"], json!("A")));
    press(&mut app, Action::Delete, "A").unwrap();
    assert!(app.session.prefs.trace_presets.is_empty());
    assert_eq!(field(&app, "selected"), json!("Default"));
}

#[test]
fn import_asks_before_replacing_saved_presets_and_export_writes_every_saved_one() {
    let dir = std::env::temp_dir().join(format!("vc-tracing-presets-ui-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("mine.vctracepresets").to_string_lossy().to_string();
    let picked = file.clone();
    let services = crate::Services {
        pick_save: Some(Box::new(move |_: &crate::FilePick| Some(picked.clone()))),
        write: Some(Box::new(|p: &str, b: &[u8]| std::fs::write(p, b).map_err(|e| e.to_string()))),
        read: Some(Box::new(|p: &str| std::fs::read(p).map_err(|e| e.to_string()))),
        ..Default::default()
    };
    let mut app = VectorcraftApp::new(Session::new(), services);
    app.run("imageTrace.presets.save", json!({"name": "Poster", "params": {"noise": 40}})).unwrap();
    app.run("imageTrace.presets.save", json!({"name": "Ink", "params": {"threshold": 90}})).unwrap();
    app.run("ui.tracingPresetsDialog", json!({"selected": "Default"})).unwrap();
    frame(&mut app);
    press(&mut app, Action::Export, "").unwrap();
    let text = std::fs::read_to_string(&file).unwrap();
    let written: Value = serde_json::from_str(&text).unwrap();
    let names: Vec<&str> = written["presets"].as_array().unwrap().iter().filter_map(|p| p["name"].as_str()).collect();
    assert_eq!((written["format"].as_str(), names), (Some(TRACE_PRESET_FORMAT), vec!["Poster", "Ink"]));
    // Saved presets of the same names stay unless the question's Replace is pressed.
    app.run("imageTrace.presets.save", json!({"name": "Poster", "params": {"noise": 7}})).unwrap();
    app.run("imageTrace.presets.delete", json!({"name": "Ink"})).unwrap();
    let mut d = app.ui.dialog.take().unwrap();
    import(&mut app, &mut d, json!({ "path": file })).unwrap();
    app.ui.dialog = Some(d);
    frame(&mut app);
    assert_eq!((kind(&app), field(&app, "names")), (Some(REPLACE_KIND), json!(["Poster"])));
    assert_eq!(saved_names(&app), ["Poster", "Ink"], "Ink came in");
    super::super::cancel(&mut app);
    assert_eq!((kind(&app), field(&app, "selected")), (Some(KIND), json!("Ink")), "Cancel keeps the saved one");
    assert_eq!(app.session.trace_preset("Poster").unwrap().params.noise, 7);
    let mut d = app.ui.dialog.take().unwrap();
    import(&mut app, &mut d, json!({ "path": file })).unwrap();
    app.ui.dialog = Some(d);
    // Replace acts on the contents the question was asked about, though the file changed since.
    let changed =
        r#"{"format": "vctracepresets", "version": 1, "presets": [{"name": "Poster", "params": {"noise": 3}}, {"name": "Extra", "params": {}}]}"#;
    std::fs::write(&file, changed).unwrap();
    super::super::confirm(&mut app).unwrap();
    std::fs::write(&file, &text).unwrap();
    assert_eq!((kind(&app), field(&app, "selected")), (Some(KIND), json!("Poster")));
    assert_eq!(app.session.trace_preset("Poster").unwrap().params.noise, 40);
    assert_eq!(saved_names(&app), ["Poster", "Ink"]);
    // Opening the file imports it, and asks the same question over the manager (also how the web's
    // picked file arrives).
    app.run("imageTrace.presets.save", json!({"name": "Poster", "params": {"noise": 7}})).unwrap();
    app.run("imageTrace.presets.delete", json!({"name": "Ink"})).unwrap();
    let bytes = std::fs::read(&file).unwrap();
    crate::io::open_bytes(&mut app, &file, &bytes, Some(file.clone())).unwrap();
    assert_eq!(saved_names(&app), ["Poster", "Ink"]);
    assert!(app.ui.status.contains("tracing presets"), "{}", app.ui.status);
    assert_eq!((kind(&app), field(&app, "names")), (Some(REPLACE_KIND), json!(["Poster"])));
    frame(&mut app);
    super::super::confirm(&mut app).unwrap();
    assert_eq!((kind(&app), field(&app, "selected")), (Some(KIND), json!("Poster")));
    assert_eq!(app.session.trace_preset("Poster").unwrap().params.noise, 40);
    // With no dialog open, Cancel keeps the saved preset and leaves no dialog.
    app.ui.dialog = None;
    app.run("imageTrace.presets.save", json!({"name": "Poster", "params": {"noise": 7}})).unwrap();
    crate::io::open_bytes(&mut app, &file, &bytes, Some(file.clone())).unwrap();
    assert_eq!(kind(&app), Some(REPLACE_KIND));
    super::super::cancel(&mut app);
    assert!(app.ui.dialog.is_none());
    assert_eq!(app.session.trace_preset("Poster").unwrap().params.noise, 7);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_file_with_a_fault_changes_nothing() {
    let dir = std::env::temp_dir().join(format!("vc-tracing-presets-bad-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("bad.vctracepresets");
    std::fs::write(
        &file,
        r#"{"format": "vctracepresets", "version": 1, "presets": [{"name": "Fine", "params": {}}, {"name": "Default", "params": {}}]}"#,
    )
    .unwrap();
    // The file is read no further than one byte past the limit.
    let read_head = |p: &str, max: usize| {
        use std::io::Read as _;
        assert_eq!(max, TRACE_PRESET_MAX_BYTES + 1);
        let mut bytes = vec![];
        std::fs::File::open(p).and_then(|f| f.take(max as u64).read_to_end(&mut bytes)).map_err(|e| e.to_string())?;
        Ok(bytes)
    };
    let services = crate::Services { read_head: Some(Box::new(read_head)), ..Default::default() };
    let mut app = VectorcraftApp::new(Session::new(), services);
    app.run("ui.tracingPresetsDialog", json!({})).unwrap();
    frame(&mut app);
    let mut d = app.ui.dialog.take().unwrap();
    let e = import(&mut app, &mut d, json!({ "path": file.to_string_lossy() })).unwrap_err();
    assert!(e.contains("built-in preset"), "{e}");
    assert_eq!(d.kind, KIND, "no question");
    assert!(app.session.prefs.trace_presets.is_empty());
    app.ui.dialog = Some(d);
    // An opened file over the size limit is refused before its bytes are encoded for the import.
    let big = vec![b' '; TRACE_PRESET_MAX_BYTES + 1];
    let e = open_file(&mut app, &big).unwrap_err();
    assert!(e.contains("tracing presets file is larger than 4 MiB"), "{e}");
    assert_eq!(kind(&app), Some(KIND), "the manager stays open");
    let big_file = dir.join("big.vctracepresets");
    std::fs::write(&big_file, &big).unwrap();
    let mut d = app.ui.dialog.take().unwrap();
    let e = import(&mut app, &mut d, json!({ "path": big_file.to_string_lossy() })).unwrap_err();
    assert!(e.contains("tracing presets file is larger than 4 MiB"), "{e}");
    app.ui.dialog = Some(d);
    assert!(app.session.prefs.trace_presets.is_empty());
    let _ = std::fs::remove_dir_all(dir);
}
