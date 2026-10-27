//! File I/O through the injected services (pick, read, write, download, reveal); the engine's
//! `fileio` decodes, encodes and saves every format.

use serde_json::{Value, json};
use vectorcraft_doc::SavedView;
use vectorcraft_doc::cancel;
use vectorcraft_engine::EngineError;
use vectorcraft_engine::cmd::fileio::progress::{self, Progress};
use vectorcraft_engine::cmd::fileio::{self, Format, SAVE_FORMATS, SaveMode, SavePlan, TEMPLATE_FORMATS};

use crate::background::{self, Writer};
use crate::dialogs::svg_options;
use crate::state::Dialog;
use crate::{FilePick, Services, VectorcraftApp, dialogs};

mod export;
mod open;
mod save;

pub use export::{EXPORT_CANCELLED, cancel_exports, export, export_for_screens, export_pdf, hide_exports, reveal, save_command_output};
pub(crate) use export::{export_files, file_url, open_in_app, reveal_path, run_to_file, save_command_output_named};
#[cfg(test)]
pub(crate) use open::open_file;
pub(crate) use open::opened;
pub use open::{
    MAX_RECENT_FILES, OpensAs, ask_revert, install_module, install_plugin, new_from_template, note_recent, open_bytes, open_dialog, open_document,
    open_path, open_recent_reporting, open_reporting, opens_as, place_bytes, recent_files, report_open_error,
};
#[cfg(test)]
pub(crate) use save::report_write_error;
pub use save::{ask_format_options, ask_pdf_options, remember_view, save, save_pdf};
pub(crate) use save::{export_path, is_web, report_saved, target_path, write_named, write_reporting, write_to};

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use serde_json::json;
    use vectorcraft_engine::Session;

    use super::*;
    use crate::Services;
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
        (VectorcraftApp::new(Session::new(), services), written)
    }

    /// #861: a file the user opens that can't be opened is reported in a dialog they close, not
    /// only in the status bar; a file an agent opens by path only answers with the error.
    #[test]
    fn a_file_the_user_cant_open_is_reported_in_a_dialog() {
        let (mut app, _) = app();
        app.services.read = Some(Box::new(|_: &str| Err("no such file".into())));
        assert!(open_reporting(&mut app, "/docs/gone.ai").is_err());
        let d = app.ui.dialog.take().expect("a dialog says so");
        assert_eq!(
            (d.kind.as_str(), d.str("message"), d.str("detail")),
            (dialogs::confirm::MESSAGE, "Can't open “gone.ai”.".to_string(), "no such file".to_string())
        );
        assert!(app.ui.status.contains("gone.ai"));
        // An agent's `file.open {path}` gets the error back and no dialog.
        assert!(app.run("file.open", json!({"path": "/docs/gone.ai"})).is_err());
        assert!(app.ui.dialog.is_none());
    }

    /// #1685: a save or an export that can't write its file says so in a dialog that names the
    /// file and the reason and stays until it is closed; the document stays modified and keeps
    /// the file it had. An agent's request only gets the error, and a message that arrives while
    /// another dialog is open waits for it to close.
    #[test]
    fn a_save_that_cannot_write_is_reported_in_a_dialog_that_stays() {
        let refuse: crate::WriteFn = Box::new(|p: &str, _: &[u8]| Err(format!("{p}: Access is denied. (os error 5)")));
        let mut app = VectorcraftApp::new(Session::new(), Services { write: Some(refuse), ..Default::default() });
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
            let d = app.ui.dialog.take().unwrap_or_else(|| panic!("{command}: a dialog says so"));
            assert_eq!((d.kind.as_str(), d.str("message").as_str()), (dialogs::confirm::MESSAGE, message), "{command}");
            assert_eq!(d.str("detail"), format!("{path}: Access is denied. (os error 5)"), "{command}");
            assert_eq!(unsaved(&app), (true, None), "{command}");
        }
        // An agent waiting on its request gets the error as its reply, and no dialog.
        app.ui.answering_control = true;
        assert!(app.run("file.save", json!({"path": "/docs/Poster.vectorcraft"})).is_err());
        app.ui.answering_control = false;
        assert!(app.ui.dialog.is_none() && app.ui.told.is_empty());
        // Under another dialog (a background save that fails later) the message waits its turn,
        // once, and is drawn when that dialog is closed.
        dialogs::confirm::ask(&mut app, "Delete the swatch?", "", "edit.undo", json!({}));
        for _ in 0..2 {
            report_write_error(&mut app, false, "/docs/Poster.vectorcraft", "the disk is full");
        }
        assert_eq!((app.ui.dialog.as_ref().map(|d| d.kind.as_str()), app.ui.told.len()), (Some(dialogs::confirm::KIND), 1));
        app.ui.dialog = None;
        for _ in 0..2 {
            let ctx = egui::Context::default();
            crate::theme::install_fonts(&ctx);
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| dialogs::show(&mut app, ui.ctx()));
            out.textures_delta.clear();
        }
        let d = app.ui.dialog.as_ref().expect("the message is shown, and stays");
        assert_eq!(
            (d.kind.as_str(), d.str("message").as_str(), d.str("detail").as_str()),
            (dialogs::confirm::MESSAGE, "Can't save “Poster.vectorcraft”.", "the disk is full")
        );
        assert!(app.ui.told.is_empty());
    }

    /// #1693: a browser download that can't be started is a failed save: the document stays
    /// modified and without a file, and Save Changes doesn't close it.
    #[test]
    fn a_download_that_cannot_start_leaves_the_document_unsaved_and_open() {
        let started = Rc::new(RefCell::new(false));
        let ok = started.clone();
        let download: crate::DownloadFn =
            Box::new(move |_: &str, _: &[u8]| if *ok.borrow() { Ok(()) } else { Err("the browser refused the download".into()) });
        let mut app = VectorcraftApp::new(Session::new(), Services { download: Some(download), ..Default::default() });
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
        let (i, uid) = (app.session.active_index().unwrap(), app.session.doc().unwrap().uid);
        app.ui.dialog = Some(crate::state::Dialog::new(crate::unsaved::KIND, json!({"index": i, "uid": uid, "name": "Poster", "then": "close"})));
        app.session.doc_mut().unwrap().path = Some("Poster.vectorcraft".into());
        assert!(crate::unsaved::confirm(&mut app).is_err());
        assert_eq!((app.session.documents().len(), app.session.doc().unwrap().is_dirty()), (1, true));
        // Once the download starts, the save counts.
        *started.borrow_mut() = true;
        save(&mut app, SaveMode::Save, &json!({}), false).unwrap();
        assert!(!app.session.doc().unwrap().is_dirty());
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
        assert!(app.ui.dialog.is_none());
        app.run("file.exportForScreens", Value::Null).unwrap();
        assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some("exportForScreens"));
    }

    #[test]
    fn export_selection_takes_its_format_from_the_picked_path() {
        let (mut app, written) = app();
        app.session.execute("file.new", &json!({"width": 60, "height": 40})).unwrap();
        app.session.execute("shape.rectangle", &json!({"x": 5, "y": 5, "width": 20, "height": 10})).unwrap();
        app.services.pick_save = Some(Box::new(|_: &FilePick| Some("/tmp/sel.svg".into())));
        assert_eq!(app.run("document.exportSelection", json!({})).unwrap()["path"], "/tmp/sel.svg");
        app.run("document.exportSelection", json!({"path": "/tmp/sel.png"})).unwrap();
        let w = written.borrow();
        assert!(String::from_utf8_lossy(&w[0].1).contains("<svg"), "SVG from the picked name");
        assert_eq!(&w[1].1[1..4], b"PNG", "PNG from the given path");
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
        let (req, _) = crate::control::ControlRequest::new("app.export", json!({"format": "png", "scale": 2}));
        let crate::control::Outcome::Done(r) = crate::control::handle(&mut app, &egui::Context::default(), &req) else { panic!("not done") };
        assert_eq!(r["ok"], true, "{r}");
        let png = vectorcraft_format::base64_decode(r["result"]["dataBase64"].as_str().unwrap()).unwrap();
        assert_eq!(&png[16..20], 60u32.to_be_bytes(), "30 pt at scale 2");
        assert!(written.borrow().is_empty(), "no file and no save dialog");
    }
}
