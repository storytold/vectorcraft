//! A name typed in a save panel without an extension gets the chosen format's: Save, Save As,
//! Save a Copy, Export, Export Selection and other writes through a save panel. When the name
//! then names a file that is there, which the panel did not ask about, the app asks first.

use std::cell::RefCell;
use std::rc::Rc;

use serde_json::json;
use vectorcraft_engine::Session;

use crate::{FilePick, Services, VectorcraftApp, dialogs, io};

type Written = Rc<RefCell<Vec<String>>>;

/// An app with a document whose save panel answers `typed` and whose writer records the paths.
fn saving_app(typed: &'static str) -> (VectorcraftApp, Written) {
    let written = Written::default();
    let w = written.clone();
    let services = Services {
        write: Some(Box::new(move |p: &str, _: &[u8]| {
            w.borrow_mut().push(p.to_string());
            Ok(())
        })),
        pick_save: Some(Box::new(move |_: &FilePick| Some(typed.to_string()))),
        ..Default::default()
    };
    let mut app = VectorcraftApp::new(Session::new(), services);
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 30, "height": 20})).unwrap();
    (app, written)
}

fn last(w: &Written) -> String {
    w.borrow().last().cloned().unwrap_or_default()
}

/// The file types each save panel was asked to show.
type Shown = Rc<RefCell<Vec<Vec<(&'static str, &'static [&'static str])>>>>;

/// An app with a document whose save panel records the file types it shows and answers the
/// suggested name in `/tmp`.
fn panel_app() -> (VectorcraftApp, Shown) {
    let shown = Shown::default();
    let s = shown.clone();
    let services = Services {
        write: Some(Box::new(|_: &str, _: &[u8]| Ok(()))),
        pick_save: Some(Box::new(move |pick: &FilePick| {
            s.borrow_mut().push(pick.filters.clone());
            Some(format!("/tmp/{}", pick.name))
        })),
        ..Default::default()
    };
    let mut app = VectorcraftApp::new(Session::new(), services);
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 30, "height": 20})).unwrap();
    (app, shown)
}

/// The file types the last save panel showed.
fn last_shown(s: &Shown) -> Vec<(&'static str, &'static [&'static str])> {
    s.borrow().last().cloned().unwrap_or_default()
}

/// Document Info's, the libraries', the presets', the CSS, the PostScript and the shortcut saves
/// show the one file type they write in the save panel, as Save and Export show theirs.
#[test]
fn save_panels_show_the_file_type_they_write() {
    use vectorcraft_engine::cmd::{flatten, pdfcmds, perspgrid, printpresets};
    let (mut app, shown) = panel_app();
    app.run("docInfo.save", json!({})).unwrap();
    assert_eq!(last_shown(&shown), [("Text", &["txt"][..])], "Document Info › Save");
    for format in ["vcswatches", "gpl", "ase", "css"] {
        app.run("ui.saveSwatchLibrary", json!({})).unwrap();
        app.ui.dialog.as_mut().unwrap().fields.insert("format".into(), json!(format));
        crate::dialogs::confirm(&mut app).unwrap();
        let label = if format == "css" { "CSS" } else { "Swatch libraries" };
        assert_eq!(last_shown(&shown), [(label, &[format][..])], "Save Swatch Library as .{format}");
    }
    app.run("ui.saveGraphicStyleLibrary", json!({})).unwrap();
    crate::dialogs::confirm(&mut app).unwrap();
    assert_eq!(last_shown(&shown), [("Graphic Style Libraries", &["vcstyles"][..])], "Save Graphic Style Library");
    let id = app.run("library.create", json!({"name": "Brand"})).unwrap()["id"].as_str().unwrap().to_string();
    io::save_command_output_named(&mut app, "library.export", "Brand.vclibrary", json!({"library": id})).unwrap();
    assert_eq!(last_shown(&shown), [("Libraries", &["vclibrary"][..])], "Libraries panel › Export Library…");
    app.run("css.exportFile", json!({"scope": "all"})).unwrap();
    assert_eq!(last_shown(&shown), [("CSS", &["css"][..])], "CSS Properties › Export All…");
    app.run("file.print", json!({"format": "postscript"})).unwrap();
    assert_eq!(last_shown(&shown), [("PostScript", &["ps"][..])], "Print to a PostScript file");
    app.run("shortcuts.export", json!({})).unwrap();
    assert_eq!(last_shown(&shown), [("Shortcut sets", &["json"][..])], "Keyboard Shortcuts › Export…");
    let presets = [
        ("pdf.preset.save", json!({"name": "Mine"}), "pdf.preset.export", "PDF presets", pdfcmds::PRESET_EXTS),
        ("print.presets.save", json!({"name": "Mine"}), "print.presets.export", "Print presets", printpresets::PRESET_EXTS),
        ("flattener.presets.save", json!({"name": "Mine"}), "flattener.presets.export", "Flattener presets", flatten::PRESET_EXTS),
        ("perspective.presets.save", json!({"name": "Mine"}), "perspective.presets.export", "Perspective grid presets", perspgrid::PRESET_EXTS),
    ];
    for (save, params, export, label, exts) in presets {
        app.run(save, params).unwrap();
        io::save_command_output(&mut app, export, exts[0], json!({"names": ["Mine"]})).unwrap();
        assert_eq!(last_shown(&shown), [(label, exts)], "{export}: the presets dialog's Export…");
    }
}

#[test]
fn saves_add_the_format_extension_to_a_bare_name() {
    let (mut app, w) = saving_app("/tmp/Untitled-1");
    app.run("file.save", json!({})).unwrap();
    assert_eq!(last(&w), "/tmp/Untitled-1.vectorcraft");
    assert_eq!(app.session.active().unwrap().path.as_deref(), Some("/tmp/Untitled-1.vectorcraft"));
    let (mut app, w) = saving_app("/tmp/copy");
    app.run("file.saveCopy", json!({"format": "svg", "svg": {}})).unwrap();
    assert_eq!(last(&w), "/tmp/copy.svg");
    let (mut app, w) = saving_app("/tmp/as");
    app.run("file.saveAs", json!({"format": "pdf", "options": {}})).unwrap();
    assert_eq!(last(&w), "/tmp/as.pdf");
}

/// An app with a document whose save dialogs show off the UI thread, as on Linux.
fn portal_app() -> (VectorcraftApp, crate::tests_picks::Shown, crate::tests_picks::Written) {
    let mut services = Services::default();
    let (shown, written) = crate::tests_picks::off_the_ui_thread(&mut services);
    let mut app = VectorcraftApp::new(Session::new(), services);
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 30, "height": 20})).unwrap();
    (app, shown, written)
}

/// Run `cmd`, whose save dialog answers `answer` (the path, then the extension of the file type
/// picked, as the Linux portal's dialog answers) → the path the save goes on with: the one written,
/// or the one the dialog that opens next holds (the options of SVG, PDF, and of native and .ai
/// files on Save As; the question whether to replace a file).
fn saved_from(app: &mut VectorcraftApp, shown: &crate::tests_picks::Shown, written: &Written, cmd: &str, answer: &[&str]) -> String {
    written.borrow_mut().clear();
    assert!(app.run(cmd, json!({})).is_err(), "{cmd} waits for its dialog");
    shown.answer(answer);
    crate::picks::poll(app, &egui::Context::default());
    match app.ui.dialog.as_ref() {
        Some(d) => d.str("path"),
        None => last(written),
    }
}

/// The file type picked in a save dialog that reports it decides the format (#2679): a bare name, or
/// one with the suggested name's extension, gets the picked type's extension; an extension typed for
/// another save format stays as typed, and any other gets the picked type's added.
#[test]
fn the_file_type_picked_in_the_save_dialog_decides_the_format() {
    for (answer, expected) in [
        (&["/art/poster", "ai"][..], "/art/poster.ai"),
        (&["/art/Untitled-1.vectorcraft", "ai"], "/art/Untitled-1.ai"),
        (&["/art/poster.SVG", "ai"], "/art/poster.SVG"),
        (&["/art/poster.foo", "ai"], "/art/poster.foo.ai"),
        (&["/art/poster.ai", "vectorcraft"], "/art/poster.ai"),
        (&["/art/poster", "svg"], "/art/poster.svg"),
        // A dialog that doesn't say, as on Windows and macOS: the default format's extension.
        (&["/art/poster"], "/art/poster.vectorcraft"),
    ] {
        let (mut app, shown, written) = portal_app();
        assert_eq!(saved_from(&mut app, &shown, &written, "file.save", answer), expected, "{answer:?}");
    }
}

/// Save As to the type picked asks for that format's options: the .ai options (Version first),
/// not VectorCraft's.
#[test]
fn save_as_asks_for_the_options_of_the_file_type_picked() {
    let (mut app, shown, written) = portal_app();
    assert_eq!(saved_from(&mut app, &shown, &written, "file.saveAs", &["/art/poster", "ai"]), "/art/poster.ai");
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!((d.kind.as_str(), d.str("format")), (dialogs::save_options::KIND, "ai".to_string()));
}

/// A document saved as .ai lists .ai first: VectorCraft picked with a bare name or with the
/// suggested name saves a native file. Save a Copy and Save as Template follow the type picked too.
#[test]
fn a_document_in_another_format_saves_as_the_type_picked() {
    let (mut app, shown, written) = portal_app();
    app.run("file.saveAs", json!({"path": "/art/a.ai", "options": {}})).unwrap();
    assert_eq!(*written.borrow(), ["/art/a.ai"]);
    assert_eq!(saved_from(&mut app, &shown, &written, "file.saveAs", &["/art/poster", "vectorcraft"]), "/art/poster.vectorcraft");
    dialogs::cancel(&mut app);
    assert_eq!(saved_from(&mut app, &shown, &written, "file.saveAs", &["/art/a.ai", "vectorcraft"]), "/art/a.vectorcraft");
    dialogs::cancel(&mut app);
    assert_eq!(saved_from(&mut app, &shown, &written, "file.saveCopy", &["/art/copy", "svg"]), "/art/copy.svg");
    dialogs::cancel(&mut app);
    assert_eq!(saved_from(&mut app, &shown, &written, "file.saveAsTemplate", &["/art/card", "ait"]), "/art/card.ait");
}

/// The name the type picked gives names a file that is there, which the dialog did not ask about:
/// the save asks first, and the question's OK writes that file in the picked format.
#[test]
fn a_name_given_the_picked_types_extension_asks_before_replacing() {
    let (mut app, shown, written) = portal_app();
    app.services.exists = Some(Box::new(|p: &str| p == "/art/poster.ai"));
    assert_eq!(saved_from(&mut app, &shown, &written, "file.save", &["/art/poster", "ai"]), "/art/poster.ai");
    assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some(dialogs::confirm::REPLACE));
    assert!(written.borrow().is_empty());
    dialogs::confirm(&mut app).unwrap();
    assert_eq!(*written.borrow(), ["/art/poster.ai"]);
    assert_eq!(app.session.active().unwrap().format, "ai");
}

#[test]
fn exports_add_the_format_extension_to_a_bare_name() {
    let (mut app, w) = saving_app("/tmp/art");
    io::export(&mut app, Some("png"), None, &json!({})).unwrap();
    assert_eq!(last(&w), "/tmp/art.png");
    app.run("select.all", json!({})).unwrap();
    app.run("document.exportSelection", json!({})).unwrap();
    assert_eq!(last(&w), "/tmp/art.png", "Export Selection: its default format");
    io::save_command_output(&mut app, "document.export", "txt", json!({"format": "txt"})).unwrap();
    assert_eq!(last(&w), "/tmp/art.txt");
}

/// An app with a document whose save panel answers `typed` in a folder of its own named after
/// `tag`, where files are written and found → (the app, the folder).
fn on_disk(tag: &str, typed: &str) -> (VectorcraftApp, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("vc-saveext-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let picked = dir.join(typed).to_string_lossy().into_owned();
    let services = Services {
        write: Some(Box::new(|p: &str, b: &[u8]| std::fs::write(p, b).map_err(|e| e.to_string()))),
        exists: Some(Box::new(|p: &str| std::path::Path::new(p).exists())),
        pick_save: Some(Box::new(move |_: &FilePick| Some(picked.clone()))),
        ..Default::default()
    };
    let mut app = VectorcraftApp::new(Session::new(), services);
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 30, "height": 20})).unwrap();
    (app, dir)
}

/// `card` typed in the save panel becomes `card.vectorcraft`, a file the panel did not ask about:
/// Save asks before replacing it, Cancel leaves it as it was, and OK replaces it.
#[test]
fn a_name_given_its_extension_asks_before_replacing_a_file_there() {
    let (mut app, dir) = on_disk("ask", "card");
    let card = dir.join("card.vectorcraft");
    let path = card.to_string_lossy().into_owned();
    std::fs::write(&card, b"the old card").unwrap();
    assert!(app.run("file.save", json!({})).is_err(), "nothing is saved while it asks");
    let d = app.ui.dialog.clone().expect("Save asks before replacing card.vectorcraft");
    assert_eq!((d.kind.as_str(), d.str("path")), (dialogs::confirm::REPLACE, path.clone()));
    assert!(d.str("message").contains("card.vectorcraft") && d.str("detail").contains("card"), "{d:?}");
    dialogs::cancel(&mut app);
    assert!(app.ui.dialog.is_none());
    assert_eq!(std::fs::read(&card).unwrap(), b"the old card", "Cancel leaves the file as it was");
    assert_eq!(app.session.active().unwrap().path, None);
    assert!(app.run("file.save", json!({})).is_err());
    dialogs::confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none());
    assert_ne!(std::fs::read(&card).unwrap(), b"the old card", "OK replaces it");
    assert_eq!(app.session.active().unwrap().path.as_deref(), Some(path.as_str()));
    let _ = std::fs::remove_dir_all(&dir);
}

/// The Enter that closed the save panel, still held as the question opens, does not replace the
/// file; Enter pressed again after it is let go does.
#[test]
fn the_enter_that_closed_the_save_panel_does_not_replace_the_file() {
    let (mut app, dir) = on_disk("enter", "card");
    let card = dir.join("card.vectorcraft");
    std::fs::write(&card, b"the old card").unwrap();
    assert!(app.run("file.save", json!({})).is_err());
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let enter = |pressed| egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed, repeat: false, modifiers: Default::default() };
    let frame = |app: &mut VectorcraftApp, events: Vec<egui::Event>| {
        let mut out = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| dialogs::show(app, ui.ctx()));
        out.textures_delta.clear();
    };
    frame(&mut app, vec![enter(true)]);
    assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some(dialogs::confirm::REPLACE));
    assert_eq!(std::fs::read(&card).unwrap(), b"the old card");
    frame(&mut app, vec![enter(false)]);
    frame(&mut app, vec![enter(true)]);
    assert!(app.ui.dialog.is_none());
    assert_ne!(std::fs::read(&card).unwrap(), b"the old card");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A name given its extension writes at once when no file is there, and a name typed with its
/// extension writes at once too (the save panel checked that file itself).
#[test]
fn a_name_given_its_extension_writes_at_once_when_no_file_is_there() {
    let (mut app, dir) = on_disk("fresh", "fresh");
    app.run("file.save", json!({})).unwrap();
    assert!(app.ui.dialog.is_none());
    assert!(dir.join("fresh.vectorcraft").exists());
    let _ = std::fs::remove_dir_all(&dir);
    let (mut app, dir) = on_disk("typed", "card.vectorcraft");
    std::fs::write(dir.join("card.vectorcraft"), b"the old card").unwrap();
    app.run("file.save", json!({})).unwrap();
    assert!(app.ui.dialog.is_none());
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
        let (mut app, w) = saving_app("/tmp/out");
        app.services.exists = Some(Box::new(|_: &str| true));
        app.run("select.all", json!({})).unwrap();
        assert!(app.run(cmd, params).is_err(), "{cmd}");
        assert!(w.borrow().is_empty(), "{cmd}: nothing is written while it asks");
        let d = app.ui.dialog.clone().expect(cmd);
        assert_eq!((d.kind.as_str(), d.str("path")), (dialogs::confirm::REPLACE, path.to_string()), "{cmd}");
        dialogs::confirm(&mut app).unwrap();
        assert_eq!(last(&w), path, "{cmd}");
    }
    // Export As: the question's OK picks the file again, and PNG Options follows.
    let (mut app, w) = saving_app("/tmp/out");
    app.services.exists = Some(Box::new(|_: &str| true));
    app.run("file.exportAs", json!({"format": "png"})).unwrap();
    assert!(dialogs::confirm(&mut app).is_err());
    let open = |app: &VectorcraftApp| app.ui.dialog.as_ref().map(|d| (d.kind.clone(), d.str("path")));
    assert_eq!(open(&app), Some((dialogs::confirm::REPLACE.to_string(), "/tmp/out.png".to_string())));
    dialogs::confirm(&mut app).unwrap();
    assert_eq!(open(&app), Some(("pngOptions".to_string(), "/tmp/out.png".to_string())));
    assert!(w.borrow().is_empty());
}

#[test]
fn typed_extensions_are_kept() {
    let (mut app, w) = saving_app("/tmp/art.svg");
    io::export(&mut app, Some("svg"), None, &json!({})).unwrap();
    assert_eq!(last(&w), "/tmp/art.svg");
    let (mut app, w) = saving_app("/tmp/My.Poster.SVG");
    io::export(&mut app, Some("svg"), None, &json!({})).unwrap();
    assert_eq!(last(&w), "/tmp/My.Poster.SVG");
    // A dot in the name isn't an extension.
    let (mut app, w) = saving_app("/tmp/v1.2");
    io::export(&mut app, Some("png"), None, &json!({})).unwrap();
    assert_eq!(last(&w), "/tmp/v1.2.png");
}
