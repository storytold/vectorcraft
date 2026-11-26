//! A name typed in a save panel without an extension gets the chosen format's: Save, Save As,
//! Save a Copy, Export, Export Selection and other writes through a save panel. When the name
//! then names a file that is there, which the panel did not ask about, the app asks first.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_engine::Session;

use super::picks::{self, Picker, answer_dialog, answer_replace, cancel_dialog, open_dialogs, replace_asked, scripted};
use crate::{Services, VectorcraftApp, io};

type Written = Rc<RefCell<Vec<String>>>;

/// An app over `services` with a held save panel and a document holding one rectangle.
fn with_document(services: Services) -> (VectorcraftApp, Picker) {
    let mut app = VectorcraftApp::new(Session::new(), services);
    let picker = scripted(&mut app);
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 30, "height": 20})).unwrap();
    (app, picker)
}

/// An app whose writer records the paths it is given.
fn saving_app() -> (VectorcraftApp, Picker, Written) {
    let written = Written::default();
    let w = written.clone();
    let (app, picker) = with_document(Services {
        write: Some(Box::new(move |p: &str, _: &[u8]| {
            w.borrow_mut().push(p.to_string());
            Ok(())
        })),
        ..Default::default()
    });
    (app, picker, written)
}

/// An app in a folder of its own named after `tag`, where files are written and found → (the app, the folder).
fn on_disk(tag: &str) -> (VectorcraftApp, Picker, PathBuf) {
    let dir = std::env::temp_dir().join(format!("vc-saveext-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let (app, picker) = with_document(Services {
        write: Some(Box::new(|p: &str, b: &[u8]| std::fs::write(p, b).map_err(|e| e.to_string()))),
        exists: Some(Arc::new(|p: &str| Path::new(p).exists())),
        ..Default::default()
    });
    (app, picker, dir)
}

fn in_dir(dir: &Path, name: &str) -> String {
    dir.join(name).to_string_lossy().into_owned()
}

fn name_of(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn last(w: &Written) -> String {
    w.borrow().last().cloned().unwrap_or_default()
}

fn poll(app: &mut VectorcraftApp) {
    picks::poll(app, &crate::testing::ui_context());
}

/// The index of the file type with extension `ext` in the last save panel's list.
fn type_index(picker: &Picker, ext: &str) -> Option<usize> {
    picker.requests().last()?.filters.iter().position(|f| f.extensions.iter().any(|e| e == ext))
}

/// Press OK (or Cancel) on the top dialog after setting `fields`.
fn answer_top(app: &mut VectorcraftApp, fields: &[(&str, Value)], ok: bool) {
    let id = open_dialogs(app).pop().map(|d| d.id).expect("a dialog is open");
    answer_dialog(app, id, fields, ok);
    if !ok {
        cancel_dialog(app, id);
    }
}

/// A field of the top dialog, as automation lists it.
fn top_field(app: &VectorcraftApp, field: &str) -> Option<Value> {
    open_dialogs(app).pop()?.fields?.get(field).cloned()
}

/// The path the top dialog holds: the options of the format being written. Save PDF's dialog lists its options only;
/// the file it writes is the app's job.
fn dialog_path(app: &VectorcraftApp) -> Option<String> {
    let top = open_dialogs(app).pop()?;
    if let Some(open) = app.save_pdf.as_ref().filter(|open| open.id == top.id) {
        return open.job.as_ref()?.path.clone();
    }
    top.fields?.get("path")?.as_str().map(str::to_string)
}

/// Run `run`, which opens a save panel, and answer the panel with `path` and the file type with
/// extension `ext` (the Linux portal reports it; Windows and macOS answer `None`).
fn save_via<R, E>(
    app: &mut VectorcraftApp,
    picker: &Picker,
    run: impl FnOnce(&mut VectorcraftApp) -> Result<R, E>,
    (path, ext): (&str, Option<&str>),
) {
    // The reply is pending, or an error saying the command waits for its panel.
    let _ = run(app);
    let file_type = ext.map(|e| type_index(picker, e).expect("the panel offers the picked type"));
    picker.answer_saved(path, file_type);
    poll(app);
}

/// [`save_via`] for a command.
fn save_with(app: &mut VectorcraftApp, picker: &Picker, cmd: &str, params: Value, answer: (&str, Option<&str>)) {
    save_via(app, picker, |a| a.run(cmd, params), answer);
}

/// Export As with `params`: its own dialog's OK asks for the file.
fn export_as(app: &mut VectorcraftApp, params: Value) -> Result<Value, String> {
    let r = app.run("file.exportAs", params)?;
    answer_top(app, &[], true);
    Ok(r)
}

/// Run `cmd` and answer its save panel → the path the save goes on with: the one written, or the one
/// the dialog that opens next holds (the options of SVG, PDF, and of native and .ai files on Save As).
fn saved_from(app: &mut VectorcraftApp, picker: &Picker, written: &Written, cmd: &str, answer: (&str, Option<&str>)) -> String {
    written.borrow_mut().clear();
    save_with(app, picker, cmd, json!({}), answer);
    dialog_path(app).unwrap_or_else(|| last(written))
}

/// Each row is (command, name typed, extension of the type picked, path saved), on a fresh app.
fn assert_saved(rows: &[(&str, &str, Option<&str>, &str)]) {
    for &(cmd, typed, ext, expected) in rows {
        let (mut app, picker, w) = saving_app();
        assert_eq!(saved_from(&mut app, &picker, &w, cmd, (typed, ext)), expected, "{cmd} {typed} {ext:?}");
    }
}

/// Run `run`, then check the types the save panel it opened lists, and close the panel.
fn shows(
    app: &mut VectorcraftApp,
    picker: &Picker,
    run: impl FnOnce(&mut VectorcraftApp) -> Result<Value, String>,
    expected: &[(&str, &[&str])],
    what: &str,
) {
    let _ = run(app);
    let shown: Vec<(String, Vec<String>)> =
        picker.requests().pop().map(|r| r.filters.into_iter().map(|f| (f.name, f.extensions)).collect()).unwrap_or_default();
    let expected: Vec<(String, Vec<String>)> =
        expected.iter().map(|(name, exts)| (name.to_string(), exts.iter().map(|e| e.to_string()).collect())).collect();
    assert_eq!(shown, expected, "{what}");
    picker.cancel();
    poll(app);
}

/// Document Info's, the libraries', the presets', the CSS, the PostScript and the shortcut saves
/// show the one file type they write in the save panel, as Save and Export show theirs.
#[test]
fn save_panels_show_the_file_type_they_write() {
    use vectorcraft_engine::cmd::{flatten, pdfcmds, perspgrid, printpresets};
    let (mut app, picker, _) = saving_app();
    shows(&mut app, &picker, |a| a.run("docInfo.save", json!({})), &[("Text", &["txt"])], "Document Info › Save");
    for format in ["vcswatches", "gpl", "ase", "css"] {
        let label = if format == "css" { "CSS" } else { "Swatch libraries" };
        let run = |a: &mut VectorcraftApp| {
            a.run("ui.saveSwatchLibrary", json!({}))?;
            answer_top(a, &[("format", json!(format))], true);
            Ok(Value::Null)
        };
        shows(&mut app, &picker, run, &[(label, &[format])], &format!("Save Swatch Library as .{format}"));
    }
    let run = |a: &mut VectorcraftApp| {
        a.run("ui.saveGraphicStyleLibrary", json!({}))?;
        answer_top(a, &[], true);
        Ok(Value::Null)
    };
    shows(&mut app, &picker, run, &[("Graphic Style Libraries", &["vcstyles"])], "Save Graphic Style Library");
    let id = app.run("library.create", json!({"name": "Brand"})).unwrap()["id"].as_str().unwrap().to_string();
    let run = |a: &mut VectorcraftApp| {
        io::save_command_output_named(a, "library.export", "Brand.vclibrary", json!({"library": id}), |_, _| Ok(Value::Null))
    };
    shows(&mut app, &picker, run, &[("Libraries", &["vclibrary"])], "Libraries panel › Export Library…");
    shows(&mut app, &picker, |a| a.run("css.exportFile", json!({"scope": "all"})), &[("CSS", &["css"])], "CSS Properties › Export All…");
    shows(&mut app, &picker, |a| a.run("file.print", json!({"format": "postscript"})), &[("PostScript", &["ps"])], "Print to a PostScript file");
    shows(&mut app, &picker, |a| a.run("shortcuts.export", json!({})), &[("Shortcut sets", &["json"])], "Keyboard Shortcuts › Export…");
    shows(
        &mut app,
        &picker,
        |a| a.run("shortcuts.exportText", json!({})),
        &[("Text", &["txt"]), ("Shortcut sheet", &["pdf"]), ("CSV", &["csv"])],
        "Keyboard Shortcuts › Export Text…",
    );
    shows(
        &mut app,
        &picker,
        |a| a.run("shortcuts.exportText", json!({"format": "sheet"})),
        &[("Shortcut sheet", &["pdf"])],
        "shortcuts.exportText with a format",
    );
    let presets = [
        ("pdf.preset.save", json!({"name": "Mine"}), "pdf.preset.export", "PDF presets", pdfcmds::PRESET_EXTS),
        ("print.presets.save", json!({"name": "Mine"}), "print.presets.export", "Print presets", printpresets::PRESET_EXTS),
        ("flattener.presets.save", json!({"name": "Mine"}), "flattener.presets.export", "Flattener presets", flatten::PRESET_EXTS),
        ("perspective.presets.save", json!({"name": "Mine"}), "perspective.presets.export", "Perspective grid presets", perspgrid::PRESET_EXTS),
        (
            "document.exportPresets.save",
            json!({"name": "Mine", "settings": {"formats": [{"format": "svg"}]}}),
            "document.exportPresets.export",
            "Export presets",
            vectorcraft_engine::cmd::fileio::SCREEN_EXPORT_PRESET_EXTS,
        ),
    ];
    for (save, params, export, label, exts) in presets {
        app.run(save, params).unwrap();
        let run = |a: &mut VectorcraftApp| crate::dialogs::preset_manager::export(a, export, exts[0], "Mine").map(|_| Value::Null);
        shows(&mut app, &picker, run, &[(label, exts)], &format!("{export}: the presets dialog's Export…"));
    }
}

#[test]
fn saves_add_the_format_extension_to_a_bare_name() {
    for (cmd, params, typed, expected) in [
        ("file.save", json!({}), "/tmp/Untitled-1", "/tmp/Untitled-1.vectorcraft"),
        ("file.saveCopy", json!({"format": "svg", "svg": {}}), "/tmp/copy", "/tmp/copy.svg"),
        ("file.saveAs", json!({"format": "pdf", "options": {}}), "/tmp/as", "/tmp/as.pdf"),
    ] {
        let (mut app, picker, w) = saving_app();
        save_with(&mut app, &picker, cmd, params, (typed, None));
        assert_eq!(last(&w), expected, "{cmd}");
        if cmd == "file.save" {
            assert_eq!(app.session.active().unwrap().path.as_deref(), Some(expected));
        }
    }
}

/// The file type picked in a save dialog that reports it decides the format (#2679): a bare name, or
/// one with the suggested name's extension in any letter case, gets the picked type's extension; an
/// extension typed for another save format stays as typed, another format's extension is replaced
/// with the picked type's, and any other gets the picked type's added. The picked type's own
/// extension typed in another case stays as typed (`a.AI` with .ai picked).
#[test]
fn the_file_type_picked_in_the_save_dialog_decides_the_format() {
    assert_saved(&[
        ("file.save", "/art/poster", Some("ai"), "/art/poster.ai"),
        ("file.save", "/art/Untitled-1.vectorcraft", Some("ai"), "/art/Untitled-1.ai"),
        ("file.save", "/art/poster.SVG", Some("ai"), "/art/poster.SVG"),
        ("file.save", "/art/poster.foo", Some("ai"), "/art/poster.foo.ai"),
        ("file.save", "/art/poster.ai", Some("vectorcraft"), "/art/poster.ai"),
        ("file.save", "/art/poster", Some("svg"), "/art/poster.svg"),
        // A dialog that doesn't say, as on Windows and macOS: the default format's extension.
        ("file.save", "/art/poster", None, "/art/poster.vectorcraft"),
        ("file.save", "/art/Untitled-1.VECTORCRAFT", Some("ai"), "/art/Untitled-1.ai"),
        ("file.save", "/art/Untitled-1.VectorCraft", Some("svg"), "/art/Untitled-1.svg"),
        ("file.saveAsTemplate", "/art/Untitled-1.VCTemplate", Some("ait"), "/art/Untitled-1.ait"),
    ]);
    // A document saved as .ai suggests `a.ai`.
    let (mut app, picker, w) = saving_app();
    app.run("file.saveAs", json!({"path": "/art/a.ai", "options": {}})).unwrap();
    assert_eq!(saved_from(&mut app, &picker, &w, "file.saveAs", ("/art/a.AI", Some("ai"))), "/art/a.AI");
}

/// A name typed with the extension of a format the save does not write has it replaced, with or
/// without a picked type: `card.ait` and `card.png` are saved as `card.vectorcraft`, not
/// `card.ait.vectorcraft`. An extension that names no format stays part of the name.
#[test]
fn a_typed_extension_of_a_format_the_save_does_not_write_is_replaced() {
    assert_saved(&[
        ("file.save", "/art/card.ait", None, "/art/card.vectorcraft"),
        ("file.save", "/art/card.png", None, "/art/card.vectorcraft"),
        ("file.save", "/art/My.Card.PNG", None, "/art/My.Card.vectorcraft"),
        ("file.save", "/art/notes.v2", None, "/art/notes.v2.vectorcraft"),
        ("file.save", "/art/card.png", Some("ai"), "/art/card.ai"),
        ("file.saveCopy", "/art/card.tif", Some("svg"), "/art/card.svg"),
        ("file.saveAsTemplate", "/art/card.ai", None, "/art/card.vctemplate"),
        ("file.saveAsTemplate", "/art/card.pdf", Some("ait"), "/art/card.ait"),
        ("file.saveAsTemplate", "/art/card.ait", None, "/art/card.ait"),
    ]);
}

/// The name the app gave (the extension replaced, or the picked type's added) names a file that is
/// there, which the dialog did not ask about: the save asks first, and the question's OK writes that
/// file in the picked format.
#[test]
fn a_replaced_extension_asks_before_replacing_a_file_there() {
    for (typed, ext, there, format) in
        [("/art/card.png", None, "/art/card.vectorcraft", None), ("/art/poster", Some("ai"), "/art/poster.ai", Some("ai"))]
    {
        let (mut app, picker, w) = saving_app();
        app.services.exists = Some(Arc::new(move |p: &str| p == there));
        save_with(&mut app, &picker, "file.save", json!({}), (typed, ext));
        assert_eq!(replace_asked(&app).as_deref(), Some(name_of(there)), "{typed}");
        assert!(w.borrow().is_empty());
        answer_replace(&mut app, true);
        assert_eq!(*w.borrow(), [there]);
        if let Some(format) = format {
            assert_eq!(app.session.active().unwrap().format, format);
        }
    }
}

/// Save As to the type picked asks for that format's options: the .ai options (Version first),
/// not VectorCraft's.
#[test]
fn save_as_asks_for_the_options_of_the_file_type_picked() {
    let (mut app, picker, w) = saving_app();
    assert_eq!(saved_from(&mut app, &picker, &w, "file.saveAs", ("/art/poster", Some("ai"))), "/art/poster.ai");
    assert_eq!(top_field(&app, "format"), Some(json!("ai")));
}

/// A document saved as .ai lists .ai first: VectorCraft picked with a bare name or with the
/// suggested name saves a native file. Save a Copy and Save as Template follow the type picked too.
#[test]
fn a_document_in_another_format_saves_as_the_type_picked() {
    let (mut app, picker, w) = saving_app();
    app.run("file.saveAs", json!({"path": "/art/a.ai", "options": {}})).unwrap();
    assert_eq!(*w.borrow(), ["/art/a.ai"]);
    assert_eq!(saved_from(&mut app, &picker, &w, "file.saveAs", ("/art/poster", Some("vectorcraft"))), "/art/poster.vectorcraft");
    answer_top(&mut app, &[], false);
    assert_eq!(saved_from(&mut app, &picker, &w, "file.saveAs", ("/art/a.ai", Some("vectorcraft"))), "/art/a.vectorcraft");
    answer_top(&mut app, &[], false);
    assert_eq!(saved_from(&mut app, &picker, &w, "file.saveCopy", ("/art/copy", Some("svg"))), "/art/copy.svg");
    answer_top(&mut app, &[], false);
    assert_eq!(saved_from(&mut app, &picker, &w, "file.saveAsTemplate", ("/art/card", Some("ait"))), "/art/card.ait");
}

/// `card` typed in the save panel becomes `card.vectorcraft`, a file the panel did not ask about:
/// Save asks before replacing it, Cancel leaves it as it was, and OK replaces it.
#[test]
fn a_name_given_its_extension_asks_before_replacing_a_file_there() {
    let (mut app, picker, dir) = on_disk("ask");
    let card = dir.join("card.vectorcraft");
    let path = card.to_string_lossy().into_owned();
    let typed = in_dir(&dir, "card");
    std::fs::write(&card, b"the old card").unwrap();
    save_with(&mut app, &picker, "file.save", json!({}), (&typed, None));
    assert!(replace_asked(&app).is_some(), "Save asks before replacing card.vectorcraft");
    answer_replace(&mut app, false);
    assert!(!app.any_dialog_open());
    assert_eq!(std::fs::read(&card).unwrap(), b"the old card", "Cancel leaves the file as it was");
    assert_eq!(app.session.active().unwrap().path, None);
    save_with(&mut app, &picker, "file.save", json!({}), (&typed, None));
    answer_replace(&mut app, true);
    assert!(!app.any_dialog_open());
    assert_ne!(std::fs::read(&card).unwrap(), b"the old card", "OK replaces it");
    assert_eq!(app.session.active().unwrap().path.as_deref(), Some(path.as_str()));
    let _ = std::fs::remove_dir_all(&dir);
}

/// A name given its extension writes at once when no file is there, and a name typed with its
/// extension writes at once too (the save panel checked that file itself).
#[test]
fn a_name_given_its_extension_writes_at_once_when_no_file_is_there() {
    let (mut app, picker, dir) = on_disk("fresh");
    save_with(&mut app, &picker, "file.save", json!({}), (&in_dir(&dir, "fresh"), None));
    assert!(replace_asked(&app).is_none());
    assert!(dir.join("fresh.vectorcraft").exists());
    let _ = std::fs::remove_dir_all(&dir);
    let (mut app, picker, dir) = on_disk("typed");
    std::fs::write(dir.join("card.vectorcraft"), b"the old card").unwrap();
    save_with(&mut app, &picker, "file.save", json!({}), (&in_dir(&dir, "card.vectorcraft"), None));
    assert!(replace_asked(&app).is_none());
    assert_ne!(std::fs::read(dir.join("card.vectorcraft")).unwrap(), b"the old card");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Save a Copy, Export Selection, Export for Office and Export As's OK ask the same way before
/// replacing a file that a name given its extension names, and the question's OK writes it.
#[test]
fn every_write_through_a_save_panel_asks_before_replacing() {
    let cases = [
        ("file.saveCopy", json!({"format": "svg", "svg": {}}), "/tmp/out.svg"),
        ("document.exportSelection", json!({}), "/tmp/out.png"),
        ("document.exportForOffice", json!({"ppi": 150}), "/tmp/out.png"),
    ];
    for (cmd, params, path) in cases {
        let (mut app, picker, w) = saving_app();
        app.services.exists = Some(Arc::new(|_: &str| true));
        app.run("select.all", json!({})).unwrap();
        save_with(&mut app, &picker, cmd, params, ("/tmp/out", None));
        assert!(w.borrow().is_empty(), "{cmd}: nothing is written while it asks");
        assert_eq!(replace_asked(&app).as_deref(), Some(name_of(path)), "{cmd}");
        answer_replace(&mut app, true);
        // Export Selection without params continues in the format's options, whose OK writes it.
        if cmd == "document.exportSelection" {
            assert_eq!(dialog_path(&app).as_deref(), Some(path), "{cmd}");
            assert!(w.borrow().is_empty(), "{cmd}: nothing is written before the options' OK");
            answer_top(&mut app, &[], true);
        }
        assert_eq!(last(&w), path, "{cmd}");
    }
    // Export As: the question's OK goes on to PNG Options.
    let (mut app, picker, w) = saving_app();
    app.services.exists = Some(Arc::new(|_: &str| true));
    save_via(&mut app, &picker, |a| export_as(a, json!({"format": "png"})), ("/tmp/out", None));
    assert_eq!(replace_asked(&app).as_deref(), Some("out.png"));
    answer_replace(&mut app, true);
    assert_eq!(dialog_path(&app).as_deref(), Some("/tmp/out.png"));
    assert!(w.borrow().is_empty());
}

/// Export As to a format without Use Artboards (a template holds the whole document) gives the
/// export no artboard option, so an export that automation confirms gets no note that one was
/// not used.
#[test]
fn export_as_a_template_is_given_no_artboard_option() {
    for use_artboards in [false, true] {
        let (mut app, picker, w) = saving_app();
        app.ui.answering_control = true;
        let params = json!({"format": "template", "useArtboards": use_artboards});
        save_via(&mut app, &picker, |a| export_as(a, params), ("/tmp/out", None));
        assert_eq!(last(&w), "/tmp/out.vctemplate");
        assert!(!app.ui.status.contains("was not used"), "{}", app.ui.status);
    }
}

#[test]
fn typed_extensions_are_kept() {
    for (typed, format, expected) in [
        ("/tmp/art.svg", "svg", "/tmp/art.svg"),
        ("/tmp/My.Poster.SVG", "svg", "/tmp/My.Poster.SVG"),
        // A dot in the name isn't an extension.
        ("/tmp/v1.2", "png", "/tmp/v1.2.png"),
    ] {
        let (mut app, picker, w) = saving_app();
        save_via(&mut app, &picker, |a| io::export(a, Some(format), None, &json!({})), (typed, None));
        assert_eq!(last(&w), expected);
    }
}

/// A name typed in an export's save panel with the extension of another format than the one the
/// export writes gets the written format's extension: Export As PNG with `card.svg` typed goes
/// on to PNG Options with `card.png`, and each export in the table below does the same.
#[test]
fn an_export_replaces_a_typed_extension_of_another_format() {
    let (mut app, picker, _) = saving_app();
    save_via(&mut app, &picker, |a| export_as(a, json!({"format": "png"})), ("/tmp/card.svg", None));
    assert_eq!(dialog_path(&app).as_deref(), Some("/tmp/card.png"), "Export As");
    for (typed, expected, what) in [
        ("/tmp/card.svg", "/tmp/card.png", "export"),
        ("/tmp/card.PNG", "/tmp/card.pdf", "Save as PDF"),
        ("/tmp/card.ai", "/tmp/card.pdf", "Save as PDF"),
        ("/tmp/card.svg", "/tmp/card.png", "document.exportForOffice"),
        ("/tmp/sel.png", "/tmp/sel.svg", "document.exportSelection"),
        ("/tmp/sel.svgz", "/tmp/sel.svg", "document.exportSelection"),
        ("/tmp/sel", "/tmp/sel.svg", "document.exportSelection"),
        ("/tmp/art.svg", "/tmp/art.txt", "Text Export Options"),
        ("/tmp/Brand.svg", "/tmp/Brand.vclibrary", "Export Library…"),
    ] {
        let (mut app, picker, w) = saving_app();
        app.run("select.all", json!({})).unwrap();
        let run = |a: &mut VectorcraftApp| match what {
            "export" => io::export(a, Some("png"), None, &json!({})).map(|_| ()),
            "Save as PDF" => io::export_pdf(a, json!({})).map(|_| ()),
            "document.exportForOffice" => a.run(what, json!({"ppi": 150})).map(|_| ()),
            "document.exportSelection" => a.run(what, json!({"format": "svg"})).map(|_| ()),
            "Text Export Options" => {
                io::save_command_output(a, "document.export", "txt", json!({"format": "txt"}), |_, _| Ok(Value::Null)).map(|_| ())
            }
            _ => {
                let id = a.run("library.create", json!({"name": "Brand"}))?["id"].as_str().unwrap_or_default().to_string();
                io::save_command_output_named(a, "library.export", "Brand.vclibrary", json!({"library": id}), |_, _| Ok(Value::Null)).map(|_| ())
            }
        };
        save_via(&mut app, &picker, run, (typed, None));
        assert_eq!(last(&w), expected, "{what} with {typed}");
    }
}

/// A name typed with an extension of the format the export writes stays as typed, in any letter
/// case (`photo.JPEG` for a JPEG), and `art.svgz` for an SVG export is written compressed. Export
/// Selection without a `format` writes the format the typed extension names.
#[test]
fn an_export_keeps_a_typed_extension_of_the_format_it_writes() {
    let (mut app, picker, w) = saving_app();
    save_via(&mut app, &picker, |a| io::export(a, Some("jpg"), None, &json!({})), ("/tmp/photo.JPEG", None));
    assert_eq!(last(&w), "/tmp/photo.JPEG");
    let (mut app, picker, _) = saving_app();
    save_via(&mut app, &picker, |a| export_as(a, json!({"format": "svg"})), ("/tmp/art.svgz", None));
    assert_eq!(dialog_path(&app).as_deref(), Some("/tmp/art.svgz"), "Export As SVG");
    let (mut app, picker, dir) = on_disk("svgz");
    save_via(&mut app, &picker, |a| io::export(a, Some("svg"), None, &json!({})), (&in_dir(&dir, "art.svgz"), None));
    assert_eq!(std::fs::read(dir.join("art.svgz")).unwrap()[..2], [0x1f, 0x8b], "written compressed");
    let _ = std::fs::remove_dir_all(&dir);
    let (mut app, picker, w) = saving_app();
    app.run("select.all", json!({})).unwrap();
    save_with(&mut app, &picker, "document.exportSelection", json!({}), ("/tmp/sel.pdf", None));
    // Save PDF follows the save panel, and its OK writes the file.
    assert_eq!(dialog_path(&app).as_deref(), Some("/tmp/sel.pdf"));
    answer_top(&mut app, &[], true);
    assert_eq!(last(&w), "/tmp/sel.pdf", "Export Selection takes its format from the typed name");
}
