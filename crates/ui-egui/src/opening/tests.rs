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
    let t0 = std::time::Instant::now();
    while t0.elapsed().as_millis() < 400 {
        poll(&mut app);
        std::thread::yield_now();
    }
    assert!(app.session.documents().len() == 1 && app.session.in_interaction(), "the file waits for the drag");
    app.session.pointer(&PointerEvent::new(PointerKind::Up, 60.0, 20.0), view).unwrap();
    let t0 = std::time::Instant::now();
    while !app.opening.pending.is_empty() && t0.elapsed().as_secs() < 30 {
        poll(&mut app);
        std::thread::yield_now();
    }
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
    let t0 = std::time::Instant::now();
    while t0.elapsed().as_millis() < 400 {
        poll(&mut app);
        std::thread::yield_now();
    }
    assert!(app.session.documents().len() == 1 && app.session.tool_busy(), "the file waits for the drag");
    crate::canvas::dispatch(&mut app, &PointerEvent::new(PointerKind::Up, 60.0, 20.0), view);
    let t0 = std::time::Instant::now();
    while !app.opening.pending.is_empty() && t0.elapsed().as_secs() < 30 {
        poll(&mut app);
        std::thread::yield_now();
    }
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
        let t0 = std::time::Instant::now();
        while t0.elapsed().as_millis() < 400 {
            poll(&mut app);
            std::thread::yield_now();
        }
        assert!(app.session.documents().len() == 1 && app.session.in_interaction(), "the file waits for the preview: {case}");
        if case == "line" {
            click(&mut app, 50.0);
        } else {
            app.session.cancel_interaction().unwrap();
        }
        let t0 = std::time::Instant::now();
        while !app.opening.pending.is_empty() && t0.elapsed().as_secs() < 30 {
            poll(&mut app);
            std::thread::yield_now();
        }
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
    let opened = |app: &mut VectorcraftApp| {
        let t0 = std::time::Instant::now();
        while !app.opening.pending.is_empty() && t0.elapsed().as_secs() < 30 {
            poll(app);
            std::thread::yield_now();
        }
    };
    click(&mut app, 20.0);
    assert!(app.session.tool_busy() && !app.session.in_interaction(), "the first point waits for the second");
    start(&mut app, "/art/Second.vectorcraft", vectorcraft_format::save(&vectorcraft_doc::Document::new(400.0, 300.0), false)).unwrap();
    opened(&mut app);
    assert_eq!(app.session.documents().len(), 2, "the file opens after one click");
    assert!(!app.session.tool_busy(), "the tool starts afresh");
    click(&mut app, 20.0);
    click(&mut app, 80.0);
    assert!(app.session.tool_busy() && !app.session.in_interaction(), "the second point waits for the line's place");
    start(&mut app, "/art/Third.vectorcraft", vectorcraft_format::save(&vectorcraft_doc::Document::new(120.0, 80.0), false)).unwrap();
    opened(&mut app);
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
