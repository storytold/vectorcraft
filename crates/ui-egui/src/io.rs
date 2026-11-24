//! File I/O through the injected services (read, write, download, reveal) and the file service's
//! pickers; the engine's `fileio` decodes, encodes and saves every format.

use serde_json::{Value, json};
use vectorcraft_doc::SavedView;
use vectorcraft_doc::cancel;
use vectorcraft_engine::EngineError;
use vectorcraft_engine::cmd::fileio::progress::{self, Progress};
use vectorcraft_engine::cmd::fileio::{self, Format, SAVE_FORMATS, SaveMode, SavePlan, TEMPLATE_FORMATS};

use crate::background::{self, Writer};
use crate::dialogs::svg_options;
use crate::{Services, VectorcraftApp, dialogs};

pub(crate) mod export;
mod open;
mod read_only_save;
pub(crate) mod save;

#[cfg(test)]
mod tests_read_only_save;

pub use export::{EXPORT_CANCELLED, cancel_exports, export, export_for_screens, export_pdf, hide_exports, reveal, save_command_output};
pub(crate) use export::{
    EXPORT_SELECTION, export_files, exported_with_notes, file_url, open_in_app, open_original, reveal_path, run_to_file, save_command_output_named,
    selection_path, take_selection,
};
pub(crate) use open::asks_first;
pub(crate) use open::report_installed;
pub(crate) use open::{EXPORT_PRESETS, import_presets, open_notes, open_with_on, opened, opened_after_notice, show_opened};
pub use open::{
    MAX_RECENT_FILES, OpensAs, ask_revert, install_module, install_plugin, new_from_template, note_recent, open_bytes, open_dialog, open_document,
    open_path, open_path_with, open_reporting, open_with, opens_as, place_bytes, recent_files, report_open_error,
};
#[cfg(test)]
pub(crate) use open::{open_file, opened_with_notes};
pub(crate) use save::save_current;
pub use save::{
    SAVE_CANCELED, ask_format_options, ask_pdf_options, cancel_saves, hide_saves, remember_active_artboard, remember_view, save, save_pdf,
};
pub(crate) use save::{
    clear_notes_left, export_path, is_web, notes_of, report_saved, show_with_notes, target_path, write_named, write_named_over, write_reporting,
    write_to,
};
#[cfg(test)]
pub(crate) use save::{report_write_error, saved_status};

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use serde_json::json;
    use vectorcraft_engine::Session;

    use super::*;
    use crate::Services;
    use crate::tests::picks::{open_dialogs, poll, scripted};
    use vectorcraft_doc::NodeKind;

    type Written = Rc<RefCell<Vec<(String, Vec<u8>)>>>;

    /// An app whose writer records what it is given.
    fn app() -> (VectorcraftApp, Written) {
        let written = Written::default();
        let w = written.clone();
        let services = Services {
            write: Some(Box::new(move |p: &str, b: &[u8]| {
                w.borrow_mut().push((p.to_string(), b.to_vec()));
                Ok(())
            })),
            ..Default::default()
        };
        (windowed(services), written)
    }

    /// An app with `services` and a window context, where its dialogs open.
    fn windowed(services: Services) -> VectorcraftApp {
        crate::testing::windowed(VectorcraftApp::new(Session::new(), services))
    }

    /// The message dialog open on the host, closed as its Close button closes it: its message and detail.
    fn told(app: &mut VectorcraftApp) -> Option<(String, String)> {
        let m = dialogs::confirm::open_message(app)?;
        let ctx = app.ui_ctx.clone()?;
        let id = open_dialogs(app).into_iter().rfind(|d| d.key.as_deref() == Some(dialogs::confirm::MESSAGE))?.id;
        app.dialogs()?.with(|h| h.cancel_by_id(&ctx, id));
        dialogs::host::pump(app, &ctx);
        Some(m)
    }

    /// #861: a file the user opens that can't be opened is reported in a dialog they close, not
    /// only in the status bar; a file an agent opens by path only answers with the error.
    #[test]
    fn a_file_the_user_cant_open_is_reported_in_a_dialog() {
        let (mut app, _) = app();
        app.services.read = Some(Box::new(|_: &str| Err("no such file".into())));
        assert!(open_reporting(&mut app, "/docs/gone.ai").is_err());
        assert_eq!(told(&mut app), Some(("Can't open “gone.ai”.".to_string(), "no such file".to_string())), "a dialog says so");
        assert!(app.ui.status.contains("gone.ai"));
        // An agent's `file.open {path}` gets the error back and no dialog.
        assert!(app.run("file.open", json!({"path": "/docs/gone.ai"})).is_err());
        assert!(open_dialogs(&app).is_empty());
    }

    /// A file opened with a note about its page first and a note that its layers weren't read
    /// second: the status bar names the layers note, counts both, and keeps both, the layers note
    /// first, for its hover. Two notes of what was left out keep the order the import gave them,
    /// and an open without notes leaves the message and the notes an earlier open left.
    #[test]
    fn an_open_names_the_note_of_what_was_left_out_first() {
        use vectorcraft_testkit::ai;
        let (mut app, _) = app();
        let damaged = b"%AI12_CompressedData not zlib";
        let notes = notes_of;
        let pdf = ai::pdf(damaged, "0 g 150 20 100 60 re f");
        let r = open_bytes(&mut app, "damaged.pdf", &pdf, None).unwrap();
        let [page, layers] = &notes(&r)[..] else { panic!("two notes: {r}") };
        assert!(layers.contains("layers weren't read") && !page.contains("layers weren't read"), "{r}");
        assert_eq!(app.ui.status, opened_with_notes(2, layers));
        assert_eq!(app.ui.status_notes, Some((app.ui.status.clone(), vec![layers.clone(), page.clone()])));

        let r = open_bytes(&mut app, "damaged.ai", &ai::ai(damaged, ai::page_pdf()), None).unwrap();
        let both = notes(&r);
        assert_eq!(both.len(), 2, "{r}");
        assert_eq!(app.ui.status, opened_with_notes(2, &both[0]));
        assert_eq!(app.ui.status_notes, Some((app.ui.status.clone(), both)));

        let r = open_document(&mut app, "page.pdf", &pdf, None, &json!({"editingData": false})).unwrap();
        let [only] = &notes(&r)[..] else { panic!("one note: {r}") };
        assert_eq!(app.ui.status, opened_with_notes(1, only));
        assert_eq!(app.ui.status_notes, Some((app.ui.status.clone(), vec![only.clone()])));

        let clean = br#"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="20"><rect width="10" height="10"/></svg>"#;
        open_bytes(&mut app, "damaged.pdf", &pdf, None).unwrap();
        let shown = (app.ui.status.clone(), app.ui.status_notes.clone());
        let r = open_bytes(&mut app, "clean.svg", clean, None).unwrap();
        assert_eq!(r["warnings"], json!([]), "{r}");
        assert_eq!((app.ui.status.clone(), app.ui.status_notes.clone()), shown);
    }

    /// A file opened with two notes and a modified linked file while Update Links doesn't ask: the
    /// status bar shows the line about the linked file, then counts the notes and names the first
    /// of what was left out, and keeps both notes, in that order, for its hover.
    #[test]
    fn an_open_with_modified_links_keeps_link_notice_and_import_notes_reachable() {
        let (mut app, _) = app();
        let pdf = vectorcraft_testkit::ai::pdf(b"%AI12_CompressedData not zlib", "0 g 150 20 100 60 re f");
        let r = open_bytes(&mut app, "damaged.pdf", &pdf, None).unwrap();
        let [page, layers] = &notes_of(&r)[..] else { panic!("two notes: {r}") };
        app.session.prefs.update_links = "manually".into();
        let mut linked = r.clone();
        linked["modifiedLinks"] = json!([{ "ids": [1] }]);
        opened(&mut app, &linked);
        let notice = dialogs::missing_links::changed_on_disk_status(1);
        assert_eq!(app.ui.status, opened_after_notice(&notice, 2, layers));
        assert!(app.ui.status.starts_with(&notice) && app.ui.status.ends_with(&opened_with_notes(2, layers)), "{}", app.ui.status);
        assert_eq!(app.ui.status_notes, Some((app.ui.status.clone(), vec![layers.clone(), page.clone()])));
        assert!(open_dialogs(&app).is_empty());
    }

    /// File › New from Template… shows the notes of the file it opens, as File › Open does.
    #[test]
    fn new_from_template_preserves_open_warnings() {
        let (mut app, _) = app();
        let pdf = vectorcraft_testkit::ai::pdf(b"%AI12_CompressedData not zlib", "0 g 150 20 100 60 re f");
        app.services.read = Some(Box::new(move |_: &str| Ok(pdf.clone())));
        let r = new_from_template(&mut app, Some("/templates/damaged.pdf".into())).unwrap();
        let [page, layers] = &notes_of(&r)[..] else { panic!("two notes: {r}") };
        assert_eq!(app.ui.status, opened_with_notes(2, layers));
        assert_eq!(app.ui.status_notes, Some((app.ui.status.clone(), vec![layers.clone(), page.clone()])));
    }

    /// A save and an export with several notes: the status bar names the first, counts them, and
    /// keeps every one of them, in the reply's order, for its hover. A save without notes keeps
    /// none.
    #[test]
    fn a_save_and_an_export_keep_every_note_for_the_hover() {
        let (mut app, _) = app();
        app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
        let r = app.run("file.save", json!({"path": "/out/a.svg", "compress": true})).unwrap();
        let notes = notes_of(&r);
        assert_eq!(notes.len(), 2, "{r}");
        assert!(notes[0].starts_with("SVG keeps the artwork but not everything"), "{r}");
        assert_eq!(app.ui.status, saved_status("/out/a.svg", &r));
        assert!(app.ui.status.contains("with 2 notes"), "{}", app.ui.status);
        assert_eq!(app.ui.status_notes, Some((app.ui.status.clone(), notes)));

        app.run("file.save", json!({"path": "/out/b.vectorcraft"})).unwrap();
        assert_eq!((app.ui.status.as_str(), &app.ui.status_notes), ("Saved /out/b.vectorcraft", &None));

        // A PDF export of a knockout group whose soft masks would draw a brushed stroke, and of type
        // in a font that isn't installed.
        app.run("text.create", json!({"x": 5, "y": 20, "text": "Ab", "font": "Nonexistent Sans Probe"})).unwrap();
        let lower = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 30, "height": 20})).unwrap()["id"].clone();
        let upper = app.run("shape.rectangle", json!({"x": 25, "y": 15, "width": 30, "height": 20})).unwrap()["id"].clone();
        app.run("brush.apply", json!({"ids": [upper], "name": "3 pt. Round"})).unwrap();
        app.run("object.setProps", json!({"ids": [lower, upper], "opacity": 50, "knockoutShape": true})).unwrap();
        app.run("select.set", json!({"ids": [lower, upper]})).unwrap();
        app.run("object.group", json!({})).unwrap();
        app.run("transparency.set", json!({"knockout": "on"})).unwrap();
        let r = export(&mut app, Some("pdf"), Some("/out/c.pdf".into()), &json!({})).unwrap();
        let notes = notes_of(&r);
        assert_eq!(notes.len(), 2, "{r}");
        assert!(notes.iter().any(|n| n.contains("brushed strokes")), "{r}");
        assert_eq!(app.ui.status, exported_with_notes("/out/c.pdf", notes.len(), &notes[0]));
        assert_eq!(app.ui.status_notes, Some((app.ui.status.clone(), notes)));
    }

    /// An export whose encoder notes what an option left out (the fill hatches that
    /// `includeHatches: false` leaves out of a DXF drawing) and what the file loses (a see-through
    /// rectangle that DXF 2000 writes opaque): the status bar names what the file loses, and the
    /// hover lists the option's note last.
    #[test]
    fn an_export_names_what_the_file_loses_before_a_note_on_the_options() {
        let (mut app, _) = app();
        app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
        app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 30, "height": 20})).unwrap();
        app.run("object.setProps", json!({"opacity": 50})).unwrap();
        let r = export(&mut app, Some("dxf"), Some("/out/d.dxf".into()), &json!({"version": "2000", "includeHatches": false})).unwrap();
        let notes = notes_of(&r);
        let Some((option, losses)) = notes.split_last() else { panic!("no notes: {r}") };
        assert!(option.starts_with("fill hatches are left out"), "{r}");
        assert!(losses.iter().any(|n| n.starts_with("transparency needs DXF 2004")), "{r}");
        assert!(!losses.iter().any(|n| n.starts_with("fill hatches")), "{r}");
        assert_eq!(app.ui.status, exported_with_notes("/out/d.dxf", notes.len(), &notes[0]));
        assert_eq!(app.ui.status_notes, Some((app.ui.status.clone(), notes)));
    }

    /// #1685: a save or an export that can't write its file says so in a dialog that names the
    /// file and the reason and stays until it is closed; the document stays modified and keeps
    /// the file it had. An agent's request only gets the error, and a message that arrives while
    /// another dialog is open waits for it to close.
    #[test]
    fn a_save_that_cannot_write_is_reported_in_a_dialog_that_stays() {
        let refuse: crate::WriteFn = Box::new(|p: &str, _: &[u8]| Err(format!("{p}: Access is denied. (os error 5)")));
        let mut app = windowed(Services { write: Some(refuse), ..Default::default() });
        app.run("file.new", json!({"width": 100, "height": 80})).unwrap();
        app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 30, "height": 20})).unwrap();
        let unsaved = |app: &VectorcraftApp| {
            let st = app.session.doc().unwrap();
            (st.is_dirty(), st.path.clone())
        };
        for (command, path, message) in [
            ("file.save", "/docs/Poster.vectorcraft", "Can't save “Poster.vectorcraft”."),
            ("file.saveAs", "/docs/Poster.vectorcraft", "Can't save “Poster.vectorcraft”."),
            ("file.saveCopy", "/docs/Copy.vectorcraft", "Can't save “Copy.vectorcraft”."),
            ("file.exportAs", "/docs/Poster.png", "Can't export “Poster.png”."),
            ("file.export.pdf", "/docs/Poster.pdf", "Can't save “Poster.pdf”."),
        ] {
            let e = app.run(command, json!({"path": path})).unwrap_err();
            assert!(e.contains("Access is denied"), "{command}: {e}");
            let (told_message, detail) = told(&mut app).unwrap_or_else(|| panic!("{command}: a dialog says so"));
            assert_eq!(told_message, message, "{command}");
            assert_eq!(detail, format!("{path}: Access is denied. (os error 5)"), "{command}");
            assert_eq!(unsaved(&app), (true, None), "{command}");
        }
        // An agent waiting on its request gets the error as its reply, and no dialog.
        app.ui.answering_control = true;
        assert!(app.run("file.save", json!({"path": "/docs/Poster.vectorcraft"})).is_err());
        app.ui.answering_control = false;
        assert!(open_dialogs(&app).is_empty() && app.told.0.is_empty());
        // Under another dialog (a background save that fails later) the message waits its turn,
        // once, and is drawn when that dialog is closed.
        dialogs::confirm::ask(&mut app, "Delete the swatch?", "", "edit.undo", json!({}));
        for _ in 0..2 {
            report_write_error(&mut app, false, "/docs/Poster.vectorcraft", "the disk is full");
        }
        assert_eq!((open_dialogs(&app).is_empty(), app.told.0.len()), (false, 1));
        let ctx = app.ui_ctx.clone().expect("a window context");
        let question = open_dialogs(&app).last().map(|d| d.id).expect("the question is open");
        app.dialogs().expect("the window's dialog host").with(|h| h.cancel_by_id(&ctx, question));
        for _ in 0..2 {
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| crate::tests::picks::dialogs(&mut app, ui));
            out.textures_delta.clear();
        }
        assert_eq!(told(&mut app), Some(("Can't save “Poster.vectorcraft”.".to_string(), "the disk is full".to_string())), "the message is shown");
        assert!(app.told.0.is_empty());
    }

    /// #1693: a browser download that can't be started is a failed save: the document stays
    /// modified and without a file, and Save Changes doesn't close it.
    #[test]
    fn a_download_that_cannot_start_leaves_the_document_unsaved_and_open() {
        let started = Rc::new(RefCell::new(false));
        let ok = started.clone();
        let download: crate::DownloadFn =
            Box::new(move |_: &str, _: &[u8]| if *ok.borrow() { Ok(()) } else { Err("the browser refused the download".into()) });
        let mut app = windowed(Services { download: Some(download), ..Default::default() });
        app.run("file.new", json!({"width": 100, "height": 80})).unwrap();
        app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 30, "height": 20})).unwrap();
        let unsaved = |app: &VectorcraftApp| {
            let st = app.session.doc().unwrap();
            (st.is_dirty(), st.path.clone())
        };
        let e = save(&mut app, SaveMode::Save, &json!({"path": "Poster.vectorcraft"}), false).unwrap_err();
        assert!(e.contains("the browser refused the download"), "{e}");
        assert_eq!(unsaved(&app), (true, None));
        // Save Changes › Save: not saved, so not closed.
        let i = app.session.active_index().unwrap();
        app.session.doc_mut().unwrap().path = Some("Poster.vectorcraft".into());
        let ctx = app.ui_ctx.clone().expect("a window context");
        crate::unsaved::close(&mut app, i).unwrap();
        let question = open_dialogs(&app).last().map(|d| d.id).expect("Save Changes asks");
        crate::tests::picks::answer_dialog(&mut app, question, &[("choice", json!("save"))], true);
        crate::unsaved::pump(&mut app, &ctx);
        assert!(app.ui.status.contains("the browser refused the download"), "{}", app.ui.status);
        assert_eq!((app.session.documents().len(), app.session.doc().unwrap().is_dirty()), (1, true));
        // Once the download starts, the save counts.
        *started.borrow_mut() = true;
        save(&mut app, SaveMode::Save, &json!({}), false).unwrap();
        assert!(!app.session.doc().unwrap().is_dirty());
    }

    /// A `.ai` file saved from the app names the view's active artboard as its active one, in
    /// the editing copy other apps read.
    #[test]
    fn a_saved_ai_file_names_the_views_active_artboard() {
        let (mut app, written) = app();
        app.run("file.new", json!({"width": 60, "height": 40, "artboards": 3})).unwrap();
        app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
        app.sync_views();
        let opts = fileio::LoadOptions { native: false, ..Default::default() };
        for artboard in [2, 0] {
            if let Some(v) = app.view_mut() {
                v.artboard = artboard;
            }
            save(&mut app, SaveMode::Copy, &json!({"path": "Art.ai"}), false).unwrap();
            let bytes = written.borrow_mut().pop().map(|(_, b)| b).unwrap();
            let l = fileio::load_with("Art.ai", &bytes, &opts).unwrap();
            assert!(l.editing_data && !l.restored, "{:?}", l.warnings);
            let active = l.doc.opened_artboard.and_then(|id| l.doc.artboards.iter().position(|a| a.id == id)).unwrap_or(0);
            assert_eq!(active, artboard);
        }
    }

    /// A `.ai` file a command exports names the view's active artboard as its active one: an
    /// index past the last artboard names the last, as the Artboards panel shows it.
    #[test]
    fn an_exported_ai_file_names_the_views_active_artboard() {
        let (mut app, _) = app();
        app.run("file.new", json!({"width": 60, "height": 40, "artboards": 3})).unwrap();
        app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
        app.sync_views();
        let opts = fileio::LoadOptions { native: false, ..Default::default() };
        for (artboard, active) in [(1, 1), (99, 2)] {
            if let Some(v) = app.view_mut() {
                v.artboard = artboard;
            }
            let out = app.run("document.serialize", json!({"format": "ai"})).unwrap();
            let bytes = vectorcraft_format::base64_decode(out["dataBase64"].as_str().unwrap()).unwrap();
            let l = fileio::load_with("Art.ai", &bytes, &opts).unwrap();
            assert!(l.editing_data && !l.restored, "{:?}", l.warnings);
            assert_eq!(l.doc.opened_artboard.and_then(|id| l.doc.artboards.iter().position(|a| a.id == id)), Some(active), "{artboard}");
        }
    }

    /// A `.ai` file exported to a path (File › Export As, its dialog, a control request) names the
    /// view's active artboard as its active one, and the open document is unchanged.
    #[test]
    fn an_ai_file_exported_to_a_path_names_the_views_active_artboard() {
        let (mut app, written) = app();
        app.run("file.new", json!({"width": 60, "height": 40, "artboards": 3})).unwrap();
        app.sync_views();
        app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
        let opts = fileio::LoadOptions { native: false, ..Default::default() };
        let active = |app: &VectorcraftApp| {
            let bytes = written.borrow_mut().pop().map(|(_, b)| b).unwrap();
            let l = fileio::load_with("Art.ai", &bytes, &opts).unwrap();
            assert!(l.editing_data && !l.restored, "{:?}", l.warnings);
            assert_eq!(app.session.doc().unwrap().doc.opened_artboard, None, "the document is unchanged");
            l.doc.opened_artboard.and_then(|id| l.doc.artboards.iter().position(|a| a.id == id))
        };
        for artboard in [2, 1] {
            if let Some(v) = app.view_mut() {
                v.artboard = artboard;
            }
            app.run("file.exportAs", json!({"format": "ai", "path": "Art.ai"})).unwrap();
            assert_eq!(active(&app), Some(artboard), "File › Export As");
            export(&mut app, Some("ai"), Some("Art.ai".into()), &json!({})).unwrap();
            assert_eq!(active(&app), Some(artboard), "the Export As dialog");
        }
    }

    fn bytes_of(app: &mut VectorcraftApp, cmd: &str, p: Value) -> Vec<u8> {
        let v = app.session.execute(cmd, &p).unwrap();
        vectorcraft_format::base64_decode(v["dataBase64"].as_str().unwrap()).unwrap()
    }

    fn image_size(app: &VectorcraftApp, id: vectorcraft_doc::NodeId) -> (u32, u32) {
        match &app.session.doc().unwrap().doc.node(id).unwrap().kind {
            NodeKind::Image(im) => (im.width, im.height),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn webp_opens_and_places_at_its_pixel_size() {
        let (mut app, _) = app();
        app.session.execute("file.new", &json!({"width": 3, "height": 2})).unwrap();
        let webp = bytes_of(&mut app, "document.serialize", json!({"format": "webp"}));
        open_bytes(&mut app, "tiny.webp", &webp, None).unwrap();
        assert_eq!(app.session.documents().len(), 2);
        assert_eq!(app.views.len(), 2, "views follow the documents");
        let first = app.session.doc().unwrap().doc.layers[0].children().unwrap()[0].id;
        assert_eq!(image_size(&app, first), (3, 2));
        app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
        place_bytes(&mut app, "tiny.webp", &webp).unwrap();
        let placed = app.session.doc().unwrap().selection.objects[0];
        assert_eq!(image_size(&app, placed), (3, 2));
        assert!(place_bytes(&mut app, "x.xyz", b"hello").is_err());
    }

    #[test]
    fn templates_open_untitled() {
        let (mut app, _) = app();
        app.session.execute("file.new", &json!({"width": 50, "height": 50})).unwrap();
        let pdf = bytes_of(&mut app, "document.serialize", json!({"format": "pdf"}));
        open_bytes(&mut app, "brochure.ait", &pdf, Some("/tmp/brochure.ait".into())).unwrap();
        let st = app.session.active().unwrap();
        assert!(st.title().starts_with("Untitled-"), "{}", st.title());
        assert_eq!(st.path, None);
    }

    #[test]
    fn export_uses_the_engine_options_and_keeps_the_path() {
        let (mut app, written) = app();
        app.session.execute("file.new", &json!({"width": 100, "height": 50, "artboards": 2})).unwrap();
        app.session.execute("artboard.setProps", &json!({"index": 1, "width": 40})).unwrap();
        app.session.doc_mut().unwrap().path = Some("/tmp/doc.vectorcraft".into());
        export(&mut app, None, Some("/tmp/b.png".into()), &json!({"artboard": 1, "scale": 2})).unwrap();
        export(&mut app, None, Some("/tmp/copy.vectorcraft".into()), &Value::Null).unwrap();
        let w = written.borrow();
        assert_eq!(&w[0].1[16..20], 80u32.to_be_bytes(), "artboard 1 (40 pt) at scale 2");
        assert!(vectorcraft_format::sniff(&w[1].1));
        assert_eq!(app.session.active().unwrap().path.as_deref(), Some("/tmp/doc.vectorcraft"));
        drop(w);
        assert!(export(&mut app, None, Some("/tmp/x.dwg".into()), &Value::Null).is_err(), "DWG can't be written");
    }

    #[test]
    fn export_for_screens_with_params_runs_the_engine() {
        let (mut app, _) = app();
        app.session.execute("file.new", &json!({"width": 60, "height": 40, "artboards": 3})).unwrap();
        let r = app.run("file.exportForScreens", json!({"range": "2-3", "formats": [{"format": "pdf"}]})).unwrap();
        let files = r["files"].as_array().unwrap();
        assert_eq!(files.len(), 2);
        assert_eq!(files[0]["name"], "Artboard-2.pdf");
        assert!(open_dialogs(&app).is_empty());
        app.run("file.exportForScreens", Value::Null).unwrap();
        assert!(open_dialogs(&app).iter().any(|d| d.key.as_deref() == Some("exportForScreens")));
    }

    #[test]
    fn export_selection_takes_its_format_from_the_picked_path() {
        let (mut app, written) = app();
        app.session.execute("file.new", &json!({"width": 60, "height": 40})).unwrap();
        app.session.execute("shape.rectangle", &json!({"x": 5, "y": 5, "width": 20, "height": 10})).unwrap();
        let picker = scripted(&mut app);
        let ctx = app.ui_ctx.clone().expect("a window context");
        let r = app.run("document.exportSelection", json!({})).unwrap();
        assert_eq!(r["pending"], "fileDialog", "the format comes from the picked path: the panel asks first");
        picker.answer_saved("/tmp/sel.svg", None);
        poll(&mut app, &ctx);
        let options = open_dialogs(&app).into_iter().rfind(|d| d.key.as_deref() == Some("svgOptions")).expect("the picked format's options follow");
        crate::tests::picks::answer_dialog(&mut app, options.id, &[], true);
        assert_eq!(written.borrow()[0].0, "/tmp/sel.svg");
        app.run("document.exportSelection", json!({"path": "/tmp/sel.png"})).unwrap();
        let w = written.borrow();
        assert!(String::from_utf8_lossy(&w[0].1).contains("<svg"), "SVG from the picked name");
        assert_eq!(&w[1].1[1..4], b"PNG", "PNG from the given path");
    }

    /// An option skips the options dialog, but without `path` the save panel still picks the file.
    #[test]
    fn export_selection_with_an_option_and_no_path_asks_for_the_file_only() {
        let (mut app, written) = app();
        app.session.execute("file.new", &json!({"width": 60, "height": 40})).unwrap();
        app.session.execute("shape.rectangle", &json!({"x": 5, "y": 5, "width": 20, "height": 10})).unwrap();
        let picker = scripted(&mut app);
        let ctx = app.ui_ctx.clone().expect("a window context");
        let r = app.run("document.exportSelection", json!({"scale": 2})).unwrap();
        assert_eq!(r["pending"], "fileDialog");
        assert_eq!(picker.requests().len(), 1, "the save panel picks the file");
        picker.answer_saved("/tmp/sel.png", None);
        poll(&mut app, &ctx);
        assert!(open_dialogs(&app).is_empty(), "no options dialog follows");
        assert_eq!(&written.borrow()[0].1[1..4], b"PNG");
        app.run("document.exportSelection", json!({"path": "/tmp/sel2.png", "scale": 2})).unwrap();
        assert_eq!(picker.requests().len(), 1, "a path writes without the save panel");
        let w = written.borrow();
        assert_eq!(w.len(), 2);
        assert_eq!(w[0].0, "/tmp/sel.png", "the picked file");
        assert_eq!(w[1].0, "/tmp/sel2.png", "the given path");
    }

    #[test]
    fn written_files_open_as_file_urls() {
        let url = super::file_url("/tmp/My Art #1.pdf");
        assert!(url.starts_with("file:///") && url.ends_with("/tmp/My%20Art%20%231.pdf"), "{url}");
        assert!(!url.contains('\\'), "{url}");
    }

    #[test]
    fn control_export_without_path_returns_bytes() {
        let (mut app, written) = app();
        app.session.execute("file.new", &json!({"width": 30, "height": 20})).unwrap();
        let (req, _) = craft::automation::control::ControlRequest::new("app.export", json!({"format": "png", "scale": 2}));
        let craft::automation::Reply::Done(r) = crate::control::handle(&mut app, &crate::testing::ui_context(), &req) else { panic!("not done") };
        assert_eq!(r["ok"], true, "{r}");
        let png = vectorcraft_format::base64_decode(r["result"]["dataBase64"].as_str().unwrap()).unwrap();
        assert_eq!(&png[16..20], 60u32.to_be_bytes(), "30 pt at scale 2");
        assert!(written.borrow().is_empty(), "no file and no save dialog");
    }

    /// A presets file, a Libraries panel file or a graphic style library that is opened or dropped
    /// goes to its importer as the file's bytes: a byte that is not UTF-8 is refused instead of
    /// read as a replacement character.
    #[test]
    fn opened_preset_and_library_files_keep_their_bytes() {
        let (mut app, _) = app();
        app.run("file.new", json!({"width": 60, "height": 40})).unwrap();
        app.run("document.exportPresets.save", json!({"name": "Pack", "settings": {"formats": [{"format": "svg"}]}})).unwrap();
        let presets = app.run("document.exportPresets.export", json!({"names": ["Pack"]})).unwrap()["data"].as_str().unwrap().to_string();
        let lib = app.run("library.create", json!({"name": "Brand"})).unwrap()["id"].as_str().unwrap().to_string();
        let library = app.run("library.export", json!({ "library": lib })).unwrap()["data"].as_str().unwrap().to_string();
        let styles = app.run("graphicStyle.saveLibrary", json!({"name": "Kit"})).unwrap()["data"].as_str().unwrap().to_string();
        for (file, text, name) in [("pack.vcexportpresets", presets, "Pack"), ("brand.vclibrary", library, "Brand"), ("kit.vcstyles", styles, "Kit")]
        {
            let mut bytes = text.as_bytes().to_vec();
            bytes.insert(text.find(name).unwrap() + 2, 0xFF);
            assert!(open_bytes(&mut app, file, &bytes, None).is_err(), "{file}: {}", app.ui.status);
            // The file as it was still opens.
            open_bytes(&mut app, file, text.as_bytes(), None).unwrap();
        }
        assert_eq!(app.session.prefs.screen_export_presets.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["Pack", "Pack 2"]);
    }
}
