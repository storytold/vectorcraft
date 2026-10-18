//! Opening documents off the UI thread (desktop): a file the user asks to open (File › Open, Open
//! Recent, the Home screen, files handed to the app) is decoded on a thread of its own, which also
//! reads its linked files and lays out its type, while the status bar shows it, and opens in a new tab
//! once it is ready, as it would at once: the same notes, recent files and errors. On wasm (no
//! threads) and in unit tests it opens at once ([`available`]).

use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError, channel};

use serde_json::{Value, json};
use vectorcraft_engine::cmd::fileio::{self, ReadFile};
use vectorcraft_engine::file_access::ScopeHandle;

use crate::VectorcraftApp;

/// A file being read.
pub struct Pending {
    /// Where it is.
    pub path: String,
    /// What the status bar shows ("Opening Poster.vectorcraft").
    pub label: String,
    done: Receiver<Result<ReadFile, String>>,
}

/// The files being read.
#[derive(Default)]
pub struct Opening {
    pub pending: Vec<Pending>,
}

/// Is a long operation running that should show the busy cursor and status spinner (background
/// save/export with a label, or a file opening off the UI thread)?
pub fn long_operation_busy(app: &VectorcraftApp) -> bool {
    app.background.jobs.iter().any(|j| !j.label.is_empty() && !j.cancelled()) || !app.opening.pending.is_empty()
}

/// Does opening happen off the UI thread here? Not on wasm (no threads), in unit tests (they look
/// at the document right after asking for it), or with `VECTORCRAFT_SYNC_OPEN` set.
pub fn available() -> bool {
    !(cfg!(target_arch = "wasm32") || cfg!(test) || std::env::var_os("VECTORCRAFT_SYNC_OPEN").is_some())
}

/// Decode `bytes`, the file at `path`, read its linked files and lay out its type on a thread of its
/// own ([`fileio::read_for_open`]) → `{background: true, status}`, or the bytes back when no thread
/// could start (the caller opens the file at once).
pub fn start(app: &mut VectorcraftApp, path: &str, bytes: Vec<u8>) -> Result<Value, Vec<u8>> {
    let (tx, done) = channel();
    let bytes = Arc::new(bytes);
    let (name, file) = (path.to_string(), bytes.clone());
    // Update Links (Preferences › File Handling) applies there as it is set now.
    let update_links = app.session.prefs.update_links == "automatically";
    // The automation roots in force here are in force there.
    let scope = ScopeHandle::capture();
    let spawned = std::thread::Builder::new().name("vectorcraft-open".into()).spawn(move || {
        let r = scope
            .run(|| {
                vectorcraft_engine::guard::catch_panic(|| {
                    fileio::read_for_open(&name, &file, Some(name.clone()), &Value::Null, update_links).map_err(|e| e.to_string())
                })
            })
            .unwrap_or_else(|msg| Err(format!("internal error: {msg} (please report this bug)")));
        // The file's bytes go before the document is handed over.
        drop(file);
        // The app may have quit meanwhile: nothing to hand it to.
        let _ = tx.send(r);
    });
    if spawned.is_err() {
        return Err(Arc::try_unwrap(bytes).unwrap_or_else(|b| b.to_vec()));
    }
    let label = format!("Opening {}", fileio::file_name(path));
    app.opening.pending.push(Pending { path: path.to_string(), label: label.clone(), done });
    let status = format!("{label}…");
    app.status(status.clone());
    Ok(json!({ "background": true, "status": status }))
}

/// Open the files that are ready (each frame). Not while a dialog is open: a document that opens
/// becomes the active one, and the dialog's OK acts on the active document (#1694: Save As
/// options confirmed after another file had finished opening saved that file's art under the
/// first one's name). The files wait, their status line too, until the dialog is closed.
pub fn poll(app: &mut VectorcraftApp) {
    if app.ui.dialog.is_some() {
        return;
    }
    let mut i = 0;
    while let Some(p) = app.opening.pending.get(i) {
        let r = match p.done.try_recv() {
            Ok(r) => r,
            Err(TryRecvError::Empty) => {
                i += 1;
                continue;
            }
            Err(TryRecvError::Disconnected) => Err("the file stopped being read (please report this bug)".to_string()),
        };
        let p = app.opening.pending.remove(i);
        // The status bar no longer says it is opening.
        if app.ui.status == format!("{}…", p.label) {
            app.ui.status.clear();
        }
        finish(app, &p.path, r);
    }
}

/// Open a file read off the UI thread, as [`crate::io::open_reporting`] opens one read at once.
fn finish(app: &mut VectorcraftApp, path: &str, r: Result<ReadFile, String>) {
    let opened = r.and_then(|file| fileio::open_read(&mut app.session, file).map_err(|e| e.to_string()));
    match opened {
        Ok(r) => {
            crate::io::opened(app, &r);
            crate::io::note_recent(app, path);
        }
        Err(e) => crate::io::report_open_error(app, &fileio::file_name(path), &e),
    }
}

#[cfg(test)]
mod tests {
    use vectorcraft_engine::Session;

    use super::*;
    use crate::Services;

    #[test]
    fn long_operation_busy_while_a_file_opens_or_a_background_save_runs() {
        let mut app = VectorcraftApp::new(Session::new(), Services::default());
        assert!(!long_operation_busy(&app));
        start(&mut app, "/art/Wait.vectorcraft", vectorcraft_format::save(&vectorcraft_doc::Document::new(10.0, 10.0), false)).unwrap();
        assert!(long_operation_busy(&app));
        let t0 = std::time::Instant::now();
        while long_operation_busy(&app) && t0.elapsed().as_secs() < 30 {
            poll(&mut app);
            std::thread::yield_now();
        }
        assert!(!long_operation_busy(&app));
        let (release, held) = std::sync::mpsc::channel::<()>();
        let services = Services { write_shared: Some(std::sync::Arc::new(|_: &str, _: &[u8]| Ok(()))), ..Default::default() };
        app = VectorcraftApp::new(Session::new(), services);
        crate::background::run(
            &mut app,
            true,
            "Saving test.vectorcraft".into(),
            None,
            move |_| {
                let _ = held.recv_timeout(std::time::Duration::from_secs(10));
                Ok(json!({}))
            },
            |_, r| r,
        )
        .unwrap();
        assert!(long_operation_busy(&app));
        let _ = release.send(());
        let t0 = std::time::Instant::now();
        while long_operation_busy(&app) && t0.elapsed().as_secs() < 30 {
            crate::background::poll(&mut app);
            std::thread::yield_now();
        }
        assert!(!long_operation_busy(&app));
    }

    /// #1694: a file that finishes opening while a dialog is up waits for it to close, so the
    /// dialog's document stays the active one.
    #[test]
    fn a_file_ready_while_a_dialog_is_open_waits_for_it_to_close() {
        let mut app = VectorcraftApp::new(Session::new(), Services::default());
        app.run("file.new", json!({"width": 100, "height": 80})).unwrap();
        let (first, before) = (app.session.doc().unwrap().uid, app.session.documents().len());
        app.ui.dialog = Some(crate::state::Dialog::new(crate::dialogs::save_options::KIND, json!({"path": "/art/First.vectorcraft"})));
        start(&mut app, "/art/Second.vectorcraft", vectorcraft_format::save(&vectorcraft_doc::Document::new(120.0, 80.0), false)).unwrap();
        let t0 = std::time::Instant::now();
        while t0.elapsed().as_millis() < 400 {
            poll(&mut app);
            std::thread::yield_now();
        }
        assert_eq!((app.session.documents().len(), app.session.doc().unwrap().uid), (before, first), "still the dialog's document");
        assert_eq!(app.opening.pending.len(), 1);
        app.ui.dialog = None;
        let t0 = std::time::Instant::now();
        while !app.opening.pending.is_empty() && t0.elapsed().as_secs() < 30 {
            poll(&mut app);
            std::thread::yield_now();
        }
        assert_eq!(app.session.documents().len(), before + 1);
        assert_eq!(app.session.doc().unwrap().path.as_deref(), Some("/art/Second.vectorcraft"));
    }

    #[test]
    fn a_file_read_off_the_ui_thread_opens_as_one_read_at_once() {
        let mut app = VectorcraftApp::new(Session::new(), Services::default());
        let mut d = vectorcraft_doc::Document::new(120.0, 80.0);
        d.title = "Poster".into();
        let bytes = vectorcraft_format::save(&d, false);
        let before = app.session.documents().len();
        let r = start(&mut app, "/art/Poster.vectorcraft", bytes).unwrap();
        assert_eq!(r["background"], true);
        assert_eq!(app.ui.status, "Opening Poster.vectorcraft…");
        let t0 = std::time::Instant::now();
        while !app.opening.pending.is_empty() && t0.elapsed().as_secs() < 30 {
            poll(&mut app);
            std::thread::yield_now();
        }
        assert_eq!(app.session.documents().len(), before + 1);
        assert!(!app.ui.status.starts_with("Opening"), "{}", app.ui.status);
        let st = app.session.doc().unwrap();
        assert_eq!((st.path.as_deref(), st.doc.artboards[0].rect.width()), (Some("/art/Poster.vectorcraft"), 120.0));
        assert_eq!(app.ui.recent_files.first().map(String::as_str), Some("/art/Poster.vectorcraft"));
        // A file that can't be read: reported, nothing opened, not a recent file.
        start(&mut app, "/art/Broken.vectorcraft", b"{\"format\":\"vectorcraft\"".to_vec()).unwrap();
        let t0 = std::time::Instant::now();
        while !app.opening.pending.is_empty() && t0.elapsed().as_secs() < 30 {
            poll(&mut app);
            std::thread::yield_now();
        }
        assert_eq!(app.session.documents().len(), before + 1);
        assert!(app.ui.status.starts_with("Couldn't open Broken.vectorcraft"), "{}", app.ui.status);
        assert_ne!(app.ui.recent_files.first().map(String::as_str), Some("/art/Broken.vectorcraft"));
    }

    /// A file read off the UI thread opens as the document read at once. Its linked TIFF, BMP and JPEG
    /// files are found beside it after its folder moved, a missing one shows its preview, a changed
    /// one follows Update Links, and its Auto Size area type in two columns is fitted. Neither open
    /// leaves a step to undo, and both ask the same question.
    #[test]
    fn a_file_read_off_the_ui_thread_has_its_links_and_type_as_one_read_at_once() {
        use image::ImageFormat::{Bmp, Jpeg, Png, Tiff};
        let dir = std::env::temp_dir().join(format!("vc-opening-links-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let (before, after) = (dir.join("before"), dir.join("after"));
        std::fs::create_dir_all(&before).unwrap();
        let file = |folder: &std::path::Path, name: &str| folder.join(name).to_string_lossy().into_owned();
        let picture = |format: image::ImageFormat, rgb: [u8; 3]| {
            let mut out = vec![];
            image::RgbImage::from_pixel(60, 40, image::Rgb(rgb)).write_to(&mut std::io::Cursor::new(&mut out), format).unwrap();
            out
        };
        let mut app = VectorcraftApp::new(Session::new(), Services::default());
        app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
        for (name, format) in [("a.tif", Tiff), ("b.bmp", Bmp), ("c.jpg", Jpeg), ("gone.png", Png), ("changed.png", Png)] {
            std::fs::write(file(&before, name), picture(format, [200, 40, 40])).unwrap();
            app.session.execute("file.place", &json!({"path": file(&before, name), "at": [200, 150]})).unwrap();
        }
        let area = json!({"width": 150, "height": 20, "fit": "autoHeight"});
        let t = app.session.execute("text.create", &json!({"x": 20, "y": 40, "text": "Type in two columns. ".repeat(12), "area": area})).unwrap();
        app.session.execute("text.areaOptions", &json!({"ids": [t["id"]], "columns": 2})).unwrap();
        app.session.execute("document.save", &json!({"path": file(&before, "poster.vectorcraft")})).unwrap();
        std::fs::remove_file(file(&before, "gone.png")).unwrap();
        let changed = file(&before, "changed.png");
        let stamp = std::fs::metadata(&changed).unwrap().modified().unwrap();
        std::fs::write(&changed, picture(Png, [40, 40, 200])).unwrap();
        // Link stamps use milliseconds: the new file is dated 2 s later.
        let later = std::fs::FileTimes::new().set_modified(stamp + std::time::Duration::from_secs(2));
        std::fs::OpenOptions::new().write(true).open(&changed).unwrap().set_times(later).unwrap();
        // The folder moves: the linked files are found beside the document.
        std::fs::rename(&before, &after).unwrap();
        let path = file(&after, "poster.vectorcraft");
        let bytes = std::fs::read(&path).unwrap();
        let asked = |app: &VectorcraftApp| app.ui.dialog.as_ref().map(|d| (d.kind.clone(), d.fields.clone()));
        for update in ["askWhenModified", "automatically"] {
            app.session.prefs.update_links = update.into();
            let r = crate::io::open_bytes(&mut app, &path, &bytes, Some(path.clone())).unwrap();
            let rows = if update == "automatically" { &r["updatedLinks"] } else { &r["modifiedLinks"] };
            assert_eq!(rows[0]["name"], "changed.png", "{update}: {r}");
            let (at_once, question) = (app.session.doc().unwrap().doc.clone(), asked(&app));
            assert!(question.as_ref().is_some_and(|(_, f)| f["missing"][0]["name"] == "gone.png"), "{update}: {question:?}");
            app.ui.dialog = None;
            start(&mut app, &path, bytes.clone()).unwrap();
            let t0 = std::time::Instant::now();
            while !app.opening.pending.is_empty() && t0.elapsed().as_secs() < 30 {
                poll(&mut app);
                std::thread::yield_now();
            }
            let st = app.session.doc().unwrap();
            assert!(*st.doc == *at_once, "{update}: the same document");
            assert!(st.history.undo.is_empty() && !st.is_dirty(), "{update}");
            assert_eq!(asked(&app), question, "{update}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
