use std::cell::RefCell;
use std::rc::Rc;

use vectorcraft_engine::Session;

use super::*;
use crate::Services;

type Log = Rc<RefCell<Vec<(String, Vec<u8>)>>>;

/// The built-in presets are told from saved ones by name (only theirs are translated).
#[test]
fn built_in_presets_are_known_by_name() {
    assert!(vectorcraft_pdf::builtin_presets().iter().all(|p| is_builtin_preset(&p.name)));
    assert!(is_builtin_preset(pdf::DEFAULT_PRESET) && !is_builtin_preset("Default") && !is_builtin_preset("My Preset"));
}

/// An app with two artboards whose writer and URL opener record what they get.
fn app() -> (VectorcraftApp, Log, Log) {
    let (written, opened) = (Log::default(), Log::default());
    let (w, o) = (written.clone(), opened.clone());
    let services = Services {
        write: Some(Box::new(move |p: &str, b: &[u8]| {
            w.borrow_mut().push((p.to_string(), b.to_vec()));
            Ok(())
        })),
        open_url: Some(Box::new(move |u: &str| o.borrow_mut().push((u.to_string(), vec![])))),
        ..Default::default()
    };
    let mut app = VectorcraftApp::new(Session::new(), services);
    app.run("file.new", json!({"width": 120, "height": 90, "artboards": 3})).unwrap();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 40})).unwrap();
    (app, written, opened)
}

/// One headless frame of the dialog layer.
fn frame(app: &mut VectorcraftApp) {
    let ctx = egui::Context::default();
    theme::install_fonts(&ctx);
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| super::super::show(app, ui.ctx()));
    out.textures_delta.clear();
}

fn set_field(app: &mut VectorcraftApp, path: &str, v: Value) {
    set(app.ui.dialog.as_mut().expect("dialog open"), path, v);
}

#[test]
fn every_section_draws() {
    let (mut app, _, _) = app();
    app.run("file.export.pdf", json!({})).unwrap();
    assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some(KIND));
    assert_eq!(super::super::DialogKind::of(KIND), Some(super::super::DialogKind::SavePdf));
    for s in SECTIONS {
        set_field(&mut app, "__section", json!(s));
        frame(&mut app);
        assert!(app.ui.dialog.is_some(), "{s} closed the dialog");
    }
    // The Summary lists what changed and the warnings for options not applied yet (permissions
    // without a password; thumbnails are embedded).
    set_field(&mut app, "thumbnails", json!(true));
    set_field(&mut app, "security.printing", json!("low"));
    let p = params(app.ui.dialog.as_ref().unwrap());
    let v = summary_of(&mut app, &egui::Context::default(), &p).unwrap();
    let changed: Vec<&str> = v["changed"].as_array().unwrap().iter().map(|c| c["option"].as_str().unwrap()).collect();
    assert_eq!(changed, ["security.printing", "thumbnails"]);
    let warnings = v["warnings"].as_array().unwrap();
    assert!(warnings.len() == 1 && warnings[0].as_str().unwrap().contains("permissions"), "{warnings:?}");
}

/// `ui.pdfSummary.save` writes the Summary as the section shows it, as UTF-8 text with LF line
/// endings: Options with each changed option in the order of the sections, a blank line, then
/// Warnings. The dialog stays open and the document does not change.
#[test]
fn save_summary_writes_the_summary_as_text() {
    let (mut app, written, _) = app();
    app.run("file.export.pdf", json!({})).unwrap();
    set_field(&mut app, "thumbnails", json!(true));
    set_field(&mut app, "security.printing", json!("low"));
    let revision = app.session.active().unwrap().revision;
    let r = app.run(SAVE_SUMMARY, json!({"path": "/tmp/Ünïcode (1) summary.txt"})).unwrap();
    assert_eq!(r["path"], "/tmp/Ünïcode (1) summary.txt", "the path is used as given");
    let w = written.borrow();
    assert_eq!(w.len(), 1);
    assert_eq!(w[0].0, "/tmp/Ünïcode (1) summary.txt");
    assert_eq!(r["bytes"], w[0].1.len());
    let text = String::from_utf8(w[0].1.clone()).expect("UTF-8");
    assert!(!text.contains('\r') && text.ends_with('\n'), "LF line endings: {text:?}");
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines[..5], ["Options", "General › Thumbnails: On", "Security › Printing: low", "", "Warnings"], "{text}");
    assert!(lines.len() == 6 && lines[5].starts_with("⚠ ") && lines[5].contains("permissions"), "{text}");
    assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some(KIND), "the dialog stays open");
    assert_eq!(app.session.active().unwrap().revision, revision, "the document does not change");
    assert_eq!(app.ui.status, "Saved /tmp/Ünïcode (1) summary.txt");
}

/// A text option whose value holds CRLF or CR line breaks is written with LF line breaks, so the
/// file has one kind of line ending.
#[test]
fn save_summary_writes_line_breaks_in_values_as_lf() {
    let (mut app, written, _) = app();
    app.run("file.export.pdf", json!({})).unwrap();
    set_field(&mut app, "output.outputCondition", json!("first\r\nsecond\rthird"));
    app.run(SAVE_SUMMARY, json!({"path": "/tmp/summary.txt"})).unwrap();
    let text = String::from_utf8(written.borrow()[0].1.clone()).unwrap();
    assert!(!text.contains('\r'), "LF line endings: {text:?}");
    assert!(text.contains("first\nsecond\nthird\n"), "{text:?}");
}

/// With every option as the preset has it and nothing to warn about, the file holds the lines
/// that say so, in the interface language.
#[test]
fn save_summary_writes_the_lines_for_no_changes_and_no_warnings() {
    let (mut app, written, _) = app();
    app.run("file.export.pdf", json!({})).unwrap();
    app.run(SAVE_SUMMARY, json!({"path": "/tmp/summary.txt"})).unwrap();
    let text = String::from_utf8(written.borrow()[0].1.clone()).unwrap();
    assert_eq!(text, "Options\nEvery option matches the preset.\n\nWarnings\nNone.\n");
    crate::i18n::set_current(crate::i18n::Lang::from_code("de").expect("de registered"));
    let r = app.run(SAVE_SUMMARY, json!({"path": "/tmp/zusammenfassung.txt"}));
    crate::i18n::set_current(crate::i18n::Lang::EN);
    r.unwrap();
    let de = String::from_utf8(written.borrow()[1].1.clone()).unwrap();
    let de_lang = crate::i18n::Lang::from_code("de").unwrap();
    let expected = format!(
        "{}\n{}\n\n{}\n{}\n",
        crate::i18n::tr(de_lang, "Options"),
        crate::i18n::fmt(crate::i18n::tr(de_lang, "Every option matches {base}."), &[("base", crate::i18n::tr(de_lang, "the preset"))]),
        crate::i18n::tr(de_lang, "Warnings"),
        crate::i18n::tr(de_lang, "None."),
    );
    assert_eq!(de, expected);
    assert_ne!(de, text, "the labels are translated");
}

/// The preset editor's Summary is saved too, measured against the app's default preset, and the
/// editor stays open.
#[test]
fn save_summary_writes_the_preset_editors_summary() {
    let (mut app, written, _) = app();
    open_preset(&mut app, &json!({})).unwrap();
    set_field(&mut app, "thumbnails", json!(true));
    app.run(SAVE_SUMMARY, json!({"path": "/tmp/preset summary.txt"})).unwrap();
    let text = String::from_utf8(written.borrow()[0].1.clone()).unwrap();
    assert_eq!(text, "Options\nGeneral › Thumbnails: On\n\nWarnings\nNone.\n");
    set_field(&mut app, "thumbnails", json!(false));
    app.run(SAVE_SUMMARY, json!({"path": "/tmp/preset summary.txt"})).unwrap();
    let text = String::from_utf8(written.borrow()[1].1.clone()).unwrap();
    assert_eq!(text, format!("Options\nEvery option matches {}.\n\nWarnings\nNone.\n", pdf::DEFAULT_PRESET));
    assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some(PRESET_KIND), "the editor stays open");
}

/// A path that is empty or not a string, and no Save PDF dialog open, are errors that write
/// nothing.
#[test]
fn save_summary_refuses_a_bad_path_and_a_closed_dialog() {
    let (mut app, written, _) = app();
    assert_eq!(app.run(SAVE_SUMMARY, json!({"path": "/tmp/s.txt"})).unwrap_err(), "the Save PDF dialog is not open");
    app.run("file.export.pdf", json!({})).unwrap();
    assert_eq!(app.run(SAVE_SUMMARY, json!({"path": ""})).unwrap_err(), "path is empty");
    assert_eq!(app.run(SAVE_SUMMARY, json!({"path": "  "})).unwrap_err(), "path is empty");
    assert_eq!(app.run(SAVE_SUMMARY, json!({"path": 5})).unwrap_err(), "`path` must be a string");
    assert!(written.borrow().is_empty());
    assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some(KIND));
}

/// The command needs a path: omitted or null, it is an error that shows no save panel and writes
/// nothing.
#[test]
fn save_summary_requires_a_path() {
    let (mut app, written, _) = app();
    let asked: Rc<RefCell<usize>> = Rc::default();
    let a = asked.clone();
    app.services.pick_save = Some(Box::new(move |pick: &crate::FilePick| {
        *a.borrow_mut() += 1;
        Some(format!("/tmp/{}", pick.name))
    }));
    app.run("file.export.pdf", json!({})).unwrap();
    assert_eq!(app.run(SAVE_SUMMARY, json!({})).unwrap_err(), "missing `path`");
    assert_eq!(app.run(SAVE_SUMMARY, json!({"path": null})).unwrap_err(), "missing `path`");
    assert_eq!(*asked.borrow(), 0, "no save panel");
    assert!(written.borrow().is_empty());
    assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some(KIND));
}

/// The command uses the Summary the section drew for the same fields and document, without
/// computing the settings again (an export in memory), and computes them once a field changes.
#[test]
fn save_summary_uses_the_summary_the_section_drew() {
    let (mut app, written, _) = app();
    app.run("file.export.pdf", json!({})).unwrap();
    set_field(&mut app, "__section", json!("Summary"));
    let ctx = egui::Context::default();
    theme::install_fonts(&ctx);
    app.ui_ctx = Some(ctx.clone());
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| super::super::show(&mut app, ui.ctx()));
    out.textures_delta.clear();
    // The drawn Summary, replaced with one that no computation gives.
    let (key, stamp) = summary_key(&app, &params(app.ui.dialog.as_ref().unwrap()));
    let drawn = ctx.data(|m| m.get_temp::<(egui::Id, Result<Value, String>)>(key)).expect("the section's Summary");
    assert_eq!(drawn.0, stamp, "drawn for these fields");
    let kept: Result<Value, String> = Ok(json!({"changed": [{"option": "thumbnails", "value": "kept"}], "warnings": []}));
    ctx.data_mut(|m| m.insert_temp(key, (stamp, kept)));
    app.run(SAVE_SUMMARY, json!({"path": "/tmp/a.txt"})).unwrap();
    let text = String::from_utf8(written.borrow()[0].1.clone()).unwrap();
    assert_eq!(text, "Options\nGeneral › Thumbnails: kept\n\nWarnings\nNone.\n");
    set_field(&mut app, "thumbnails", json!(true));
    app.run(SAVE_SUMMARY, json!({"path": "/tmp/b.txt"})).unwrap();
    let text = String::from_utf8(written.borrow()[1].1.clone()).unwrap();
    assert!(text.starts_with("Options\nGeneral › Thumbnails: On\n"), "computed for the changed field: {text}");
}

/// The Summary drawn for one document is not used for another that becomes active with the same
/// revision and the same fields.
#[test]
fn save_summary_does_not_use_another_documents_summary() {
    let (mut app, written, _) = app();
    let first = app.session.active_index().unwrap();
    let revision = app.session.active().unwrap().revision;
    app.run("file.new", json!({"width": 120, "height": 90})).unwrap();
    let second = app.session.active_index().unwrap();
    app.session.set_active(first);
    app.run("file.export.pdf", json!({})).unwrap();
    let ctx = egui::Context::default();
    app.ui_ctx = Some(ctx.clone());
    let (key, stamp) = summary_key(&app, &params(app.ui.dialog.as_ref().unwrap()));
    let kept: Result<Value, String> = Ok(json!({"changed": [{"option": "thumbnails", "value": "kept"}], "warnings": []}));
    ctx.data_mut(|m| m.insert_temp(key, (stamp, kept)));
    app.session.set_active(second);
    app.session.active_mut().unwrap().revision = revision;
    app.run(SAVE_SUMMARY, json!({"path": "/tmp/a.txt"})).unwrap();
    let text = String::from_utf8(written.borrow()[0].1.clone()).unwrap();
    assert!(!text.contains("kept"), "the other document's Summary: {text}");
}

/// The Summary is computed again when the saved preset it compares against is replaced while the
/// dialog stays open.
#[test]
fn save_summary_compares_against_a_replaced_preset() {
    let (mut app, written, _) = app();
    app.run("pdf.preset.save", json!({"name": "Mine", "thumbnails": true})).unwrap();
    app.run("ui.savePdfDialog", json!({"preset": "Mine"})).unwrap();
    set_field(&mut app, "thumbnails", json!(false));
    app.ui_ctx = Some(egui::Context::default());
    app.run(SAVE_SUMMARY, json!({"path": "/tmp/a.txt"})).unwrap();
    let text = String::from_utf8(written.borrow()[0].1.clone()).unwrap();
    assert!(text.starts_with("Options\nGeneral › Thumbnails: Off\n"), "{text}");
    app.run("pdf.preset.save", json!({"name": "Mine", "thumbnails": false})).unwrap();
    app.run(SAVE_SUMMARY, json!({"path": "/tmp/b.txt"})).unwrap();
    let text = String::from_utf8(written.borrow()[1].1.clone()).unwrap();
    assert!(text.starts_with("Options\nEvery option matches the preset.\n"), "compared against the replaced preset: {text}");
}

/// The Summary is computed again when the saved flattener preset the dialog names is deleted while
/// the dialog stays open: the settings are then refused, and nothing is written.
#[test]
fn save_summary_sees_a_deleted_flattener_preset() {
    let (mut app, written, _) = app();
    app.run("flattener.presets.save", json!({"name": "Mine Flat", "preset": "Low Resolution"})).unwrap();
    app.run("file.export.pdf", json!({})).unwrap();
    set_field(&mut app, "flattenerPreset", json!("Mine Flat"));
    app.ui_ctx = Some(egui::Context::default());
    app.run(SAVE_SUMMARY, json!({"path": "/tmp/a.txt"})).unwrap();
    app.run("flattener.presets.delete", json!({"name": "Mine Flat"})).unwrap();
    let e = app.run(SAVE_SUMMARY, json!({"path": "/tmp/b.txt"})).unwrap_err();
    assert!(e.contains("Mine Flat"), "the deleted preset is refused: {e}");
    assert_eq!(written.borrow().len(), 1, "nothing written for the deleted preset");
}

/// The Summary is computed again when a loaded profile the dialog converts to is removed while the
/// dialog stays open: the settings are then refused, and nothing is written.
#[test]
fn save_summary_sees_a_removed_profile() {
    use vectorcraft_color::cms;
    let name = "Summary Stamp CMYK";
    // The built-in CMYK profile's file renamed `name`: padded with spaces, which names drop.
    let utf16 = |s: &str| s.encode_utf16().flat_map(u16::to_be_bytes).collect::<Vec<u8>>();
    let mut bytes = cms::icc_bytes(cms::DEVICE_CMYK).unwrap().to_vec();
    let (from, to) = (utf16(cms::DEVICE_CMYK), utf16(&format!("{name:<width$}", width = cms::DEVICE_CMYK.len())));
    let at = bytes.windows(from.len()).position(|w| w == from).unwrap();
    bytes[at..at + to.len()].copy_from_slice(&to);
    let path = vectorcraft_testkit::temp_dir("save-summary-profile").join("stamp.icc");
    std::fs::write(&path, bytes).unwrap();
    let (mut app, written, _) = app();
    assert_eq!(app.run("color.loadProfile", json!({"path": path.to_str().unwrap()})).unwrap()["name"], name);
    app.run("file.export.pdf", json!({})).unwrap();
    set_field(&mut app, "output.conversion", json!("destination"));
    set_field(&mut app, "output.destination", json!(name));
    app.ui_ctx = Some(egui::Context::default());
    app.run(SAVE_SUMMARY, json!({"path": "/tmp/a.txt"})).unwrap();
    app.run("color.removeProfile", json!({"name": name})).unwrap();
    let e = app.run(SAVE_SUMMARY, json!({"path": "/tmp/b.txt"})).unwrap_err();
    assert!(e.contains(name), "the removed profile is refused: {e}");
    assert_eq!(written.borrow().len(), 1, "nothing written for the removed profile");
}

/// With the desktop's file writer: a file already at the path is replaced whole, a folder's path
/// is an error that leaves the folder as it was, and under automation write roots a path outside
/// them is refused without writing.
#[test]
fn save_summary_writes_files_on_disk_within_the_automation_roots() {
    let base = std::env::temp_dir().join(format!("vc-pdf-summary-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    for d in ["out", "outside"] {
        std::fs::create_dir_all(base.join(d)).unwrap();
    }
    let roots = vectorcraft_engine::file_access::AutomationRoots::new(&[], &[&base.join("out")]).unwrap();
    let services = Services {
        write: Some(Box::new(|p: &str, b: &[u8]| vectorcraft_format::write_atomic(std::path::Path::new(p), b).map_err(|e| e.to_string()))),
        ..Default::default()
    };
    let mut app = VectorcraftApp::new(Session::new(), services).with_automation_roots(roots.clone());
    app.run("file.new", json!({"width": 120, "height": 90})).unwrap();
    app.run("file.export.pdf", json!({})).unwrap();
    let mut save = |path: &std::path::Path| {
        let p = json!({"path": path.to_string_lossy()});
        vectorcraft_engine::file_access::confine(roots.as_ref(), || app.run(SAVE_SUMMARY, p))
    };
    let file = base.join("out").join("Ünïcode (1), summary.txt");
    std::fs::write(&file, "an older file, longer than the summary\n".repeat(40)).unwrap();
    save(&file).unwrap();
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "Options\nEvery option matches the preset.\n\nWarnings\nNone.\n");
    assert!(save(&base.join("out")).is_err(), "a folder is not a file");
    assert!(base.join("out").is_dir() && file.is_file());
    let outside = base.join("outside").join("summary.txt");
    assert!(save(&outside).is_err(), "outside the write roots");
    assert!(!outside.exists());
    let _ = std::fs::remove_dir_all(&base);
}

/// A failed `document.pdfSettings` is the command's error, and nothing is written.
#[test]
fn save_summary_reports_a_failed_summary() {
    let (mut app, written, _) = app();
    app.run("file.export.pdf", json!({})).unwrap();
    set_field(&mut app, "compatibility", json!("bogus"));
    let p = params(app.ui.dialog.as_ref().unwrap());
    let expected = summary_settings(&mut app, &p).unwrap_err();
    assert_eq!(app.run(SAVE_SUMMARY, json!({"path": "/tmp/s.txt"})).unwrap_err(), expected);
    assert!(written.borrow().is_empty());
}

/// The Summary section's Save Summary… button: the save panel suggests `<document> PDF
/// Summary.txt`, the file holds the text the section shows, and the dialog stays open. A canceled
/// panel writes nothing; a failed write is told over the dialog, which shows again when the
/// message closes. The web downloads the file under the suggested name.
#[test]
fn the_save_summary_button_writes_the_summary() {
    let (mut app, written, _) = app();
    let suggested: Rc<RefCell<Vec<String>>> = Rc::default();
    let s = suggested.clone();
    app.services.pick_save = Some(Box::new(move |pick: &crate::FilePick| {
        s.borrow_mut().push(pick.name.clone());
        Some(format!("/tmp/{}", pick.name))
    }));
    app.run("file.export.pdf", json!({})).unwrap();
    set_field(&mut app, "__section", json!("Summary"));
    set_field(&mut app, "thumbnails", json!(true));
    let ctx = egui::Context::default();
    theme::install_fonts(&ctx);
    click_save_summary(&mut app, &ctx);
    let title = app.session.active().unwrap().title();
    let stem = std::path::Path::new(&title).file_stem().unwrap().to_string_lossy().into_owned();
    let name = format!("{stem} PDF Summary.txt");
    assert_eq!(suggested.borrow()[..], *std::slice::from_ref(&name));
    {
        let w = written.borrow();
        assert_eq!(w.len(), 1, "written once");
        assert_eq!(w[0].0, format!("/tmp/{name}"));
        let text = String::from_utf8(w[0].1.clone()).unwrap();
        assert!(text.starts_with("Options\nGeneral › Thumbnails: On\n\nWarnings\n"), "{text}");
    }
    assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some(KIND), "the dialog stays open");
    // Canceled: nothing written, no status, the dialog stays.
    app.services.pick_save = Some(Box::new(|_: &crate::FilePick| None));
    app.ui.status.clear();
    click_save_summary(&mut app, &ctx);
    assert_eq!(written.borrow().len(), 1, "a canceled panel writes nothing");
    assert!(app.ui.status.is_empty(), "{}", app.ui.status);
    assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some(KIND));
    // A failed write: a message names the file, over the dialog.
    app.services.pick_save = Some(Box::new(|pick: &crate::FilePick| Some(format!("/tmp/{}", pick.name))));
    app.services.write = Some(Box::new(|_: &str, _: &[u8]| Err("disk full".into())));
    click_save_summary(&mut app, &ctx);
    let told = app.ui.dialog.clone().expect("a message");
    assert_eq!(told.kind, super::super::confirm::MESSAGE, "status {:?}, waiting {:?}", app.ui.status, app.ui.told);
    assert!(told.str("message").contains(&name), "{:?}", told.fields);
    assert_eq!(super::super::parent(&told).map(|d| d.kind), Some(KIND.to_string()), "the dialog is under the message");
    super::super::cancel(&mut app);
    assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some(KIND), "the dialog shows again");
    // The web downloads it under the suggested name.
    let downloads: Log = Log::default();
    let dl = downloads.clone();
    app.services.download = Some(Box::new(move |n: &str, b: &[u8]| {
        dl.borrow_mut().push((n.to_string(), b.to_vec()));
        Ok(())
    }));
    click_save_summary(&mut app, &ctx);
    let d = downloads.borrow();
    assert_eq!(d.len(), 1);
    assert_eq!(d[0].0, name);
    assert!(String::from_utf8(d[0].1.clone()).unwrap().starts_with("Options\n"));
}

/// The saved Summary writes each option line with the colon the interface language uses, as the
/// Summary section shows it: French puts a no-break space before the colon, Japanese writes a
/// full-width colon.
#[test]
fn the_summary_text_uses_the_languages_colon() {
    let changed = json!({"option": "thumbnails", "value": true});
    for (code, colon) in [("en", ":"), ("fr", "\u{a0}:"), ("ja", "：")] {
        crate::i18n::set_current(crate::i18n::Lang::from_code(code).expect("registered"));
        let line = option_line(&changed, |_| "X".to_string());
        crate::i18n::set_current(crate::i18n::Lang::EN);
        assert!(line.starts_with(&format!("X{colon} ")), "{code}: {line:?}");
    }
}

/// Click the Summary section's Save Summary… button in frames of the dialog layer (the dialog's
/// window draws nothing in its first frame, which sizes it).
fn click_save_summary(app: &mut VectorcraftApp, ctx: &egui::Context) {
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 1000.0));
    let run = |app: &mut VectorcraftApp, events: Vec<egui::Event>| {
        let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), events, ..Default::default() }, |ui| super::super::show(app, ui.ctx()));
        out.textures_delta.clear();
        crate::tests_removeanchors::shapes_text(&out.shapes.iter().map(|c| c.shape.clone()).collect::<Vec<_>>())
    };
    run(app, vec![]);
    let labels = run(app, vec![]);
    let at =
        labels.iter().find(|(t, _)| t.trim() == "Save Summary…").map(|(_, r)| r.center()).unwrap_or_else(|| panic!("no Save Summary… in {labels:?}"));
    let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
    run(app, vec![egui::Event::PointerMoved(at), button(true)]);
    run(app, vec![button(false)]);
}

#[test]
fn confirm_runs_export_pdf_with_the_dialog_options() {
    let (mut app, written, opened) = app();
    app.run("ui.savePdfDialog", json!({"path": "/tmp/out.pdf", "compatibility": "1.5"})).unwrap();
    set_field(&mut app, "__allArtboards", json!(false));
    set_field(&mut app, "range", json!("1,3"));
    set_field(&mut app, "compression.compressText", json!(false));
    set_field(&mut app, "viewAfterSaving", json!(true));
    let r = super::super::confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none(), "closes after saving");
    assert_eq!(r["path"], "/tmp/out.pdf");
    assert!(r.get("dataBase64").is_none());
    let w = written.borrow();
    assert_eq!(w.len(), 1);
    assert_eq!(w[0].0, "/tmp/out.pdf");
    assert!(w[0].1.starts_with(b"%PDF-1.5"), "compatibility reaches the writer");
    assert_eq!(vectorcraft_pdf::import(&w[0].1).unwrap().artboards.len(), 2, "range 1,3");
    assert!(!String::from_utf8_lossy(&w[0].1).contains("/FlateDecode"), "uncompressed content");
    assert_eq!(opened.borrow().len(), 1, "View PDF after Saving opens the file once");
    let url = &opened.borrow()[0].0;
    assert!(url.starts_with("file:///") && url.ends_with("/tmp/out.pdf"), "the written file as a URL: {url}");
    assert!(app.ui.status.starts_with("Saved /tmp/out.pdf"), "{}", app.ui.status);
}

/// A Save PDF that fails keeps the dialog open and says why inside it (#6861): the status bar is
/// behind the dialog. Saving again, once it works, clears the message.
#[test]
fn a_failed_save_says_why_in_the_dialog() {
    let (mut app, written, _) = app();
    app.run("ui.savePdfDialog", json!({"path": "/tmp/out.pdf"})).unwrap();
    set_field(&mut app, "__allArtboards", json!(false));
    set_field(&mut app, "range", json!("9"));
    let e = super::super::confirm(&mut app).unwrap_err();
    assert!(written.borrow().is_empty(), "nothing was written");
    let d = app.ui.dialog.as_ref().expect("the dialog stays open");
    assert_eq!(d.str(ERROR), e, "the dialog holds the reason");
    assert!(!e.is_empty());
    assert!(params(d).get(ERROR).is_none(), "the message is not an option of the export");
    frame(&mut app);
    set_field(&mut app, "range", json!("1"));
    super::super::confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none());
    assert_eq!(written.borrow().len(), 1);
}

/// Opened for a page of the art's bounds (Export As with Use Artboards off), the dialog has no
/// Artboards section, and OK writes one page of the art's bounds with its options, a saved preset
/// and View PDF after Saving included.
#[test]
fn a_page_of_the_arts_bounds_has_no_artboards_section_and_takes_the_options() {
    let (mut app, written, opened) = app();
    app.run("pdf.preset.save", json!({"name": "Mine", "compatibility": "1.5"})).unwrap();
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    theme::install_fonts(&ctx);
    let has_range = |app: &mut VectorcraftApp| {
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| super::super::show(app, ui.ctx()));
        out.textures_delta.clear();
        let nodes = &out.platform_output.accesskit_update.as_ref().expect("an accessibility tree").nodes;
        nodes.iter().any(|(_, n)| n.label() == Some("Range:") && n.role() == egui::accesskit::Role::RadioButton)
    };
    app.run("ui.savePdfDialog", json!({"path": "/tmp/artboards.pdf"})).unwrap();
    has_range(&mut app);
    assert!(has_range(&mut app), "the artboards' dialog has the section");
    app.run("ui.savePdfDialog", json!({"path": "/tmp/art.pdf", "preset": "Mine", "useArtboards": false})).unwrap();
    assert!(app.ui.dialog.as_ref().unwrap().bool(ART_BOUNDS));
    has_range(&mut app);
    assert!(!has_range(&mut app), "no artboards to choose");
    set_field(&mut app, "viewAfterSaving", json!(true));
    let r = super::super::confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none(), "closes after saving");
    assert_eq!(r["path"], "/tmp/art.pdf");
    let w = written.borrow();
    assert_eq!(w.len(), 1);
    assert!(w[0].1.starts_with(b"%PDF-1.5"), "the saved preset reaches the writer");
    let pdf = vectorcraft_pdf::import(&w[0].1).unwrap();
    assert_eq!(pdf.artboards.len(), 1);
    let size = pdf.artboards[0].rect.size();
    assert!((50.0..52.0).contains(&size.width) && (40.0..42.0).contains(&size.height), "the rectangle's bounds with its stroke: {size:?}");
    assert_eq!(opened.borrow().len(), 1, "View PDF after Saving opens the file");
    assert!(opened.borrow()[0].0.ends_with("/tmp/art.pdf"));
}

/// `useArtboards` other than true, false or null is refused, as `document.exportPdf` refuses it,
/// and no dialog opens.
#[test]
fn a_use_artboards_that_is_not_true_or_false_is_refused() {
    let (mut app, _, _) = app();
    for bad in [json!("false"), json!(0), json!({})] {
        let e = app.run("ui.savePdfDialog", json!({"path": "/tmp/x.pdf", "useArtboards": bad})).unwrap_err();
        assert!(e.contains("useArtboards must be true or false"), "{bad}: {e}");
        assert!(app.ui.dialog.is_none(), "{bad}: no dialog opens");
    }
    for ok in [json!(true), json!(null)] {
        app.run("ui.savePdfDialog", json!({"path": "/tmp/x.pdf", "useArtboards": ok})).unwrap();
        assert!(!app.ui.dialog.take().unwrap().bool(ART_BOUNDS), "{ok}: the artboards' dialog");
    }
}

/// With Background Export on, a page of the art's bounds is written in the background, as Export
/// As writes its other formats.
#[test]
fn a_page_of_the_arts_bounds_is_written_in_the_background() {
    let written = std::sync::Arc::new(std::sync::Mutex::new(Vec::<(String, Vec<u8>)>::new()));
    let w = written.clone();
    let services = Services {
        write_shared: Some(std::sync::Arc::new(move |p: &str, b: &[u8]| {
            w.lock().unwrap().push((p.to_string(), b.to_vec()));
            Ok(())
        })),
        ..Default::default()
    };
    let mut app = VectorcraftApp::new(Session::new(), services);
    app.session.prefs.background_export = true;
    app.run("file.new", json!({"width": 120, "height": 90})).unwrap();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 40})).unwrap();
    app.run("ui.savePdfDialog", json!({"path": "/tmp/bg.pdf", "useArtboards": false})).unwrap();
    let r = super::super::confirm(&mut app).unwrap();
    assert_eq!(r["background"], true, "{r}");
    assert!(app.ui.dialog.is_none());
    crate::background::wait_all(&mut app);
    let w = written.lock().unwrap();
    assert_eq!(w.len(), 1);
    assert_eq!(w[0].0, "/tmp/bg.pdf");
    assert_eq!(vectorcraft_pdf::import(&w[0].1).unwrap().artboards.len(), 1);
}

/// A document without a title of its own, saved to a file, is titled after that file in the PDF,
/// and viewers show the file name, with or without Use Artboards; one with its own title shows
/// that title.
#[test]
fn a_page_of_the_arts_bounds_shows_the_title_only_when_it_is_the_documents_own() {
    let (mut app, written, _) = app();
    app.session.doc_mut().unwrap().path = Some("/docs/Poster.vectorcraft".into());
    for artboards in [true, false] {
        app.run("ui.savePdfDialog", json!({"path": "/tmp/t.pdf", "useArtboards": artboards, "compression": {"compressText": false}})).unwrap();
        super::super::confirm(&mut app).unwrap();
    }
    std::sync::Arc::make_mut(&mut app.session.doc_mut().unwrap().doc).title = "Spring Sale".into();
    app.run("ui.savePdfDialog", json!({"path": "/tmp/t.pdf", "useArtboards": false, "compression": {"compressText": false}})).unwrap();
    super::super::confirm(&mut app).unwrap();
    let w = written.borrow();
    assert_eq!(w.len(), 3);
    let texts: Vec<String> = w.iter().map(|(_, b)| String::from_utf8_lossy(b).into_owned()).collect();
    for (pdf, artboards) in texts[..2].iter().zip([true, false]) {
        assert!(pdf.contains("/Title(Poster)"), "useArtboards {artboards}");
        assert!(!pdf.contains("/DisplayDocTitle true"), "useArtboards {artboards}");
    }
    assert!(texts[2].contains("/Title(Spring Sale)") && texts[2].contains("/DisplayDocTitle true"));
}

#[test]
fn warnings_reach_the_status_and_errors_keep_the_dialog_open() {
    let (mut app, written, opened) = app();
    app.run("ui.savePdfDialog", json!({"path": "/tmp/w.pdf"})).unwrap();
    set_field(&mut app, "security.printing", json!("low"));
    super::super::confirm(&mut app).unwrap();
    assert!(app.ui.status.contains("1 note:") && app.ui.status.contains("permissions"), "{}", app.ui.status);
    assert!(opened.borrow().is_empty(), "not opened unless asked");
    app.run("ui.savePdfDialog", json!({"path": "/tmp/bad.pdf"})).unwrap();
    set_field(&mut app, "__allArtboards", json!(false));
    set_field(&mut app, "range", json!("7"));
    assert!(super::super::confirm(&mut app).is_err());
    assert!(app.ui.dialog.is_some(), "a bad range keeps the dialog open");
    assert_eq!(written.borrow().len(), 1);
    // Without a path, bad options are refused before a save dialog asks for one.
    let asked = Rc::new(RefCell::new(0));
    let a = asked.clone();
    app.services.pick_save = Some(Box::new(move |pick: &crate::FilePick| {
        *a.borrow_mut() += 1;
        Some(format!("/tmp/picked-{}", pick.name))
    }));
    app.run("ui.savePdfDialog", json!({"range": "7"})).unwrap();
    assert!(super::super::confirm(&mut app).is_err());
    assert_eq!(*asked.borrow(), 0, "no save dialog for a bad range");
    set_field(&mut app, "range", json!("2"));
    let r = super::super::confirm(&mut app).unwrap();
    assert_eq!(*asked.borrow(), 1);
    assert!(r["path"].as_str().is_some_and(|p| p.starts_with("/tmp/picked-") && p.ends_with(".pdf")), "{r}");
    // Agents can export without the dialog.
    let r = app.run("file.export.pdf", json!({"path": "/tmp/direct.pdf", "range": "2"})).unwrap();
    assert_eq!(r["path"], "/tmp/direct.pdf");
    assert!(r["warnings"].is_array());
}

/// The Summary's warnings are translated as far as the catalog has them: a warning without a row
/// keeps its words, and a colon in it can still take the catalog's punctuation.
#[test]
fn summary_warnings_are_translated() {
    let fr = crate::i18n::Lang::from_code("fr").unwrap();
    let shown = warning_line(fr, vectorcraft_engine::cmd::fileio::pdf::BLEED_LEFT_OUT);
    assert!(shown.starts_with("⚠ ") && !shown.contains("leaves out"), "{shown}");
    assert_eq!(warning_line(fr, "no row for this note"), "⚠ no row for this note");
    assert_eq!(warning_line(fr, "no row: for this note"), "⚠ no row\u{a0}: for this note");
}

/// The warnings and the error for a page longer than 14,400 pt are translated: a PDF 1.5 file
/// written as PDF 1.6, a page with a `UserUnit` with the option that writes it in points, a
/// PDF/X-4 page in points, a refused PDF/X-1a file and a page too narrow for the `UserUnit` its
/// long side needs.
#[test]
fn long_page_notes_are_translated() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 56_692.91, "height": 2_834.65})).unwrap();
    let mut notes: Vec<String> = vec![];
    for p in [json!({"compatibility": "1.5", "longPagesInPoints": false}), json!({"preset": "PDF/X-4:2010"})] {
        let r = s.execute("document.exportPdf", &p).unwrap();
        let warnings = r["warnings"].as_array().unwrap().iter().filter_map(|w| w.as_str());
        notes.extend(warnings.filter(|w| w.contains("14,400 pt")).map(str::to_string));
    }
    notes.push(s.execute("document.exportPdf", &json!({"preset": "PDF/X-1a:2001"})).unwrap_err().to_string());
    s.execute("file.new", &json!({"width": 85_040, "height": 17})).unwrap();
    let r = s.execute("document.exportPdf", &json!({"longPagesInPoints": false})).unwrap();
    let warnings = r["warnings"].as_array().unwrap().iter().filter_map(|w| w.as_str());
    notes.extend(warnings.filter(|w| w.contains("too long and narrow")).map(str::to_string));
    assert_eq!(notes.len(), 5, "{notes:?}");
    let english = [
        "has no UserUnit, which",
        "apps that ignore UserUnit",
        "apps that keep to the limit",
        "real numbers up to",
        "too long and narrow",
        "turn on",
    ];
    for code in ["de", "fr", "ja", "zh-hans"] {
        let lang = crate::i18n::Lang::from_code(code).unwrap();
        for note in &notes {
            let shown = warning_line(lang, note);
            assert!(english.iter().all(|e| !shown.contains(e)), "{code}: {shown}");
        }
    }
}

/// A placed image whose pixel data can't be decoded is left out of the PDF, and the status bar
/// shows the note raster exports give.
#[test]
fn an_image_that_couldnt_be_decoded_is_named_in_the_status() {
    let (mut app, written, _) = app();
    // A PNG whose header reads but whose pixel data is cut short.
    let mut png = vec![];
    {
        let __dyn: image::DynamicImage = (image::RgbaImage::from_pixel(64, 32, image::Rgba([200, 30, 30, 255]))).into();
        let (__w, __h) = (__dyn.width(), __dyn.height());
        let (__layout, __sample, __raw) = match __dyn {
            image::DynamicImage::ImageLuma8(b) => (craft_image_codecs::ChannelLayout::Gray, craft_image_codecs::SampleType::U8, b.into_raw()),
            image::DynamicImage::ImageLumaA8(b) => (craft_image_codecs::ChannelLayout::GrayA, craft_image_codecs::SampleType::U8, b.into_raw()),
            image::DynamicImage::ImageRgb8(b) => (craft_image_codecs::ChannelLayout::Rgb, craft_image_codecs::SampleType::U8, b.into_raw()),
            image::DynamicImage::ImageRgba8(b) => (craft_image_codecs::ChannelLayout::Rgba, craft_image_codecs::SampleType::U8, b.into_raw()),
            image::DynamicImage::ImageLuma16(b) => (
                craft_image_codecs::ChannelLayout::Gray,
                craft_image_codecs::SampleType::U16,
                b.into_raw().into_iter().flat_map(|s| s.to_ne_bytes()).collect(),
            ),
            image::DynamicImage::ImageLumaA16(b) => (
                craft_image_codecs::ChannelLayout::GrayA,
                craft_image_codecs::SampleType::U16,
                b.into_raw().into_iter().flat_map(|s| s.to_ne_bytes()).collect(),
            ),
            image::DynamicImage::ImageRgb16(b) => (
                craft_image_codecs::ChannelLayout::Rgb,
                craft_image_codecs::SampleType::U16,
                b.into_raw().into_iter().flat_map(|s| s.to_ne_bytes()).collect(),
            ),
            image::DynamicImage::ImageRgba16(b) => (
                craft_image_codecs::ChannelLayout::Rgba,
                craft_image_codecs::SampleType::U16,
                b.into_raw().into_iter().flat_map(|s| s.to_ne_bytes()).collect(),
            ),
            other => (craft_image_codecs::ChannelLayout::Rgba, craft_image_codecs::SampleType::U8, other.into_rgba8().into_raw()),
        };
        let __cimg = craft_image_codecs::Image::from_raw(__w, __h, __layout, __sample, __raw).unwrap();
        craft_image_codecs::encode(&__cimg, craft_image_codecs::Format::Png, &craft_image_codecs::EncodeOptions::default())
            .map(|__b| png.extend_from_slice(&__b))
    }
    .unwrap();
    // The image is placed on the first artboard.
    vectorcraft_testkit::fixtures::place_cut_png(&mut app.session, &png, json!({"at": [60, 50]}));
    app.run("ui.savePdfDialog", json!({"path": "/tmp/cut.pdf"})).unwrap();
    super::super::confirm(&mut app).unwrap();
    assert_eq!(app.ui.status, "Saved /tmp/cut.pdf with 1 note: an image couldn't be decoded and is missing from the picture");
    assert_eq!(written.borrow().len(), 1);
}

#[test]
fn presets_reset_the_settings_and_fields_fall_back_to_defaults() {
    let (mut app, _, _) = app();
    app.run("ui.savePdfDialog", json!({"compatibility": "1.4", "marks": {"trim": true}})).unwrap();
    let d = app.ui.dialog.as_mut().unwrap();
    assert_eq!(get(d, "compatibility"), "1.4");
    assert_eq!(get(d, "marks.weight"), 0.25, "the rest of a section keeps its defaults");
    d.fields.remove("compression");
    assert_eq!(get(d, "compression.color.ppi"), 300.0, "a field an agent dropped reads as its default");
    let mut d = d.clone();
    apply_preset(&mut app, &mut d, pdf::DEFAULT_PRESET);
    assert_eq!(get(&d, "compatibility"), "1.7");
    assert_eq!(get(&d, "marks.trim"), false);
    assert!(app.run("ui.savePdfDialog", json!({"compatibility": "1.0"})).is_err(), "bad options are refused");
}

/// Reset (`ui.dialogReset`, or Cancel with Alt or Option held) puts every option back to the
/// chosen preset's settings, passwords and their checkboxes included, and keeps the preset, the
/// path, the artboard range, the section and the dialog. After another preset is chosen, Reset
/// goes back to that one, and choosing it also clears the password checkboxes.
#[test]
fn reset_puts_the_chosen_presets_settings_back_and_keeps_the_path_and_artboards() {
    let (mut app, written, _) = app();
    app.run("pdf.preset.save", json!({"name": "Mine", "compatibility": "1.5", "marks": {"trim": true}})).unwrap();
    app.run("ui.savePdfDialog", json!({"path": "/tmp/reset.pdf", "preset": "Mine"})).unwrap();
    set_field(&mut app, "__allArtboards", json!(false));
    set_field(&mut app, "range", json!("2"));
    set_field(&mut app, "__section", json!("Security"));
    set_field(&mut app, "compatibility", json!("1.7"));
    set_field(&mut app, "marks.trim", json!(false));
    set_field(&mut app, "compression.color.ppi", json!(72));
    set_field(&mut app, OPEN_PASSWORD, json!("secret"));
    set_field(&mut app, "__restrictPermissions", json!(true));
    let r = app.run("ui.dialogReset", json!({})).unwrap();
    assert_eq!(r["kind"], KIND);
    let d = app.ui.dialog.as_ref().expect("Reset leaves the dialog open");
    assert_eq!(get(d, "compatibility"), "1.5");
    assert_eq!(get(d, "marks.trim"), true);
    assert_eq!(get(d, "compression.color.ppi"), 300.0, "a nested option goes back too");
    assert_eq!(get(d, OPEN_PASSWORD).as_str().unwrap_or_default(), "", "the password is cleared");
    assert!(!d.fields.contains_key("__restrictPermissions"), "the checkbox is cleared");
    assert_eq!(
        (d.str("preset"), d.str("path"), d.str("range"), d.str("__section")),
        ("Mine".to_string(), "/tmp/reset.pdf".to_string(), "2".to_string(), "Security".to_string())
    );
    assert!(!d.bool("__allArtboards"));

    let mut d = d.clone();
    d.fields.insert("__requireOpenPassword".into(), json!(true));
    apply_preset(&mut app, &mut d, pdf::DEFAULT_PRESET);
    assert!(!d.fields.contains_key("__requireOpenPassword"), "choosing a preset clears the checkbox");
    app.ui.dialog = Some(d);
    set_field(&mut app, "compatibility", json!("1.4"));
    app.run("ui.dialogReset", json!({})).unwrap();
    let d = app.ui.dialog.as_ref().expect("the dialog stays open");
    assert_eq!(d.str("preset"), pdf::DEFAULT_PRESET);
    assert_eq!(get(d, "compatibility"), "1.7");
    assert_eq!(get(d, "marks.trim"), false);
    // OK writes the reset options to the kept path and range.
    super::super::confirm(&mut app).unwrap();
    let w = written.borrow();
    assert_eq!((w.len(), w[0].0.as_str()), (1, "/tmp/reset.pdf"));
    assert!(w[0].1.starts_with(b"%PDF-1.7"));
    assert_eq!(vectorcraft_pdf::import(&w[0].1).unwrap().artboards.len(), 1, "range 2");
}

/// Reset with a saved preset deleted while the dialog is open is an error naming the preset, and
/// it leaves the options and the password as they are rather than switching to another preset's.
#[test]
fn reset_with_a_deleted_preset_is_an_error_and_keeps_the_options() {
    let (mut app, _, _) = app();
    app.run("pdf.preset.save", json!({"name": "Gone", "compatibility": "1.5"})).unwrap();
    app.run("ui.savePdfDialog", json!({"preset": "Gone"})).unwrap();
    set_field(&mut app, "compatibility", json!("1.6"));
    set_field(&mut app, OPEN_PASSWORD, json!("secret"));
    app.run("pdf.preset.delete", json!({"name": "Gone"})).unwrap();
    let e = app.run("ui.dialogReset", json!({})).unwrap_err();
    assert!(e.contains("unknown PDF preset `Gone`"), "{e}");
    let d = app.ui.dialog.as_ref().expect("the dialog stays open");
    assert_eq!(d.str("preset"), "Gone");
    assert_eq!(get(d, "compatibility"), "1.6");
    assert_eq!(get(d, OPEN_PASSWORD), "secret");
}

#[test]
fn marks_and_bleeds_reach_the_pdf() {
    let (mut app, written, _) = app();
    app.run("document.setup", json!({"bleed": 9})).unwrap();
    let notes = app.run("layer.new", json!({})).unwrap()["id"].clone();
    app.run("layer.setProps", json!({"id": notes, "printable": false})).unwrap();
    app.run("shape.ellipse", json!({"x": 60, "y": 40, "width": 20, "height": 20})).unwrap();
    app.run("ui.savePdfDialog", json!({"path": "/tmp/marks.pdf"})).unwrap();
    set_field(&mut app, "__section", json!("Marks and Bleeds"));
    set_field(&mut app, "bleed.useDocument", json!(true));
    set_field(&mut app, "marks.trim", json!(true));
    frame(&mut app);
    set_field(&mut app, "__section", json!("General"));
    set_field(&mut app, "includeNonPrinting", json!(true));
    frame(&mut app);
    let r = super::super::confirm(&mut app).unwrap();
    assert_eq!(r["warnings"], json!([]), "marks and bleed are applied");
    let bytes = &written.borrow()[0].1;
    let page = &vectorcraft_pdf::info(bytes, None).unwrap().pages[0];
    let trim = page.boxes.iter().find(|(b, _)| *b == vectorcraft_pdf::CropTo::Trim).unwrap().1;
    assert_eq!((trim.width(), trim.x0), (120.0, 9.0 + 18.0 + 0.25), "the trim box is the artboard, inside the bleed and marks");
    let mut ellipse = false;
    vectorcraft_pdf::import(bytes).unwrap().walk(|n| {
        let b = n.path_data().and_then(|p| p.bounds()).unwrap_or_default();
        ellipse |= (b.width() - 20.0).abs() < 0.1 && (b.height() - 20.0).abs() < 0.1;
    });
    assert!(ellipse, "the non-printing layer is included");
}

/// The four custom bleed values, `[top, bottom, left, right]`.
fn bleed_of(d: &Dialog) -> [f64; 4] {
    BLEED_SIDES.map(|p| get(d, p).as_f64().unwrap_or(f64::NAN))
}

/// Headless frames of the dialog layer with an accessibility tree, as a 1280 × 900 window.
struct LinkFrames(egui::Context);

impl LinkFrames {
    fn new() -> Self {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install_fonts(&ctx);
        Self(ctx)
    }

    fn run(&self, app: &mut VectorcraftApp, events: Vec<egui::Event>) -> egui::FullOutput {
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 900.0));
        let input = egui::RawInput { events, screen_rect: Some(screen), ..Default::default() };
        let mut out = self.0.run_ui(input, |ui| super::super::show(app, ui.ctx()));
        out.textures_delta.clear();
        out
    }

    /// The bleed link's button once the dialog has settled: its name, whether it shows pressed,
    /// whether it is grayed out, and its center.
    fn link(&self, app: &mut VectorcraftApp) -> (String, bool, bool, egui::Pos2) {
        self.run(app, vec![]);
        let out = self.run(app, vec![]);
        let nodes = &out.platform_output.accesskit_update.as_ref().expect("an accessibility tree").nodes;
        let names = ["Make the bleed values differ", "Make all bleed settings the same"];
        let mut found = nodes.iter().filter(|(_, n)| n.role() == egui::accesskit::Role::Button && n.label().is_some_and(|l| names.contains(&l)));
        let (_, n) = found.next().expect("a bleed link button");
        assert!(found.next().is_none(), "one bleed link button");
        let b = n.bounds().expect("bounds");
        let pressed = n.toggled() == Some(egui::accesskit::Toggled::True) || n.is_selected() == Some(true);
        let center = egui::pos2((b.x0 + b.x1) as f32 * 0.5, (b.y0 + b.y1) as f32 * 0.5);
        (n.label().unwrap_or_default().to_string(), pressed, n.is_disabled(), center)
    }

    fn click(&self, app: &mut VectorcraftApp, at: egui::Pos2) {
        let press = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
        self.run(app, vec![egui::Event::PointerMoved(at), press(true)]);
        self.run(app, vec![press(false)]);
    }
}

/// The link button beside the custom bleed values: the built-in presets' equal sides open linked,
/// an edit to one side sets all four while linked and only that one while unlinked, and linking
/// again gives every side the top's value. The link isn't an export option, and the linked values
/// reach the PDF's page boxes.
#[test]
fn the_bleed_link_keeps_the_four_sides_the_same() {
    let (mut app, written, _) = app();
    app.run("ui.savePdfDialog", json!({"path": "/tmp/bleed.pdf"})).unwrap();
    set_field(&mut app, "__section", json!("Marks and Bleeds"));
    let f = LinkFrames::new();
    let (name, pressed, disabled, at) = f.link(&mut app);
    assert_eq!((name.as_str(), pressed, disabled), ("Make the bleed values differ", true, false), "equal sides open linked");
    edit_bleed(app.ui.dialog.as_mut().unwrap(), 2, 6.0);
    assert_eq!(bleed_of(app.ui.dialog.as_ref().unwrap()), [6.0; 4], "linked: the left value sets all four");

    f.click(&mut app, at);
    let (name, pressed, ..) = f.link(&mut app);
    assert_eq!((name.as_str(), pressed), ("Make all bleed settings the same", false), "the click unlinks");
    let d = app.ui.dialog.as_mut().unwrap();
    assert_eq!(d.fields.get(BLEED_LINKED), Some(&json!(false)));
    edit_bleed(d, 1, 9.0);
    edit_bleed(d, 3, 2.0);
    assert_eq!(bleed_of(d), [6.0, 9.0, 6.0, 2.0], "unlinked: each side is its own");
    edit_bleed(d, 0, 4.0);
    assert_eq!(bleed_of(d), [4.0, 9.0, 6.0, 2.0]);

    let (.., at) = f.link(&mut app);
    f.click(&mut app, at);
    let (name, pressed, ..) = f.link(&mut app);
    assert_eq!((name.as_str(), pressed), ("Make the bleed values differ", true), "the click links again");
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!(bleed_of(d), [4.0; 4], "linking gives every side the top's value");
    let p = params(d);
    assert_eq!(p["bleed"], json!({"useDocument": false, "top": 4.0, "bottom": 4.0, "left": 4.0, "right": 4.0}));
    assert!(p.get(BLEED_LINKED).is_none() && preset_params(d).get(BLEED_LINKED).is_none(), "the link is the dialog's own");

    super::super::confirm(&mut app).unwrap();
    let bytes = &written.borrow()[0].1;
    let page = &vectorcraft_pdf::info(bytes, None).unwrap().pages[0];
    let bleed = page.boxes.iter().find(|(b, _)| *b == vectorcraft_pdf::CropTo::Bleed).expect("a bleed box").1;
    let trim = page.boxes.iter().find(|(b, _)| *b == vectorcraft_pdf::CropTo::Trim).expect("a trim box").1;
    assert_eq!((trim.x0 - bleed.x0, trim.y0 - bleed.y0, bleed.x1 - trim.x1, bleed.y1 - trim.y1), (4.0, 4.0, 4.0, 4.0));
}

/// A preset whose four bleed values differ opens unlinked; choosing a preset or Reset sets the link
/// from that preset's values rather than keeping the one shown before.
#[test]
fn presets_and_reset_set_the_bleed_link_from_their_values() {
    let (mut app, _, _) = app();
    app.run("pdf.preset.save", json!({"name": "Uneven", "bleed": {"top": 3, "bottom": 5, "left": 3, "right": 3}})).unwrap();
    app.run("ui.savePdfDialog", json!({"preset": "Uneven"})).unwrap();
    let d = app.ui.dialog.as_ref().unwrap();
    assert!(!bleed_linked(d), "unequal sides open unlinked");
    assert_eq!(bleed_of(d), [3.0, 5.0, 3.0, 3.0], "opening leaves the values as they are");

    let mut d = d.clone();
    link_bleed(&mut d, true);
    apply_preset(&mut app, &mut d, "Uneven");
    assert!(!bleed_linked(&d) && bleed_of(&d) == [3.0, 5.0, 3.0, 3.0], "choosing the preset takes its values and its link");
    apply_preset(&mut app, &mut d, pdf::DEFAULT_PRESET);
    assert!(bleed_linked(&d), "the default preset's sides are equal");
    link_bleed(&mut d, false);
    app.ui.dialog = Some(d);
    app.run("ui.dialogReset", json!({})).unwrap();
    assert!(bleed_linked(app.ui.dialog.as_ref().unwrap()), "Reset sets the link from the preset's values");

    // The preset editor has the same link.
    open_preset(&mut app, &json!({"name": "Uneven"})).unwrap();
    set_field(&mut app, "__section", json!("Marks and Bleeds"));
    let f = LinkFrames::new();
    let (name, pressed, disabled, at) = f.link(&mut app);
    assert_eq!((name.as_str(), pressed, disabled), ("Make all bleed settings the same", false, false));
    f.click(&mut app, at);
    assert_eq!(bleed_of(app.ui.dialog.as_ref().unwrap()), [3.0; 4]);
}

/// With Use Document Bleed Settings on, the fields show the document's bleed and the link is
/// grayed out; the custom values and the link stay as they were for when it is turned off.
#[test]
fn the_document_bleed_grays_the_link_and_keeps_the_custom_values() {
    let (mut app, _, _) = app();
    app.run("document.setup", json!({"bleed": 9})).unwrap();
    app.run("ui.savePdfDialog", json!({"bleed": {"top": 3, "bottom": 5, "left": 3, "right": 3}})).unwrap();
    set_field(&mut app, "__section", json!("Marks and Bleeds"));
    set_field(&mut app, "bleed.useDocument", json!(true));
    let f = LinkFrames::new();
    let (name, pressed, disabled, at) = f.link(&mut app);
    assert_eq!((name.as_str(), pressed, disabled), ("Make all bleed settings the same", false, true));
    f.click(&mut app, at);
    let d = app.ui.dialog.as_ref().unwrap();
    assert!(!d.fields.contains_key(BLEED_LINKED), "a grayed-out link does not change");
    assert_eq!(bleed_of(d), [3.0, 5.0, 3.0, 3.0]);
    set_field(&mut app, "bleed.useDocument", json!(false));
    let (name, _, disabled, _) = f.link(&mut app);
    assert_eq!((name.as_str(), disabled), ("Make all bleed settings the same", false));
}

/// `ui.dialog.set` can set the link (`__bleedLinked`) as its button does, refuses a value other
/// than true or false, and a new `bleed` lets the link follow its values.
#[test]
fn ui_dialog_set_links_the_bleed() {
    let (mut app, _, _) = app();
    app.run("ui.savePdfDialog", json!({"bleed": {"top": 3, "bottom": 5, "left": 3, "right": 3}})).unwrap();
    let set = |app: &mut VectorcraftApp, field: &str, value: Value| {
        let d = app.ui.dialog.as_mut().unwrap();
        assert!(super::super::accepts_field(d, field), "{field}");
        let old = d.fields.get(field).cloned();
        super::super::set_field(d, field.into(), value);
        super::super::field_set(app, field, old.as_ref())
    };
    set(&mut app, BLEED_LINKED, json!(true)).unwrap();
    assert_eq!(bleed_of(app.ui.dialog.as_ref().unwrap()), [3.0; 4], "linking gives every side the top's value");
    for bad in [json!("yes"), json!(1), Value::Null] {
        let e = set(&mut app, BLEED_LINKED, bad.clone()).unwrap_err();
        assert!(e.contains("__bleedLinked must be true or false"), "{bad}: {e}");
    }
    set(&mut app, BLEED_LINKED, json!(false)).unwrap();
    set(&mut app, "bleed", json!({"top": 1, "bottom": 1, "left": 1, "right": 1})).unwrap();
    let d = app.ui.dialog.as_ref().unwrap();
    assert!(!d.fields.contains_key(BLEED_LINKED) && bleed_linked(d), "equal values set are linked");
    set(&mut app, "bleed", json!({"top": 1, "bottom": 2, "left": 1, "right": 1})).unwrap();
    assert!(!bleed_linked(app.ui.dialog.as_ref().unwrap()), "unequal values set are unlinked");

    // Writing the same `bleed` again also lets the link follow its values; a `__bleedLinked` set
    // after it in the same request still decides.
    set(&mut app, "bleed", json!({"top": 1, "bottom": 1, "left": 1, "right": 1})).unwrap();
    set(&mut app, BLEED_LINKED, json!(false)).unwrap();
    set(&mut app, "bleed", json!({"top": 1, "bottom": 1, "left": 1, "right": 1})).unwrap();
    assert!(bleed_linked(app.ui.dialog.as_ref().unwrap()), "the same equal values set again are linked");
    set(&mut app, BLEED_LINKED, json!(false)).unwrap();
    let d = app.ui.dialog.as_ref().unwrap();
    assert!(!bleed_linked(d) && bleed_of(d) == [1.0; 4], "a later __bleedLinked unlinks them");
}

/// An unlinked edit that makes the four sides equal leaves the link off, so the next edit
/// changes one side only.
#[test]
fn an_unlinked_edit_that_evens_the_sides_keeps_the_link_off() {
    let (mut app, _, _) = app();
    app.run("ui.savePdfDialog", json!({"bleed": {"top": 3, "bottom": 5, "left": 3, "right": 3}})).unwrap();
    let d = app.ui.dialog.as_mut().unwrap();
    assert!(!bleed_linked(d));
    edit_bleed(d, 1, 3.0);
    assert!(!bleed_linked(d), "the link stays off");
    edit_bleed(d, 2, 6.0);
    assert_eq!(bleed_of(d), [3.0, 3.0, 6.0, 3.0], "the next edit changes one side");
}

#[test]
fn option_labels_read_well() {
    assert_eq!(option_label("compression.color.abovePpi"), "Compression › Color › Above Ppi");
    assert_eq!(option_label("compression.compressText"), "Compression › Compress Text");
    assert_eq!(option_label("thumbnails"), "General › Thumbnails");
    assert_eq!(option_label("bleed.top"), "Marks and Bleeds › Bleed › Top");
    assert_eq!(section_of("bleed.top"), 2);
    assert_eq!(SECTIONS[section_of("standard")], "General");
}

#[test]
fn confirming_preserves_passwords_supplied_when_opening_the_dialog() {
    for (open, owner) in [("open", ""), ("", "owner"), ("open", "owner")] {
        let (mut app, written, _) = app();
        app.run(
            "ui.savePdfDialog",
            json!({"path": "/tmp/locked.pdf", "security": {"openPassword": open, "permissionsPassword": owner, "printing": "none"}}),
        )
        .unwrap();
        // Drawing the Security and Summary sections must retain the live passwords.
        for section in ["Security", "Summary"] {
            set_field(&mut app, "__section", json!(section));
            frame(&mut app);
        }
        let p = params(app.ui.dialog.as_ref().unwrap());
        let summary = summary_of(&mut app, &egui::Context::default(), &p).unwrap();
        assert!(!summary.to_string().contains("Password"), "summaries omit passwords");
        let mut d = app.ui.dialog.clone().unwrap();
        set(&mut d, SAVE_AS, json!("Protected"));
        save_preset(&mut app, &mut d).unwrap();
        let saved = &app.session.prefs.pdf_presets[0].settings.security;
        assert!(saved.open_password.is_empty() && saved.permissions_password.is_empty(), "presets forget passwords");

        super::super::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
        let written = written.borrow();
        let bytes = &written[0].1;
        assert!(String::from_utf8_lossy(bytes).contains("/Encrypt"));
        if open.is_empty() {
            assert_eq!(vectorcraft_pdf::info(bytes, None).unwrap().pages.len(), 3);
        } else {
            assert_eq!(vectorcraft_pdf::info(bytes, None).unwrap_err(), vectorcraft_pdf::PdfError::NeedsPassword);
        }
        assert_eq!(vectorcraft_pdf::info(bytes, Some("wrong")).unwrap_err(), vectorcraft_pdf::PdfError::WrongPassword);
        for password in [open, owner].into_iter().filter(|p| !p.is_empty()) {
            assert_eq!(vectorcraft_pdf::info(bytes, Some(password)).unwrap().pages.len(), 3);
        }
    }
}

#[test]
fn the_security_section_takes_passwords_that_presets_drop() {
    let (mut app, _, _) = app();
    app.run("ui.savePdfDialog", json!({})).unwrap();
    set_field(&mut app, "__section", json!("Security"));
    set_field(&mut app, OPEN_PASSWORD, json!("open"));
    set_field(&mut app, PERMISSIONS_PASSWORD, json!("own"));
    set_field(&mut app, "security.copy", json!(false));
    frame(&mut app);
    let p = params(app.ui.dialog.as_ref().unwrap());
    assert_eq!((p["security"]["openPassword"].as_str(), p["security"]["permissionsPassword"].as_str()), (Some("open"), Some("own")));
    assert_eq!(p["security"]["copy"], false);
    // With a standard the section still draws (greyed); a preset brings its own security.
    set_field(&mut app, "standard", json!(Standard::PdfA2b.id()));
    frame(&mut app);
    let mut d = app.ui.dialog.clone().unwrap();
    apply_preset(&mut app, &mut d, pdf::DEFAULT_PRESET);
    assert_eq!(get(&d, OPEN_PASSWORD), &Value::Null, "presets carry no passwords");
    assert_eq!(get(&d, "security.copy"), true);
}

#[test]
fn choosing_a_pdfx_standard_sets_its_version_and_writes_the_standard() {
    let (mut app, written, _) = app();
    app.run("ui.savePdfDialog", json!({"path": "/tmp/x.pdf", "createLayers": true})).unwrap();
    let d = app.ui.dialog.as_mut().unwrap();
    set(d, "standard", json!(Standard::PdfX1a.id()));
    standard_chosen(d, Standard::PdfX1a);
    assert_eq!((get(d, "compatibility"), get(d, "createLayers"), get(d, "preserveEditing")), (&json!("1.3"), &json!(false), &json!(false)));
    for s in SECTIONS {
        set_field(&mut app, "__section", json!(s));
        frame(&mut app);
    }
    super::super::confirm(&mut app).unwrap();
    let pdf = String::from_utf8_lossy(&written.borrow()[0].1).into_owned();
    assert!(pdf.starts_with("%PDF-1.3") && pdf.contains("/GTS_PDFXConformance(PDF/X-1a:2001)") && pdf.contains("/S/GTS_PDFX"));
    // PDF/X-4 keeps layers and is PDF 1.6.
    app.run("ui.savePdfDialog", json!({"compatibility": "2.0", "createLayers": true})).unwrap();
    let d = app.ui.dialog.as_mut().unwrap();
    set(d, "standard", json!(Standard::PdfX4.id()));
    standard_chosen(d, Standard::PdfX4);
    assert_eq!((get(d, "compatibility"), get(d, "createLayers")), (&json!("1.6"), &json!(true)));
}

/// Profile inclusion is disabled with a standard, so choosing PDF/X-3 or PDF/X-4 sets it back to
/// unset: a `none` chosen before would write every CMYK color as DeviceCMYK.
#[test]
fn choosing_pdfx3_or_pdfx4_unsets_profile_inclusion() {
    let (mut app, _, _) = app();
    for standard in [Standard::PdfX3, Standard::PdfX4] {
        app.run("ui.savePdfDialog", json!({"path": "/tmp/x.pdf", "output": {"profiles": "none"}})).unwrap();
        let d = app.ui.dialog.as_mut().unwrap();
        assert_eq!(get(d, "output.profiles"), &json!("none"));
        set(d, "standard", json!(standard.id()));
        standard_chosen(d, standard);
        assert_eq!(get(d, "output.profiles"), &json!(vectorcraft_pdf::UNSET_CHOICE), "{}", standard.id());
    }
}

/// Opened with PDF/X-3 or PDF/X-4 and `none` (params or a preset, without choosing the standard),
/// the dialog keeps `none` and its Output note says every CMYK color is written as DeviceCMYK.
#[test]
fn a_loaded_pdfx_standard_with_no_profiles_says_cmyk_is_device_cmyk() {
    const NOTE: &str = "every CMYK color is written as DeviceCMYK";
    let (mut app, _, _) = app();
    for (standard, profiles, shown) in
        [(Standard::PdfX4, "none", true), (Standard::PdfX3, "none", true), (Standard::PdfX4, "all", false), (Standard::PdfX1a, "none", false)]
    {
        let params = json!({"path": "/tmp/x.pdf", "standard": standard.id(), "output": {"profiles": profiles}});
        app.run("ui.savePdfDialog", params).unwrap();
        set_field(&mut app, "__section", json!("Output"));
        assert_eq!(get(app.ui.dialog.as_ref().unwrap(), "output.profiles"), &json!(profiles));
        let text = crate::tests_labels::painted_text(&mut app, |app, ui| super::super::show(app, ui.ctx()));
        assert_eq!(text.contains(NOTE), shown, "{} {profiles}: {text}", standard.id());
    }
}

/// "Write pages longer than 200 in at full size in points" is enabled only without a standard, at
/// PDF 1.5 and later: PDF/X files and PDF 1.3 and 1.4 files write long pages in points anyway, and
/// PDF/A files keep the `UserUnit`.
#[test]
fn the_long_pages_in_points_check_box_is_disabled_with_a_standard() {
    const LABEL: &str = "Write pages longer than 200 in at full size in points";
    let (mut app, _, _) = app();
    app.run("ui.savePdfDialog", json!({})).unwrap();
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    theme::install_fonts(&ctx);
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 900.0));
    let frame = |app: &mut VectorcraftApp| {
        let input = egui::RawInput { screen_rect: Some(screen), ..Default::default() };
        let mut out = ctx.run_ui(input, |ui| super::super::show(app, ui.ctx()));
        out.textures_delta.clear();
        out
    };
    for s in Standard::ALL.iter().copied() {
        for c in Compatibility::ALL.iter().copied().filter(|c| s.allows(*c)) {
            set_field(&mut app, "standard", json!(s.id()));
            set_field(&mut app, "compatibility", json!(c.id()));
            frame(&mut app);
            let out = frame(&mut app);
            let nodes = &out.platform_output.accesskit_update.as_ref().expect("an accessibility tree").nodes;
            let node = nodes.iter().find(|(_, n)| n.label() == Some(LABEL) && n.role() == egui::accesskit::Role::CheckBox);
            let disabled = node.unwrap_or_else(|| panic!("no check box named {LABEL:?}")).1.is_disabled();
            let enabled = s == Standard::None && !matches!(c, Compatibility::Pdf13 | Compatibility::Pdf14);
            assert_eq!(disabled, !enabled, "{} {}", s.label(), c.label());
        }
    }
}

#[test]
fn create_layers_writes_pdf_layers() {
    let (mut app, written, _) = app();
    app.run("layer.new", json!({"name": "Notes"})).unwrap();
    app.run("shape.ellipse", json!({"x": 60, "y": 40, "width": 20, "height": 20})).unwrap();
    app.run("ui.savePdfDialog", json!({"path": "/tmp/layers.pdf", "createLayers": true})).unwrap();
    // The option is drawn greyed at PDF 1.4 (no PDF layers there), then live again.
    for c in ["1.4", "1.7"] {
        set_field(&mut app, "compatibility", json!(c));
        frame(&mut app);
    }
    let r = super::super::confirm(&mut app).unwrap();
    assert_eq!(r["warnings"], json!([]));
    let back = vectorcraft_pdf::import(&written.borrow()[0].1).unwrap();
    let names: Vec<_> = back.layers.iter().filter_map(|l| l.name.clone()).collect();
    // The empty pages of the other artboards come in as page layers.
    assert_eq!(names[..2], ["Layer 1", "Notes"]);
}

#[test]
fn pdf_1_3_takes_a_flattener_preset_in_advanced() {
    let (mut app, written, _) = app();
    app.run("ui.savePdfDialog", json!({"path": "/tmp/old.pdf", "compatibility": "1.3"})).unwrap();
    for s in ["Advanced", "Security"] {
        set_field(&mut app, "__section", json!(s));
        frame(&mut app);
    }
    set_field(&mut app, "flattenerPreset", json!("Low Resolution"));
    let p = params(app.ui.dialog.as_ref().unwrap());
    assert_eq!((p["compatibility"].as_str(), p["flattenerPreset"].as_str()), (Some("1.3"), Some("Low Resolution")));
    super::super::confirm(&mut app).unwrap();
    assert!(written.borrow()[0].1.starts_with(b"%PDF-1.3"));
    // A preset keeps the flattener preset.
    app.run("pdf.preset.save", json!({"name": "Old", "compatibility": "1.3", "flattenerPreset": "Low Resolution"})).unwrap();
    let saved = app.session.prefs.pdf_presets.iter().find(|p| p.name == "Old").unwrap();
    assert_eq!((saved.settings.compatibility, saved.settings.flattener_preset.as_str()), (Compatibility::Pdf13, "Low Resolution"));
}

#[test]
fn save_pdf_flattens_transparent_document_without_modal_prompt() {
    let (mut app, written, _) = app();
    // Add transparent artwork to the document
    app.run("transparency.set", json!({"opacity": 50})).unwrap();

    app.run("ui.savePdfDialog", json!({"path": "/tmp/transparent.pdf", "compatibility": "1.3"})).unwrap();
    assert_eq!(app.ui.dialog.as_ref().unwrap().kind, KIND);

    // Clicking Save PDF goes ahead without stopping at a confirmation prompt
    super::super::confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none(), "Save PDF closes without a modal prompt");
    assert_eq!(written.borrow().len(), 1);
    assert!(written.borrow()[0].1.starts_with(b"%PDF-1.3"));
}

#[test]
fn summary_warnings_include_flattener_resolutions_when_flattening_transparency() {
    let (mut app, _, _) = app();
    app.run("transparency.set", json!({"opacity": 50})).unwrap();

    app.run("ui.savePdfDialog", json!({"path": "/tmp/out.pdf", "compatibility": "1.3"})).unwrap();
    let p = params(app.ui.dialog.as_ref().unwrap());
    let v = summary_of(&mut app, &egui::Context::default(), &p).unwrap();
    let warnings = v["warnings"].as_array().unwrap();
    assert!(
        warnings.iter().any(|w| {
            let s = w.as_str().unwrap();
            s.contains("transparency is flattened into opaque art and images")
                && s.contains("at 1200 ppi line art and text, 300 ppi gradients and meshes")
        }),
        "{warnings:?}"
    );

    // With a custom flattener override, the resolution in the warning reflects the options
    set_field(&mut app, "flattener.lineArtPpi", json!(600));
    set_field(&mut app, "flattener.gradientPpi", json!(200));
    let p2 = params(app.ui.dialog.as_ref().unwrap());
    let v2 = summary_of(&mut app, &egui::Context::default(), &p2).unwrap();
    let warnings2 = v2["warnings"].as_array().unwrap();
    assert!(
        warnings2.iter().any(|w| {
            let s = w.as_str().unwrap();
            s.contains("at 600 ppi line art and text, 200 ppi gradients and meshes")
        }),
        "{warnings2:?}"
    );
}

#[test]
fn advanced_section_shows_raster_resolution_note() {
    let (mut app, _, _) = app();
    app.run("ui.savePdfDialog", json!({"path": "/tmp/out.pdf", "compatibility": "1.3"})).unwrap();
    set_field(&mut app, "__section", json!("Advanced"));
    set_field(&mut app, "flattener.lineArtPpi", json!(600));
    set_field(&mut app, "flattener.gradientPpi", json!(200));
    frame(&mut app);
    assert!(app.ui.dialog.is_some());
}

/// Headless frames of the dialog layer with screen-reader output.
struct Reader(egui::Context);

impl Reader {
    fn new() -> Self {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install_fonts(&ctx);
        Self(ctx)
    }

    fn run(&self, app: &mut VectorcraftApp, events: Vec<egui::Event>) -> egui::FullOutput {
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 900.0));
        let mut out =
            self.0.run_ui(egui::RawInput { events, screen_rect: Some(screen), ..Default::default() }, |ui| super::super::show(app, ui.ctx()));
        out.textures_delta.clear();
        out
    }

    /// The Custom… button of the open dialog, with whether it is enabled.
    fn custom(&self, app: &mut VectorcraftApp) -> Option<(egui::accesskit::NodeId, bool)> {
        self.run(app, vec![]);
        let out = self.run(app, vec![]);
        let nodes = &out.platform_output.accesskit_update.as_ref()?.nodes;
        nodes.iter().find(|(_, n)| n.label() == Some("Custom…") && n.role() == egui::accesskit::Role::Button).map(|(id, n)| (*id, !n.is_disabled()))
    }

    /// The labels and values the open dialog shows.
    fn texts(&self, app: &mut VectorcraftApp) -> Vec<String> {
        self.run(app, vec![]);
        let out = self.run(app, vec![]);
        let nodes = out.platform_output.accesskit_update.map(|u| u.nodes).unwrap_or_default();
        nodes.iter().flat_map(|(_, n)| [n.label(), n.value()]).flatten().map(str::to_string).collect()
    }

    /// Click Custom… in Save PDF's Advanced section.
    fn click_custom(&self, app: &mut VectorcraftApp) {
        let (node, enabled) = self.custom(app).expect("a Custom… button");
        assert!(enabled, "Custom… is enabled");
        let click = egui::accesskit::ActionRequest {
            action: egui::accesskit::Action::Click,
            target_node: node,
            target_tree: egui::accesskit::TreeId::ROOT,
            data: None,
        };
        self.run(app, vec![egui::Event::AccessKitActionRequest(click)]);
    }
}

fn kind(app: &VectorcraftApp) -> Option<&str> {
    app.ui.dialog.as_ref().map(|d| d.kind.as_str())
}

/// Save PDF on `params` (and `path`), showing Advanced.
fn open_advanced(app: &mut VectorcraftApp, params: Value) {
    let mut p = params;
    p["path"] = json!("/tmp/out.pdf");
    app.run("ui.savePdfDialog", p).unwrap();
    set_field(app, "__section", json!("Advanced"));
}

/// A second rectangle, half transparent, over the first one.
fn transparent_overlap(app: &mut VectorcraftApp) {
    app.run("select.none", json!({})).unwrap();
    app.run("shape.rectangle", json!({"x": 30, "y": 20, "width": 50, "height": 40})).unwrap();
    app.run("transparency.set", json!({"opacity": 50})).unwrap();
}

/// The images a written PDF reopens with.
fn images(pdf: &[u8]) -> usize {
    let doc = vectorcraft_pdf::import(pdf).unwrap();
    let mut n = 0;
    for l in &doc.layers {
        l.walk(&mut |x| n += matches!(x.kind, vectorcraft_doc::NodeKind::Image(_)) as usize);
    }
    n
}

#[test]
fn custom_is_enabled_where_the_save_flattens() {
    let (mut app, _, _) = app();
    let r = Reader::new();
    for (params, flattens) in [
        (json!({"compatibility": "1.3"}), true),
        (json!({"standard": "pdfX1a"}), true),
        (json!({"standard": "pdfX3"}), true),
        (json!({"compatibility": "1.4"}), false),
        (json!({"standard": "pdfX4"}), false),
    ] {
        open_advanced(&mut app, params.clone());
        assert_eq!(r.custom(&mut app).map(|(_, enabled)| enabled), Some(flattens), "{params}");
    }
    // PDF presets keep no custom flattener options, so the preset editor has no Custom….
    open_preset(&mut app, &json!({})).unwrap();
    set_field(&mut app, "__section", json!("Advanced"));
    set_field(&mut app, "compatibility", json!("1.3"));
    assert_eq!(r.custom(&mut app), None);
}

#[test]
fn custom_starts_from_the_preset_and_cancel_goes_back_unchanged() {
    let (mut app, _, _) = app();
    open_advanced(&mut app, json!({"compatibility": "1.3", "flattenerPreset": "low"}));
    let r = Reader::new();
    r.custom(&mut app);
    let before = app.ui.dialog.as_ref().unwrap().fields.clone();
    r.click_custom(&mut app);
    assert_eq!(kind(&app), Some(CUSTOM_KIND));
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!((d.f64("balance", 0.0), d.f64("gradientPpi", 0.0)), (75.0, 150.0), "Low Resolution's");
    // Low Resolution doesn't clip, but the PDF 1.3 writer does.
    assert!(d.bool("clipComplexRegions"), "shown as written");
    app.ui.dialog.as_mut().unwrap().fields.insert("balance".into(), json!(10));
    super::super::cancel(&mut app);
    assert_eq!(kind(&app), Some(KIND));
    assert_eq!(app.ui.dialog.as_ref().unwrap().fields, before, "Cancel leaves Save PDF as it was");
}

#[test]
fn custom_options_are_written_until_a_preset_is_chosen() {
    let (mut app, written, _) = app();
    transparent_overlap(&mut app);
    open_advanced(&mut app, json!({"compatibility": "1.3"}));
    let r = Reader::new();
    r.click_custom(&mut app);
    app.ui.dialog.as_mut().unwrap().fields.insert("balance".into(), json!(0));
    super::super::confirm(&mut app).unwrap();
    assert_eq!(kind(&app), Some(KIND), "OK goes back to Save PDF");
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!(d.fields["flattener"]["balance"].as_f64(), Some(0.0));
    let saved = &app.session.prefs.flattener_presets;
    assert_ne!(flattener(d, saved).ok(), FlattenOptions::preset(pdf::DEFAULT_FLATTENER), "the preset shows [Custom]");
    // A preset set through `ui.dialog.set` drops the custom options.
    let kept = d.clone();
    let dialog = app.ui.dialog.as_mut().unwrap();
    assert!(super::super::accepts_field(dialog, "flattenerPreset"));
    let old = dialog.fields.get("flattenerPreset").cloned();
    super::super::set_field(dialog, "flattenerPreset".into(), json!("Low Resolution"));
    super::super::field_set(&mut app, "flattenerPreset", old.as_ref()).unwrap();
    assert!(!app.ui.dialog.as_ref().unwrap().fields.contains_key("flattener"));
    app.ui.dialog = Some(kept);
    super::super::confirm(&mut app).unwrap();
    assert!(images(&written.borrow()[0].1) > 0, "balance 0 rasterizes the flattened art");
    open_advanced(&mut app, json!({"compatibility": "1.3"}));
    super::super::confirm(&mut app).unwrap();
    assert_eq!(images(&written.borrow()[1].1), 0, "High Resolution keeps it vector");
}

#[test]
fn custom_options_equal_to_the_preset_leave_its_name() {
    let (mut app, _, _) = app();
    let r = Reader::new();
    for overprint in ["preserve", "discard"] {
        open_advanced(&mut app, json!({"compatibility": "1.3", "flattenerPreset": "medium", "advanced": {"overprint": overprint}}));
        r.click_custom(&mut app);
        assert_eq!(app.ui.dialog.as_ref().unwrap().bool("preserveOverprints"), overprint == "preserve", "{overprint}");
        super::super::confirm(&mut app).unwrap();
        assert_eq!(kind(&app), Some(KIND));
        assert!(!app.ui.dialog.as_ref().unwrap().fields.contains_key("flattener"), "{overprint}");
    }
}

/// OK keeps Custom… options that drop overprints dropping them, although the preset keeps them.
#[test]
fn custom_keeps_options_that_drop_overprints() {
    let (mut app, _, _) = app();
    let params = json!({"compatibility": "1.3", "advanced": {"overprint": "preserve"}, "flattener": {"preserveOverprints": false}});
    open_advanced(&mut app, params);
    Reader::new().click_custom(&mut app);
    assert!(!app.ui.dialog.as_ref().unwrap().bool("preserveOverprints"), "shown as written");
    super::super::confirm(&mut app).unwrap();
    assert_eq!(kind(&app), Some(KIND));
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!(flattener(d, &app.session.prefs.flattener_presets).map(|o| o.preserve_overprints), Ok(false));
}

#[test]
fn custom_shows_alpha_as_the_pdf_1_3_writer_drops_it() {
    let (mut app, written, _) = app();
    transparent_overlap(&mut app);
    open_advanced(&mut app, json!({"compatibility": "1.3", "flattener": {"balance": 0, "preserveAlpha": true}}));
    Reader::new().click_custom(&mut app);
    assert!(!app.ui.dialog.as_ref().unwrap().bool("preserveAlpha"), "shown as written");
    super::super::confirm(&mut app).unwrap();
    assert_eq!(kind(&app), Some(KIND));
    assert_eq!(app.ui.dialog.as_ref().unwrap().fields["flattener"]["preserveAlpha"].as_bool(), Some(false));
    super::super::confirm(&mut app).unwrap();
    assert!(written.borrow()[0].1.starts_with(b"%PDF-1.3") && images(&written.borrow()[0].1) > 0);
}

#[test]
fn bad_custom_options_keep_the_dialog_open() {
    let (mut app, _, _) = app();
    open_advanced(&mut app, json!({"compatibility": "1.3"}));
    Reader::new().click_custom(&mut app);
    app.ui.dialog.as_mut().unwrap().fields.insert("gradientPpi".into(), json!(9000));
    let e = super::super::confirm(&mut app).unwrap_err();
    assert!(e.contains("resolutions"), "{e}");
    assert_eq!(kind(&app), Some(CUSTOM_KIND));
}

#[test]
fn custom_options_outlive_a_saved_preset_deleted_meanwhile() {
    let (mut app, written, _) = app();
    app.run("flattener.presets.save", json!({"name": "Mine", "balance": 20})).unwrap();
    open_advanced(&mut app, json!({"compatibility": "1.3", "flattenerPreset": "Mine"}));
    Reader::new().click_custom(&mut app);
    assert_eq!(app.ui.dialog.as_ref().unwrap().f64("balance", 0.0), 20.0, "the saved preset's");
    app.run("flattener.presets.delete", json!({"name": "Mine"})).unwrap();
    app.ui.dialog.as_mut().unwrap().fields.insert("balance".into(), json!(30));
    super::super::confirm(&mut app).unwrap();
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!((d.str("flattenerPreset"), d.fields["flattener"]["balance"].as_f64()), (String::new(), Some(30.0)));
    super::super::confirm(&mut app).unwrap();
    assert!(written.borrow()[0].1.starts_with(b"%PDF-1.3"));
}

/// OK keeps the overprint option of a saved preset deleted meanwhile, whichever value Overprint
/// shows it with, although the default preset keeps overprints.
#[test]
fn custom_keeps_the_overprint_option_of_a_saved_preset_deleted_meanwhile() {
    let (mut app, _, _) = app();
    for (kept, overprint) in [(false, "preserve"), (true, "discard")] {
        app.run("flattener.presets.save", json!({"name": "Mine", "balance": 20, "preserveOverprints": kept})).unwrap();
        open_advanced(&mut app, json!({"compatibility": "1.3", "flattenerPreset": "Mine", "advanced": {"overprint": overprint}}));
        Reader::new().click_custom(&mut app);
        assert!(!app.ui.dialog.as_ref().unwrap().bool("preserveOverprints"), "shown as written ({overprint})");
        app.run("flattener.presets.delete", json!({"name": "Mine"})).unwrap();
        super::super::confirm(&mut app).unwrap();
        let d = app.ui.dialog.as_ref().unwrap();
        assert_eq!(d.fields["flattener"]["preserveOverprints"].as_bool(), Some(kept), "{overprint}");
    }
}

/// OK keeps the overprint option Custom… opened with when its saved preset is deleted and made
/// again under the same name with another overprint option meanwhile.
#[test]
fn custom_keeps_the_overprint_option_it_opened_with_over_a_preset_made_again() {
    let (mut app, _, _) = app();
    app.run("flattener.presets.save", json!({"name": "Mine", "balance": 20, "preserveOverprints": false})).unwrap();
    open_advanced(&mut app, json!({"compatibility": "1.3", "flattenerPreset": "Mine", "advanced": {"overprint": "preserve"}}));
    Reader::new().click_custom(&mut app);
    assert!(!app.ui.dialog.as_ref().unwrap().bool("preserveOverprints"), "shown as the preset had it");
    app.run("flattener.presets.delete", json!({"name": "Mine"})).unwrap();
    app.run("flattener.presets.save", json!({"name": "Mine", "balance": 20, "preserveOverprints": true})).unwrap();
    super::super::confirm(&mut app).unwrap();
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!(d.fields["flattener"]["preserveOverprints"].as_bool(), Some(false));
}

/// Flattener preset shows a preset named in another case by its name, not as [Custom].
#[test]
fn flattener_preset_shows_a_name_given_in_any_case() {
    let (mut app, _, _) = app();
    app.run("flattener.presets.save", json!({"name": "Mine", "balance": 20})).unwrap();
    let r = Reader::new();
    for (given, shown) in [("MINE", "Mine"), ("HIGH RESOLUTION", "High Resolution")] {
        open_advanced(&mut app, json!({"compatibility": "1.3", "flattenerPreset": given}));
        let texts = r.texts(&mut app);
        assert!(texts.iter().any(|t| t == shown), "{given}: {texts:?}");
        assert!(!texts.iter().any(|t| t == "[Custom]"), "{given}: {texts:?}");
    }
}

#[test]
fn save_preset_says_it_leaves_the_custom_flattener_options_out() {
    let (mut app, _, _) = app();
    let r = Reader::new();
    let note = "The PDF preset keeps the flattener preset but not the Custom… settings.";
    app.run("ui.savePdfDialog", json!({"compatibility": "1.3", "flattener": {"balance": 10}})).unwrap();
    set_field(&mut app, SAVE_AS, json!("Mine"));
    assert!(r.texts(&mut app).iter().any(|t| t == note), "with Custom… options");
    app.ui.dialog.as_mut().unwrap().fields.remove("flattener");
    assert!(!r.texts(&mut app).iter().any(|t| t == note), "without them");
}

#[test]
fn save_pdf_opens_with_the_flattener_options_given_and_refuses_bad_ones() {
    let (mut app, _, _) = app();
    app.run("ui.savePdfDialog", json!({"compatibility": "1.3", "flattener": {"lineArtPpi": 600}})).unwrap();
    assert_eq!(app.ui.dialog.as_ref().unwrap().fields["flattener"], json!({"lineArtPpi": 600}));
    for bad in [json!({"bogus": 1}), json!(5), json!({"gradientPpi": 9000})] {
        app.ui.dialog = None;
        assert!(app.run("ui.savePdfDialog", json!({"flattener": bad})).is_err(), "{bad}");
        assert!(app.ui.dialog.is_none(), "{bad}");
    }
}
