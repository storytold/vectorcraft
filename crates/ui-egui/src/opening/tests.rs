use vectorcraft_engine::Session;

use super::*;
use crate::Services;

#[test]
fn long_operation_busy_while_a_file_opens_or_a_background_save_runs() {
    let mut app = VectorcraftApp::new(Session::new(), Services::default());
    assert!(!long_operation_busy(&app));
    start(&mut app, "/art/Wait.vectorcraft", vectorcraft_format::save(&vectorcraft_doc::Document::new(10.0, 10.0), false)).unwrap();
    assert!(long_operation_busy(&app));
    open_ready(&mut app);
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
            let _ = held.recv();
            Ok(json!({}))
        },
        |_, r| r,
    )
    .unwrap();
    assert!(long_operation_busy(&app));
    let _ = release.send(());
    crate::background::wait_all(&mut app);
    assert!(!long_operation_busy(&app));
}

/// Wait until every file being opened has been read, keeping each result for [`poll`].
fn wait_read(app: &mut VectorcraftApp) {
    for p in &mut app.opening.pending {
        let (tx, done) = std::sync::mpsc::channel();
        // A reader that stopped without a result leaves the new channel closed too.
        if let Ok(r) = p.done.recv() {
            let _ = tx.send(r);
        }
        p.done = done;
    }
}

/// Wait until every file being opened has been read, then open the files that may open now.
fn open_ready(app: &mut VectorcraftApp) {
    wait_read(app);
    poll(app);
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
    open_ready(&mut app);
    assert_eq!((app.session.documents().len(), app.session.doc().unwrap().uid), (before, first), "still the dialog's document");
    assert_eq!(app.opening.pending.len(), 1);
    app.ui.dialog = None;
    poll(&mut app);
    assert!(app.opening.pending.is_empty());
    assert_eq!(app.session.documents().len(), before + 1);
    assert_eq!(app.session.doc().unwrap().path.as_deref(), Some("/art/Second.vectorcraft"));
}

/// A file that finishes opening while an object is being dragged waits for the drag to end, so
/// the drag is not canceled.
#[test]
fn a_file_ready_during_a_drag_waits_for_it_to_end() {
    use vectorcraft_tools::{PointerEvent, PointerKind};
    let mut app = VectorcraftApp::new(Session::new(), Services::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    let square = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap();
    app.select_tool("selection");
    let view = vectorcraft_engine::ViewInfo::default();
    for (kind, x) in [(PointerKind::Down, 20.0), (PointerKind::Drag, 40.0), (PointerKind::Drag, 60.0)] {
        app.session.pointer(&PointerEvent::new(kind, x, 20.0), view).unwrap();
    }
    start(&mut app, "/art/Second.vectorcraft", vectorcraft_format::save(&vectorcraft_doc::Document::new(120.0, 80.0), false)).unwrap();
    open_ready(&mut app);
    assert!(app.session.documents().len() == 1 && app.session.in_interaction(), "the file waits for the drag");
    app.session.pointer(&PointerEvent::new(PointerKind::Up, 60.0, 20.0), view).unwrap();
    poll(&mut app);
    assert_eq!(app.session.documents().len(), 2);
    let moved = app.session.documents()[0].doc.node(vectorcraft_doc::NodeId(square)).and_then(|n| n.geometric_bounds());
    assert_eq!(moved.map(|b| (b.x0, b.y0)), Some((50.0, 10.0)), "the drag was not canceled");
}

/// A file that finishes opening while the place cursor is being dragged, a drag the tool keeps
/// to itself, waits for the drag to end: the drag still places its file.
#[test]
fn a_file_ready_during_a_place_cursor_drag_waits_for_it_to_end() {
    use vectorcraft_tools::{PointerEvent, PointerKind};
    let mut app = VectorcraftApp::new(Session::new(), Services::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    let mut png = vec![];
    image::RgbImage::new(8, 8).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    app.run("file.place.queue", json!({"files": [{"name": "a.png", "dataBase64": vectorcraft_format::base64_encode(&png)}]})).unwrap();
    let view = vectorcraft_engine::ViewInfo::default();
    for (kind, x) in [(PointerKind::Down, 20.0), (PointerKind::Drag, 60.0)] {
        crate::canvas::dispatch(&mut app, &PointerEvent::new(kind, x, 20.0), view);
    }
    start(&mut app, "/art/Second.vectorcraft", vectorcraft_format::save(&vectorcraft_doc::Document::new(120.0, 80.0), false)).unwrap();
    open_ready(&mut app);
    assert!(app.session.documents().len() == 1 && app.session.tool_busy(), "the file waits for the drag");
    crate::canvas::dispatch(&mut app, &PointerEvent::new(PointerKind::Up, 60.0, 20.0), view);
    poll(&mut app);
    assert_eq!(app.session.documents().len(), 2);
    let first = &app.session.documents()[0].doc;
    let names: Vec<_> = first.layers.iter().flat_map(|l| l.children().into_iter().flatten()).filter_map(|n| n.name.clone()).collect();
    assert_eq!(names, ["a.png"], "the drag placed its file");
}

/// A file that finishes opening while a preview runs waits for the preview to end, so the
/// preview is not canceled: a slider's preview with the tool idle, a slider's preview while the
/// Dimension tool waits after its first point, and the Dimension tool's line following the
/// pointer after its second point.
#[test]
fn a_file_ready_during_a_preview_waits_for_it_to_end() {
    use vectorcraft_tools::{PointerEvent, PointerKind};
    let view = vectorcraft_engine::ViewInfo::default();
    let click = |app: &mut VectorcraftApp, x: f64| {
        for kind in [PointerKind::Down, PointerKind::Up] {
            crate::canvas::dispatch(app, &PointerEvent::new(kind, x, 20.0), view);
        }
    };
    for case in ["slider", "slider between clicks", "line"] {
        let mut app = VectorcraftApp::new(Session::new(), Services::default());
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        if case != "slider" {
            app.select_tool("dimension");
            click(&mut app, 20.0);
        }
        if case == "line" {
            click(&mut app, 80.0);
            crate::canvas::dispatch(&mut app, &PointerEvent::new(PointerKind::Move, 50.0, 60.0), view);
        } else {
            app.session.begin_interaction("Preview").unwrap();
        }
        assert_eq!(app.session.tool_busy(), case != "slider", "{case}");
        start(&mut app, "/art/Second.vectorcraft", vectorcraft_format::save(&vectorcraft_doc::Document::new(120.0, 80.0), false)).unwrap();
        open_ready(&mut app);
        assert!(app.session.documents().len() == 1 && app.session.in_interaction(), "the file waits for the preview: {case}");
        if case == "line" {
            click(&mut app, 50.0);
        } else {
            app.session.cancel_interaction().unwrap();
        }
        poll(&mut app);
        assert_eq!(app.session.documents().len(), 2, "{case}");
    }
}

/// A file that finishes opening while a tool waits between its clicks with no preview open (the
/// Dimension tool after its first point, then after its second before the pointer moves) opens
/// without waiting for the next click, and the tool starts afresh in it, as when the user
/// switches documents.
#[test]
fn a_file_ready_between_a_tools_clicks_opens() {
    use vectorcraft_tools::{PointerEvent, PointerKind};
    let mut app = VectorcraftApp::new(Session::new(), Services::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.select_tool("dimension");
    let view = vectorcraft_engine::ViewInfo::default();
    let click = |app: &mut VectorcraftApp, x: f64| {
        for kind in [PointerKind::Down, PointerKind::Up] {
            crate::canvas::dispatch(app, &PointerEvent::new(kind, x, 20.0), view);
        }
    };
    click(&mut app, 20.0);
    assert!(app.session.tool_busy() && !app.session.in_interaction(), "the first point waits for the second");
    start(&mut app, "/art/Second.vectorcraft", vectorcraft_format::save(&vectorcraft_doc::Document::new(400.0, 300.0), false)).unwrap();
    open_ready(&mut app);
    assert_eq!(app.session.documents().len(), 2, "the file opens after one click");
    assert!(!app.session.tool_busy(), "the tool starts afresh");
    click(&mut app, 20.0);
    click(&mut app, 80.0);
    assert!(app.session.tool_busy() && !app.session.in_interaction(), "the second point waits for the line's place");
    start(&mut app, "/art/Third.vectorcraft", vectorcraft_format::save(&vectorcraft_doc::Document::new(120.0, 80.0), false)).unwrap();
    open_ready(&mut app);
    assert_eq!(app.session.documents().len(), 3, "the file opens after two clicks");
    assert!(!app.session.tool_busy() && !app.session.in_interaction(), "the tool starts afresh");
    let second = &app.session.documents()[1].doc;
    assert!(second.layers.iter().all(|l| l.children().is_none_or(|c| c.is_empty())), "the unfinished dimension is not made");
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
    open_ready(&mut app);
    assert_eq!(app.session.documents().len(), before + 1);
    assert!(!app.ui.status.starts_with("Opening"), "{}", app.ui.status);
    let st = app.session.doc().unwrap();
    assert_eq!((st.path.as_deref(), st.doc.artboards[0].rect.width()), (Some("/art/Poster.vectorcraft"), 120.0));
    assert_eq!(app.ui.recent_files.first().map(String::as_str), Some("/art/Poster.vectorcraft"));
    // A file that can't be read: reported, nothing opened, not a recent file.
    start(&mut app, "/art/Broken.vectorcraft", b"{\"format\":\"vectorcraft\"".to_vec()).unwrap();
    open_ready(&mut app);
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
        open_ready(&mut app);
        let st = app.session.doc().unwrap();
        assert!(*st.doc == *at_once, "{update}: the same document");
        assert!(st.history.undo.is_empty() && !st.is_dirty(), "{update}");
        assert_eq!(asked(&app), question, "{update}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// A file opened under automation roots is installed under them too: the open keeps the scope it
/// started in, not the desktop's access of the frame it finishes in.
#[test]
fn a_file_opened_under_automation_roots_finishes_under_them() {
    use vectorcraft_engine::file_access;
    let dir = scope_dir();
    let roots = file_access::AutomationRoots::new(&[dir.as_path()], &[]).unwrap();
    let mut app = VectorcraftApp::new(Session::new(), Services::default());
    file_access::confine(roots.as_ref(), || {
        start(&mut app, "/art/Scoped.vectorcraft", vectorcraft_format::save(&vectorcraft_doc::Document::new(10.0, 10.0), false))
    })
    .unwrap();
    let scope = app.opening.pending[0].scope.clone();
    assert!(scope.run(file_access::current).is_some(), "the roots go with the open");
}

fn scope_dir() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("vectorcraft-opening-scope-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A file asked for again while it is still being read is not read a second time (#3455).
#[test]
fn a_file_asked_for_again_while_it_is_read_opens_once() {
    let mut app = VectorcraftApp::new(Session::new(), Services::default());
    let bytes = vectorcraft_format::save(&vectorcraft_doc::Document::new(120.0, 80.0), false);
    start(&mut app, "/art/Poster.vectorcraft", bytes).unwrap();
    assert_eq!(crate::io::open_file(&mut app, "/art/Poster.vectorcraft", true).unwrap(), Value::Null);
    assert_eq!(app.opening.pending.len(), 1);
    open_ready(&mut app);
    assert_eq!(app.session.documents().len(), 1);
}

/// A document asked for again while a dialog was open, closed meanwhile, opens again once the
/// dialog closes and asks about its fonts; a file read meanwhile waits for that question too, so
/// the dialog acts on its own document (#5369).
#[test]
fn a_queued_reopen_that_asks_holds_back_files_ready_to_open() {
    let dir = std::env::temp_dir().join(format!("vectorcraft-opening-queued-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut app = VectorcraftApp::new(Session::new(), Services::default());
    app.services.read = Some(Box::new(|p: &str| std::fs::read(p).map_err(|e| e.to_string())));
    let (doc, other) = (dir.join("doc.vectorcraft"), dir.join("other.vectorcraft"));
    let (doc, other) = (doc.to_string_lossy().into_owned(), other.to_string_lossy().into_owned());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.run("text.create", json!({"x": 10, "y": 40, "text": "Type", "font": "No Such Font Family"})).unwrap();
    app.run("document.save", json!({"path": doc})).unwrap();
    app.run("file.new", json!({})).unwrap();
    app.run("document.save", json!({"path": other})).unwrap();
    app.run("file.new", json!({})).unwrap();
    app.ui.dialog = Some(crate::state::Dialog::new(crate::dialogs::save_options::KIND, json!({})));
    crate::io::open_reporting(&mut app, &doc).unwrap();
    crate::io::open_reporting(&mut app, &other).unwrap();
    assert_eq!(app.opening.activate.len(), 2);
    app.session.execute("file.close", &json!({"index": 1})).unwrap();
    app.session.execute("file.close", &json!({"index": 0})).unwrap();
    start(&mut app, "/art/Other.vectorcraft", vectorcraft_format::save(&vectorcraft_doc::Document::new(120.0, 80.0), false)).unwrap();
    // The other file is read; it waits for the dialog meanwhile.
    open_ready(&mut app);
    assert_eq!(app.opening.pending.len(), 1);
    app.ui.dialog = None;
    poll(&mut app);
    assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some(crate::dialogs::missing_fonts::KIND));
    assert_eq!(app.session.doc().unwrap().path.as_deref(), Some(&*doc), "the dialog's document stays active");
    assert_eq!(app.opening.activate.first().map(|(p, _)| p), Some(&other), "the later activation waits");
    assert_eq!(app.opening.pending.len(), 1, "the other file waits");
    let _ = std::fs::remove_dir_all(dir);
}

/// Of two files read off the UI thread and ready in the same frame, the first that opens and asks
/// about its fonts holds back the second, so the question acts on its own document (#5369).
#[test]
fn a_file_that_opens_and_asks_holds_back_the_next_one_ready() {
    let mut app = VectorcraftApp::new(Session::new(), Services::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.run("text.create", json!({"x": 10, "y": 40, "text": "Type", "font": "No Such Font Family"})).unwrap();
    let asks = vectorcraft_format::save(&app.session.doc().unwrap().doc, false);
    app.session.execute("file.close", &json!({})).unwrap();
    app.ui.dialog = Some(crate::state::Dialog::new(crate::dialogs::save_options::KIND, json!({})));
    start(&mut app, "/art/Asks.vectorcraft", asks).unwrap();
    start(&mut app, "/art/Plain.vectorcraft", vectorcraft_format::save(&vectorcraft_doc::Document::new(120.0, 80.0), false)).unwrap();
    // Both files are read; they wait for the dialog meanwhile.
    open_ready(&mut app);
    assert_eq!(app.opening.pending.len(), 2);
    app.ui.dialog = None;
    poll(&mut app);
    assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some(crate::dialogs::missing_fonts::KIND));
    assert_eq!(app.session.doc().unwrap().path.as_deref(), Some("/art/Asks.vectorcraft"), "the dialog's document stays active");
    assert_eq!(app.opening.pending.len(), 1, "the next file waits");
}

/// Draws the opening progress window at `time` seconds → the texts it drew.
fn progress(app: &mut VectorcraftApp, ctx: &egui::Context, time: f64) -> String {
    let mut out = ctx.run_ui(egui::RawInput { time: Some(time), ..Default::default() }, |ui| show(app, ui.ctx()));
    out.textures_delta.clear();
    let mut texts = String::new();
    for shape in out.shapes {
        if let egui::epaint::Shape::Text(t) = shape.shape {
            texts.push_str(t.galley.text());
            texts.push('\n');
        }
    }
    texts
}

/// A document's bytes, `title` naming it.
fn bytes(title: &str) -> Vec<u8> {
    let mut d = vectorcraft_doc::Document::new(120.0, 80.0);
    d.title = title.into();
    vectorcraft_format::save(&d, false)
}

/// #5930: a file that takes a while to open shows a progress window with its name and a moving
/// bar, after [`DELAY`] so a quick open doesn't flash it; a second file says it waits, and the
/// window waits for a dialog to close.
#[test]
fn a_file_that_takes_a_while_to_open_shows_a_progress_window() {
    let mut app = VectorcraftApp::new(Session::new(), Services::default());
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    assert_eq!(progress(&mut app, &ctx, 0.0), "", "nothing opening");
    start(&mut app, "/art/Big.vectorcraft", bytes("Big")).unwrap();
    assert_eq!(progress(&mut app, &ctx, 1.0), "", "not at once");
    progress(&mut app, &ctx, 1.0 + DELAY + 0.05); // a new window measures itself unseen first
    let texts = progress(&mut app, &ctx, 1.0 + DELAY + 0.1);
    for want in ["Opening Big.vectorcraft…", "Reading the file…", "Cancel", "Continue in Background"] {
        assert!(texts.contains(want), "{want} in {texts}");
    }
    assert!(!texts.contains("waiting"), "{texts}");
    start(&mut app, "/art/Next.vectorcraft", bytes("Next")).unwrap();
    let texts = progress(&mut app, &ctx, 2.0);
    assert!(texts.contains("Opening Big.vectorcraft…") && texts.contains("1 more file waiting"), "{texts}");
    app.ui.dialog = Some(crate::state::Dialog::new(crate::dialogs::save_options::KIND, json!({})));
    assert_eq!(progress(&mut app, &ctx, 3.0), "", "never two modals at once");
    app.ui.dialog = None;
    // Continue in Background: the window closes, the files still open.
    let before = app.session.documents().len();
    assert!(crate::menus::enabled(&app, "file.openInBackground"));
    assert_eq!(app.run("file.openInBackground", json!({})).unwrap()["hidden"], 2);
    assert_eq!(progress(&mut app, &ctx, 4.0), "");
    assert!(!crate::menus::enabled(&app, "file.openInBackground"));
    assert!(long_operation_busy(&app), "the status bar still shows them");
    open_ready(&mut app);
    assert_eq!(app.session.documents().len(), before + 2);
}

/// #5930: the progress window's Cancel (`file.cancelOpen`): the file doesn't open, even once it
/// has been read, and the status bar no longer says it is opening.
#[test]
fn cancelling_an_open_opens_nothing() {
    let mut app = VectorcraftApp::new(Session::new(), Services::default());
    assert!(!crate::menus::enabled(&app, "file.cancelOpen"));
    let before = app.session.documents().len();
    start(&mut app, "/art/Big.vectorcraft", bytes("Big")).unwrap();
    assert!(crate::menus::enabled(&app, "file.cancelOpen"));
    assert_eq!(app.run("file.cancelOpen", json!({})).unwrap()["cancelled"], 1);
    assert!(app.opening.pending.is_empty() && !long_operation_busy(&app));
    assert!(!app.ui.status.starts_with("Opening"), "{}", app.ui.status);
    std::thread::sleep(std::time::Duration::from_millis(50));
    poll(&mut app);
    assert_eq!(app.session.documents().len(), before);
    assert_ne!(app.ui.recent_files.first().map(String::as_str), Some("/art/Big.vectorcraft"));
}

/// The moving bar's segment enters at the left, crosses the track and leaves at the right, and a
/// time that isn't finite puts it at the start.
#[test]
fn the_indeterminate_bar_sweeps_across_its_track() {
    use crate::export_progress::sweep_at;
    assert_eq!(sweep_at(0.0), (0.0, 0.0));
    let (from, to) = sweep_at(0.8);
    assert!(from > 0.0 && to < 1.0 && to > from, "{from} {to}");
    let (from, to) = sweep_at(1.55);
    assert!(from > 0.6 && to == 1.0, "{from} {to}");
    assert_eq!(sweep_at(f64::NAN), (0.0, 0.0));
    assert_eq!(sweep_at(-0.8), sweep_at(0.8));
}
