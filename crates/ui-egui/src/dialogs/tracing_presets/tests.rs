//! Edit → Tracing Presets: New… and Edit… through the preset editor, Delete, and the import with
//! the question about saved presets of the same names, all through the dialog host.

use craft::ui::dialog::dialog_id;
use craft::ui::patterns::form_dialogs::{FieldValue, HostedParamDialog};
use serde_json::{Value, json};
use vectorcraft_engine::Session;

use super::*;

/// An app with a window, and the context of it.
fn windowed(services: crate::Services) -> (VectorcraftApp, egui::Context) {
    let mut app = VectorcraftApp::new(Session::new(), services);
    let ctx = crate::testing::ui_context();
    app.adopt_context(&ctx);
    (app, ctx)
}

/// A frame of the dialogs and their pump; a second one shows what the first opened.
fn pass(app: &mut VectorcraftApp, ctx: &egui::Context) {
    let dialogs = app.dialogs().expect("a window");
    for _ in 0..2 {
        ctx.run_ui(egui::RawInput::default(), |ui| dialogs.with(|host| host.run(ui))).textures_delta.clear();
        // The dialog layer's pump: the module's, then the held OKs it answers.
        crate::dialogs::host::pump(app, ctx);
    }
}

/// Press a manager button, as its click files it.
fn press(app: &mut VectorcraftApp, ctx: &egui::Context, act: Action) {
    assert!(with_manager(app, |m| m.action = Some(act)).is_some(), "the manager is open");
    pass(app, ctx);
}

fn selected(app: &VectorcraftApp) -> Option<String> {
    with_manager(app, |m| m.selected.clone())
}

fn editor_open(app: &VectorcraftApp) -> bool {
    app.dialogs().is_some_and(|d| d.with(|h| h.get_by_id::<HostedParamDialog>(dialog_id(PRESET_KIND)).is_some()))
}

/// A value of the open editor.
fn field(app: &VectorcraftApp, key: &str) -> Value {
    let read = |h: &mut craft::ui::dialog::DialogHost| {
        h.get_by_id::<HostedParamDialog>(dialog_id(PRESET_KIND)).and_then(|d| match d.fields().get(key) {
            Some(FieldValue::Json(v)) => Some(v.clone()),
            Some(FieldValue::Text(t)) => Some(json!(t)),
            _ => None,
        })
    };
    app.dialogs().and_then(|d| d.with(read)).unwrap_or_default()
}

fn set(app: &VectorcraftApp, ctx: &egui::Context, key: &str, value: Value) {
    let id = dialog_id(PRESET_KIND);
    app.dialogs().expect("a window").with(|h| h.set_json_field(ctx, id, key, value)).unwrap();
}

/// OK or Cancel of the editor; the pump then answers a held OK.
fn finish(app: &mut VectorcraftApp, ctx: &egui::Context, ok: bool) {
    let id = dialog_id(PRESET_KIND);
    app.dialogs().expect("a window").with(|h| if ok { h.confirm_by_id(ctx, id) } else { h.cancel_by_id(ctx, id) });
    pass(app, ctx);
}

/// The texts the dialogs paint, once they have faded in (each frame is a quarter second on).
fn frame_texts(app: &VectorcraftApp, ctx: &egui::Context) -> Vec<String> {
    frame_runs(app, ctx).into_iter().map(|(text, _, _)| text).collect()
}

/// The texts the dialogs paint with where each is drawn and its font, once they have faded in.
fn frame_runs(app: &VectorcraftApp, ctx: &egui::Context) -> Vec<(String, egui::Rect, Option<egui::FontId>)> {
    fn walk(s: &egui::Shape, out: &mut Vec<(String, egui::Rect, Option<egui::FontId>)>) {
        match s {
            egui::Shape::Text(t) => {
                let font = t.galley.job.sections.first().map(|section| section.format.font_id.clone());
                out.push((t.galley.text().to_string(), t.galley.rect.translate(t.pos.to_vec2()), font));
            }
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
            _ => {}
        }
    }
    let dialogs = app.dialogs().expect("a window");
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 1200.0));
    let mut texts = vec![];
    for k in 0..4 {
        let raw = egui::RawInput { screen_rect: Some(screen), time: Some(f64::from(k) * 0.25), ..Default::default() };
        let mut out = ctx.run_ui(raw, |ui| dialogs.with(|host| host.run(ui)));
        out.textures_delta.clear();
        texts.clear();
        out.shapes.iter().for_each(|c| walk(&c.shape, &mut texts));
    }
    texts
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
fn new_and_edit_save_through_the_editor_and_cancel_saves_nothing() {
    let (mut app, ctx) = windowed(Default::default());
    let revision = app.session.trace_presets_revision();
    app.run("ui.tracingPresetsDialog", json!({"selected": "6 Colors"})).unwrap();
    pass(&mut app, &ctx);
    // New… starts from the selected built-in preset; Cancel saves nothing.
    press(&mut app, &ctx, Action::New);
    assert_eq!(field(&app, "name"), json!("Tracing Preset 1"));
    let six = listed(&mut app, "6 Colors");
    assert_eq!(field(&app, "params"), six);
    set(&app, &ctx, "name", json!("Poster"));
    finish(&mut app, &ctx, false);
    assert!(!editor_open(&app));
    assert_eq!(selected(&app).as_deref(), Some("6 Colors"), "back on the manager");
    assert!(app.session.prefs.trace_presets.is_empty());
    assert_eq!(app.session.trace_presets_revision(), revision);
    // OK saves the preset with the editor's settings and selects it.
    press(&mut app, &ctx, Action::New);
    set(&app, &ctx, "name", json!(" Poster "));
    let mut params = field(&app, "params");
    params["noise"] = json!(40);
    set(&app, &ctx, "params", params);
    finish(&mut app, &ctx, true);
    assert!(!editor_open(&app));
    assert_eq!((selected(&app).as_deref(), saved_names(&app)), (Some("Poster"), vec!["Poster"]));
    let poster = listed(&mut app, "Poster");
    assert_eq!((poster["noise"].clone(), poster["colors"].clone()), (json!(40), six["colors"].clone()));
    // Edit… changes the saved preset; Cancel leaves it as it was.
    press(&mut app, &ctx, Action::Edit);
    assert_eq!((field(&app, "name"), field(&app, "params")), (json!("Poster"), poster.clone()));
    let mut params = field(&app, "params");
    params["noise"] = json!(3);
    set(&app, &ctx, "params", params.clone());
    finish(&mut app, &ctx, false);
    assert_eq!(listed(&mut app, "Poster"), poster);
    press(&mut app, &ctx, Action::Edit);
    params["mode"] = json!("grayscale");
    set(&app, &ctx, "params", params);
    set(&app, &ctx, "name", json!("Posters"));
    finish(&mut app, &ctx, true);
    assert_eq!((selected(&app).as_deref(), saved_names(&app)), (Some("Posters"), vec!["Posters"]));
    let edited = listed(&mut app, "Posters");
    assert_eq!((edited["noise"].clone(), edited["mode"].clone()), (json!(3), json!("grayscale")));
    // New… from a saved preset starts from its settings.
    press(&mut app, &ctx, Action::New);
    assert_eq!((field(&app, "name"), field(&app, "params")), (json!("Tracing Preset 1"), edited));
}

#[test]
fn the_selected_preset_shows_beside_the_list_under_a_strong_name() {
    let (mut app, ctx) = windowed(Default::default());
    app.run("ui.tracingPresetsDialog", json!({})).unwrap();
    pass(&mut app, &ctx);
    let runs = frame_runs(&app, &ctx);
    let rect = |names: &[&str]| runs.iter().find(|r| names.contains(&r.0.as_str())).map(|r| r.1);
    let (Some(export), Some(mode)) = (rect(&["Export…"]), rect(&["Mode", "Mode:"])) else { panic!("Export… and Mode: {runs:?}") };
    // The settings column starts right of the list's buttons and level with the list, not under it.
    assert!(mode.left() >= export.right() && mode.top() < export.top(), "Mode {mode:?}, Export… {export:?}");
    // The selected preset's name over its settings is set apart from its row in the list.
    let names: Vec<_> = runs.iter().filter(|r| r.0 == "[Default]").collect();
    assert_eq!(names.len(), 2, "{runs:?}");
    assert_ne!(names[0].2, names[1].2, "the list row and the name: {names:?}");
}

#[test]
fn the_editor_shows_document_library_s_library_and_colors() {
    let (mut app, ctx) = windowed(Default::default());
    app.run("ui.tracingPresetsDialog", json!({})).unwrap();
    pass(&mut app, &ctx);
    press(&mut app, &ctx, Action::New);
    let mut params = field(&app, "params");
    params["mode"] = json!("color");
    params["palette"] = json!("documentLibrary");
    params["colorGroup"] = json!("Brand Colors");
    set(&app, &ctx, "params", params.clone());
    let texts = frame_texts(&app, &ctx);
    for t in ["Library:", "Colors:", "Brand Colors"] {
        assert!(texts.iter().any(|s| s == t), "{t}: {texts:?}");
    }
    // Another library's colors are all its swatches: no Colors menu.
    params["library"] = json!("lib:Other");
    params["colorGroup"] = Value::Null;
    set(&app, &ctx, "params", params.clone());
    let texts = frame_texts(&app, &ctx);
    assert!(texts.iter().any(|s| s == "Library:") && !texts.iter().any(|s| s == "Colors:"), "{texts:?}");
    // Other palettes show neither.
    params["palette"] = json!("limited");
    set(&app, &ctx, "params", params);
    let texts = frame_texts(&app, &ctx);
    assert!(!texts.iter().any(|s| s == "Library:" || s == "Colors:"), "{texts:?}");
}

#[test]
fn a_taken_or_protected_name_keeps_the_editor_open() {
    let (mut app, ctx) = windowed(Default::default());
    app.run("imageTrace.presets.save", json!({"name": "Ink", "params": {"threshold": 90}})).unwrap();
    app.run("imageTrace.presets.save", json!({"name": "Poster", "params": {"noise": 40}})).unwrap();
    let saved = app.session.prefs.trace_presets.clone();
    app.run("ui.tracingPresetsDialog", json!({"selected": "Ink"})).unwrap();
    pass(&mut app, &ctx);
    press(&mut app, &ctx, Action::New);
    for name in ["Default", "[6 Colors]", "custom", "poster"] {
        set(&app, &ctx, "name", json!(name));
        finish(&mut app, &ctx, true);
        assert!(editor_open(&app), "{name}: stays open");
        assert_eq!(field(&app, "name"), json!(name));
    }
    finish(&mut app, &ctx, false);
    press(&mut app, &ctx, Action::Edit);
    for name in ["Poster", "Default", "Custom"] {
        set(&app, &ctx, "name", json!(name));
        finish(&mut app, &ctx, true);
        assert!(editor_open(&app), "{name}: stays open");
    }
    assert_eq!(app.session.prefs.trace_presets, saved, "nothing changed");
}

#[test]
fn delete_selects_the_next_saved_preset_then_default() {
    let (mut app, ctx) = windowed(Default::default());
    for name in ["A", "B", "C"] {
        app.run("imageTrace.presets.save", json!({ "name": name })).unwrap();
    }
    app.run("ui.tracingPresetsDialog", json!({"selected": "B"})).unwrap();
    pass(&mut app, &ctx);
    press(&mut app, &ctx, Action::Delete);
    assert_eq!((saved_names(&app), selected(&app).as_deref()), (vec!["A", "C"], Some("C")));
    press(&mut app, &ctx, Action::Delete);
    assert_eq!((saved_names(&app), selected(&app).as_deref()), (vec!["A"], Some("A")));
    press(&mut app, &ctx, Action::Delete);
    assert!(app.session.prefs.trace_presets.is_empty());
    assert_eq!(selected(&app).as_deref(), Some("Default"));
}

#[test]
fn import_asks_before_replacing_saved_presets() {
    let dir = std::env::temp_dir().join(format!("vc-tracing-presets-ui-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("mine.vctracepresets").to_string_lossy().to_string();
    let services = crate::Services {
        write: Some(Box::new(|p: &str, b: &[u8]| std::fs::write(p, b).map_err(|e| e.to_string()))),
        read: Some(Box::new(|p: &str| std::fs::read(p).map_err(|e| e.to_string()))),
        ..Default::default()
    };
    let (mut app, ctx) = windowed(services);
    app.run("imageTrace.presets.save", json!({"name": "Poster", "params": {"noise": 40}})).unwrap();
    app.run("imageTrace.presets.save", json!({"name": "Ink", "params": {"threshold": 90}})).unwrap();
    app.run("imageTrace.presets.export", json!({"path": file})).unwrap();
    let text = std::fs::read_to_string(&file).unwrap();
    // Saved presets of the same names stay unless the question's Replace is pressed.
    app.run("imageTrace.presets.save", json!({"name": "Poster", "params": {"noise": 7}})).unwrap();
    app.run("imageTrace.presets.delete", json!({"name": "Ink"})).unwrap();
    app.run("ui.tracingPresetsDialog", json!({})).unwrap();
    pass(&mut app, &ctx);
    let replace = dialog_id(REPLACE_KIND);
    let asking = |app: &VectorcraftApp| app.dialogs().is_some_and(|d| d.with(|h| h.is_open::<ReplacePresets>()));
    open_path(&mut app, &file).unwrap();
    pass(&mut app, &ctx);
    assert!(asking(&app));
    assert_eq!(saved_names(&app), ["Poster", "Ink"], "Ink came in");
    assert_eq!(selected(&app).as_deref(), Some("Ink"));
    // Cancel keeps the saved one.
    app.dialogs().unwrap().with(|h| h.cancel_by_id(&ctx, replace));
    pass(&mut app, &ctx);
    assert!(!asking(&app));
    assert_eq!(app.session.trace_preset("Poster").unwrap().params.noise, 7);
    // Replace acts on the contents the question was asked about, though the file changed since.
    open_path(&mut app, &file).unwrap();
    let changed =
        r#"{"format": "vctracepresets", "version": 1, "presets": [{"name": "Poster", "params": {"noise": 3}}, {"name": "Extra", "params": {}}]}"#;
    std::fs::write(&file, changed).unwrap();
    pass(&mut app, &ctx);
    app.dialogs().unwrap().with(|h| {
        h.set_json_field(&ctx, replace, "choice", json!("proceed")).unwrap();
        h.confirm_by_id(&ctx, replace)
    });
    pass(&mut app, &ctx);
    std::fs::write(&file, &text).unwrap();
    assert_eq!(selected(&app).as_deref(), Some("Poster"));
    assert_eq!(app.session.trace_preset("Poster").unwrap().params.noise, 40);
    assert_eq!(saved_names(&app), ["Poster", "Ink"]);
    // Opening the file with no manager open asks the same question, over nothing.
    app.dialogs().unwrap().with(|h| h.cancel::<Manager>(&ctx));
    app.run("imageTrace.presets.save", json!({"name": "Poster", "params": {"noise": 7}})).unwrap();
    let bytes = std::fs::read(&file).unwrap();
    crate::io::open_bytes(&mut app, &file, &bytes, Some(file.clone())).unwrap();
    assert!(app.ui.status.contains("tracing presets"), "{}", app.ui.status);
    pass(&mut app, &ctx);
    assert!(asking(&app));
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
    let (mut app, ctx) = windowed(crate::Services { read_head: Some(Box::new(read_head)), ..Default::default() });
    app.run("ui.tracingPresetsDialog", json!({})).unwrap();
    pass(&mut app, &ctx);
    let e = open_path(&mut app, &file.to_string_lossy()).unwrap_err();
    assert!(e.contains("built-in preset"), "{e}");
    pass(&mut app, &ctx);
    assert!(app.dialogs().is_some_and(|d| d.with(|h| h.is_open::<Manager>() && !h.is_open::<ReplacePresets>())), "no question");
    assert!(app.session.prefs.trace_presets.is_empty());
    // An opened file over the size limit is refused before its bytes are encoded for the import.
    let big = vec![b' '; TRACE_PRESET_MAX_BYTES + 1];
    let e = open_file(&mut app, &big).unwrap_err();
    assert!(e.contains("tracing presets file is larger than 4 MiB"), "{e}");
    let big_file = dir.join("big.vctracepresets");
    std::fs::write(&big_file, &big).unwrap();
    let e = open_path(&mut app, &big_file.to_string_lossy()).unwrap_err();
    assert!(e.contains("tracing presets file is larger than 4 MiB"), "{e}");
    assert!(app.session.prefs.trace_presets.is_empty());
    let _ = std::fs::remove_dir_all(dir);
}

/// File › Open reads a tracing presets file no further than one byte past the size limit, and
/// refuses a larger one, without the whole-file reader.
#[test]
fn an_opened_file_is_read_no_further_than_the_limit() {
    let dir = std::env::temp_dir().join(format!("vc-tracing-presets-open-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let fine = dir.join("fine.vctracepresets");
    std::fs::write(&fine, r#"{"format": "vctracepresets", "version": 1, "presets": [{"name": "Fine", "params": {}}]}"#).unwrap();
    let big = dir.join("big.vctracepresets");
    std::fs::write(&big, vec![b' '; TRACE_PRESET_MAX_BYTES + 1]).unwrap();
    let read_head = |p: &str, max: usize| {
        use std::io::Read as _;
        assert_eq!(max, TRACE_PRESET_MAX_BYTES + 1);
        let mut bytes = vec![];
        std::fs::File::open(p).and_then(|f| f.take(max as u64).read_to_end(&mut bytes)).map_err(|e| e.to_string())?;
        Ok(bytes)
    };
    let read = |p: &str| -> Result<Vec<u8>, String> { Err(format!("`{p}` was read in full")) };
    let services = crate::Services { read_head: Some(Box::new(read_head)), read: Some(Box::new(read)), ..Default::default() };
    let mut app = VectorcraftApp::new(Session::new(), services);
    crate::io::open_path(&mut app, &fine.to_string_lossy()).unwrap();
    assert_eq!(saved_names(&app), ["Fine"]);
    let e = crate::io::open_path(&mut app, &big.to_string_lossy()).unwrap_err();
    assert!(e.contains("tracing presets file is larger than 4 MiB"), "{e}");
    assert_eq!(saved_names(&app), ["Fine"]);
    let _ = std::fs::remove_dir_all(dir);
}
