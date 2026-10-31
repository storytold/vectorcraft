use image::ImageFormat::{Bmp, Png, Tiff};
use vectorcraft_doc::{Document, NodeId};
use vectorcraft_engine::Session;
use vectorcraft_engine::cmd::clipboard::{BITMAP, Flavour, PNG, TEXT};

use super::*;
use crate::Services;
use crate::place::{DropAt, DropTarget};

/// An app reading files from disk, with a 400 × 300 pt document, that reads the files the user
/// places off the UI thread when `background`.
fn app(background: bool) -> VectorcraftApp {
    let services = Services { read: Some(Box::new(|p: &str| std::fs::read(p).map_err(|e| e.to_string()))), ..Default::default() };
    let mut app = VectorcraftApp::new(Session::new(), services);
    app.placing.off_the_ui_thread = background;
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app
}

/// `bytes` written to a fresh file named `name` in a folder of `test`'s own → its path.
fn saved(test: &str, name: &str, bytes: &[u8]) -> String {
    let dir = std::env::temp_dir().join(format!("vc-placing-{test}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    std::fs::write(&path, bytes).unwrap();
    path.to_string_lossy().into_owned()
}

/// A `w` × `h` picture of one color in `format`.
fn picture(format: image::ImageFormat, w: u32, h: u32) -> Vec<u8> {
    let mut out = vec![];
    image::RgbImage::from_pixel(w, h, image::Rgb([200, 40, 40])).write_to(&mut std::io::Cursor::new(&mut out), format).unwrap();
    out
}

/// A flat `w` × `h` Photoshop document of one color (raw RGB, no layers).
fn psd(w: u32, h: u32) -> Vec<u8> {
    let mut f = b"8BPS\0\x01\0\0\0\0\0\0\0\x03".to_vec();
    f.extend(h.to_be_bytes());
    f.extend(w.to_be_bytes());
    f.extend([0, 8, 0, 3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    for v in [40u8, 40, 200] {
        f.extend(std::iter::repeat_n(v, (w * h) as usize));
    }
    f
}

/// A one-page PDF with a rectangle on it.
fn pdf() -> Vec<u8> {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 100, "height": 50})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 5, "y": 5, "width": 20, "height": 10})).unwrap();
    let v = s.execute("document.serialize", &json!({"format": "pdf"})).unwrap();
    vectorcraft_format::base64_decode(v["dataBase64"].as_str().unwrap()).unwrap()
}

/// Poll each frame until every file read for Place has been placed (at most 60 s).
fn placed(app: &mut VectorcraftApp) {
    let t0 = std::time::Instant::now();
    while !app.placing.pending.is_empty() && t0.elapsed().as_secs() < 60 {
        poll(app);
        std::thread::yield_now();
    }
    assert!(app.placing.pending.is_empty(), "read within 60 s");
}

/// What a place changes: the active document (without the second it was made in), its
/// selection, its undo steps and the last journal entry.
fn state(app: &VectorcraftApp) -> (Document, Vec<NodeId>, usize, Option<(String, Value)>) {
    let st = app.session.doc().unwrap();
    let mut doc = (*st.doc).clone();
    doc.metadata.created = None;
    (doc, st.selection.objects.clone(), st.history.undo.len(), app.session.journal.last().cloned())
}

/// The names of the active document's top-level objects, bottom first.
fn names(app: &VectorcraftApp) -> Vec<String> {
    let st = app.session.doc().unwrap();
    st.doc.layers.iter().flat_map(|l| l.children().into_iter().flatten()).filter_map(|n| n.name.clone()).collect()
}

/// A TIFF, a Photoshop document, a BMP, a PDF page and a PDF-compatible `.ai` page (both placed
/// linked) read off the UI thread are placed as when they are read at once: the same object
/// with its bounds and link, the same selection, one undo step and the same journal entry. Until
/// then the document is unchanged, and the status bar and the busy cursor show the file is being
/// placed.
#[test]
fn a_file_read_off_the_ui_thread_places_as_one_read_at_once() {
    let page = pdf();
    let files = [
        ("photo.tif", picture(Tiff, 300, 200)),
        ("photo.psd", psd(300, 200)),
        ("photo.bmp", picture(Bmp, 300, 200)),
        ("page.pdf", page.clone()),
        ("page.ai", page),
    ];
    for (name, bytes) in files {
        let p = json!({"path": saved("same", name, &bytes), "at": [200, 150]});
        let mut at_once = app(false);
        at_once.run("file.place", p.clone()).unwrap();
        let mut app = app(true);
        let before = state(&app);
        let r = start(&mut app, &p, false).unwrap();
        assert_eq!(r, json!({"background": true, "status": format!("Placing {name}…")}));
        assert_eq!(app.ui.status, format!("Placing {name}…"));
        assert!(crate::opening::long_operation_busy(&app), "{name}");
        assert!(state(&app) == before, "{name}: nothing is placed until the file is read");
        placed(&mut app);
        assert!(state(&app) == state(&at_once), "{name}: placed as at once");
        assert!(app.ui.status.is_empty(), "{name}: {}", app.ui.status);
        assert!(!crate::opening::long_operation_busy(&app), "{name}");
    }
}

/// Another app's bitmap on the system clipboard, and the text beside it, read as the desktop
/// hosts read them: with `converts`, the bitmap as the work that makes its flavour
/// ([`crate::ClipboardRead::Later`], `None`: it can't be made), which notes the thread it runs
/// on in `thread`. The work of each read waits for its entry of `holds` to send, when it has
/// one.
struct OtherApps {
    bitmap: Option<Flavour>,
    text: Option<Flavour>,
    converts: bool,
    thread: Arc<Mutex<Option<String>>>,
    holds: Vec<Option<std::sync::mpsc::Receiver<()>>>,
}

impl crate::SystemClipboard for OtherApps {
    fn write(&mut self, _: &[Flavour]) -> Result<(), String> {
        Ok(())
    }
    fn holds_ours(&mut self) -> bool {
        false
    }
    fn read(&mut self, mimes: &[&'static str]) -> Option<Flavour> {
        self.read_deferred(mimes)?.flavour()
    }
    fn has(&mut self, mimes: &[&'static str]) -> bool {
        mimes.contains(&BITMAP) || (mimes.contains(&TEXT) && self.text.is_some())
    }
    fn read_deferred(&mut self, mimes: &[&'static str]) -> Option<crate::ClipboardRead> {
        if mimes.contains(&TEXT)
            && let Some(text) = &self.text
        {
            return Some(crate::ClipboardRead::Now(text.clone()));
        }
        if !mimes.contains(&BITMAP) {
            return None;
        }
        let (bitmap, thread) = (self.bitmap.clone(), self.thread.clone());
        if !self.converts {
            return bitmap.map(crate::ClipboardRead::Now);
        }
        let hold = if self.holds.is_empty() { None } else { self.holds.remove(0) };
        Some(crate::ClipboardRead::Later(Box::new(move || {
            if let Some(hold) = hold {
                let _ = hold.recv_timeout(std::time::Duration::from_secs(10));
            }
            *thread.lock().unwrap() = std::thread::current().name().map(str::to_string);
            bitmap
        })))
    }
}

/// [`app`] with `bitmap` and `text` on the system clipboard ([`OtherApps`]), the view centered
/// on (250, 180) → the app and the thread the bitmap was converted on.
fn pasting_app(background: bool, bitmap: Option<Flavour>, text: Option<&str>, converts: bool) -> (VectorcraftApp, Arc<Mutex<Option<String>>>) {
    let thread = Arc::new(Mutex::new(None));
    let mut app = app(background);
    let text = text.map(|t| Flavour { mime: TEXT, data: t.as_bytes().to_vec() });
    app.services.system_clipboard = Some(Box::new(OtherApps { bitmap, text, converts, thread: thread.clone(), holds: vec![] }));
    let v = app.view_mut().unwrap();
    (v.center, v.fitted) = (vectorcraft_geom::Point::new(250.0, 180.0), true);
    (app, thread)
}

/// A Paste the user gives decodes another app's bitmap off the UI thread: a BMP as
/// `clipboard.importImage` reads it, and a bitmap the host converts to PNG (Windows'
/// device-independent bitmap, Linux's image) converted there too. Until then the document is
/// unchanged, and the status bar and the busy cursor show the paste; then it is pasted as at
/// once, where the view was centered when the paste was given: the same object, selection, undo
/// step and internal clipboard. `edit.paste` from MCP and scripts reads it at once.
#[test]
fn a_bitmap_the_user_pastes_is_read_off_the_ui_thread() {
    let bmp = Flavour { mime: "image/bmp", data: picture(Bmp, 300, 200) };
    let png = Flavour { mime: PNG, data: picture(Png, 300, 200) };
    for (bitmap, converts) in [(bmp, false), (png, true)] {
        for id in ["edit.paste", "edit.pasteInPlace"] {
            let (mut at_once, _) = pasting_app(false, Some(bitmap.clone()), None, converts);
            crate::menus::invoke(&mut at_once, id, json!({}));
            let (mut app, _) = pasting_app(true, Some(bitmap.clone()), None, converts);
            crate::menus::invoke(&mut app, id, json!({}));
            // The view moves before the bitmap is read.
            app.view_mut().unwrap().center = vectorcraft_geom::Point::new(40.0, 30.0);
            placed(&mut app);
            assert!(state(&app) == state(&at_once), "{} {id}: pasted where the view was", bitmap.mime);
        }
        let (mut at_once, _) = pasting_app(false, Some(bitmap.clone()), None, converts);
        crate::menus::invoke(&mut at_once, "edit.paste", json!({}));
        let (mut app, thread) = pasting_app(true, Some(bitmap.clone()), None, converts);
        let before = state(&app);
        app.placing.for_user = true;
        let r = app.run("edit.paste", json!({}));
        app.placing.for_user = false;
        assert_eq!(r, Ok(json!({"background": true, "status": "Pasting…"})), "{}", bitmap.mime);
        assert_eq!(app.ui.status, "Pasting…");
        assert!(crate::opening::long_operation_busy(&app));
        assert!(state(&app) == before, "{}: nothing is pasted until the bitmap is read", bitmap.mime);
        placed(&mut app);
        assert!(state(&app) == state(&at_once), "{}: pasted as at once", bitmap.mime);
        assert!(app.session.clipboard == at_once.session.clipboard, "{}", bitmap.mime);
        assert!(app.ui.status.is_empty(), "{}", app.ui.status);
        if converts {
            assert_eq!(thread.lock().unwrap().as_deref(), Some("vectorcraft-place"));
        }
        // From the menu or the keys too.
        crate::menus::invoke(&mut app, "edit.paste", json!({}));
        assert_eq!(app.placing.pending.len(), 1);
        placed(&mut app);
        // MCP and scripts: at once.
        let r = app.run("edit.paste", json!({})).unwrap();
        assert!(r["ids"].is_array() && app.placing.pending.is_empty(), "{r}");
        assert_eq!(app.session.doc().unwrap().selection.objects.len(), 1);
    }
}

/// A bitmap that can't be read reports as at once and pastes nothing; one the host can't
/// convert pastes the internal clipboard as it is, or the address beside it as text, as at
/// once.
#[test]
fn a_pasted_bitmap_that_cant_be_read_off_the_ui_thread_reports_as_at_once() {
    let url = "https://example.com/images/cat.png";
    let cases = [(Some(Flavour { mime: PNG, data: b"not a picture".to_vec() }), None, false), (None, None, true), (None, Some(url), true)];
    for (bitmap, text, converts) in cases {
        let (mut at_once, _) = pasting_app(false, bitmap.clone(), text, converts);
        let (mut app, _) = pasting_app(true, bitmap, text, converts);
        for app in [&mut at_once, &mut app] {
            app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
            app.run("edit.copy", json!({})).unwrap();
            crate::menus::invoke(app, "edit.paste", json!({}));
            assert_eq!(app.placing.pending.len(), usize::from(app.placing.off_the_ui_thread));
            placed(app);
        }
        assert_eq!(app.ui.status, at_once.ui.status);
        assert!(state(&app) == state(&at_once), "text: {text:?}, converts: {converts}");
        let st = app.session.doc().unwrap();
        let pasted = st.selection.objects.first().and_then(|id| st.doc.node(*id)).map(|n| &n.kind);
        match (text, converts) {
            (Some(_), _) => assert!(matches!(pasted, Some(vectorcraft_doc::NodeKind::Text(_))), "the address is pasted as text"),
            (None, true) => assert!(matches!(pasted, Some(vectorcraft_doc::NodeKind::Path { .. })), "the copied square is pasted"),
            (None, false) => assert!(app.ui.status.starts_with("Couldn't paste from the clipboard: "), "{}", app.ui.status),
        }
    }
}

#[test]
fn a_failed_bitmap_paste_keeps_the_document_the_user_switched_to() {
    let (mut app, _) = pasting_app(true, None, None, true);
    let (go, hold) = channel();
    let clip = OtherApps {
        bitmap: Some(Flavour { mime: PNG, data: b"not a picture".to_vec() }),
        text: None,
        converts: true,
        thread: Arc::new(Mutex::new(None)),
        holds: vec![Some(hold)],
    };
    app.services.system_clipboard = Some(Box::new(clip));
    let first = app.session.doc().unwrap();
    let first = (first.doc.clone(), first.selection.objects.clone(), first.history.undo.len());
    crate::menus::invoke(&mut app, "edit.paste", json!({}));
    assert_eq!(app.placing.pending.len(), 1);
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    let selected = app.run("shape.rectangle", json!({"x": 30, "y": 40, "width": 25, "height": 35})).unwrap()["id"].as_u64().unwrap();
    app.run("layer.new", json!({})).unwrap();
    app.run("select.set", json!({"ids": [selected]})).unwrap();
    let second = state(&app);
    let insertion = Insertion::of(&app);
    let clipboard = app.session.clipboard.clone();
    let view = app.view_mut().unwrap();
    (view.center, view.zoom, view.rotation, view.fitted) = (vectorcraft_geom::Point::new(70.0, 90.0), 2.0, 15.0, true);
    let view = *app.view().unwrap();
    assert!(go.send(()).is_ok());
    placed(&mut app);
    assert_eq!(app.session.active_index(), Some(1));
    assert!(state(&app) == second);
    assert!(Insertion::of(&app) == insertion);
    assert!(app.session.clipboard == clipboard);
    let current_view = app.view().unwrap();
    assert_eq!(
        (current_view.center, current_view.zoom, current_view.rotation, current_view.fitted),
        (view.center, view.zoom, view.rotation, view.fitted)
    );
    assert!(app.ui.status.starts_with("Couldn't paste from the clipboard: "), "{}", app.ui.status);
    assert!(app.session.set_active(0));
    let after = app.session.doc().unwrap();
    assert!(after.doc == first.0 && after.selection.objects == first.1 && after.history.undo.len() == first.2);
    assert!(app.placing.chains.is_empty() && app.placing.latest_chain.is_empty());
}

#[test]
fn a_paste_queued_after_a_failed_bitmap_uses_the_original_selection() {
    for id in ["edit.pasteInFront", "edit.pasteInBack"] {
        let (mut app, _) = pasting_app(true, None, None, true);
        let source = app.run("shape.rectangle", json!({"x": 150, "y": 10, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap();
        app.run("edit.copy", json!({})).unwrap();
        let clipboard = app.session.clipboard.clone();
        let rect = |app: &mut VectorcraftApp, x: u32| {
            app.run("shape.rectangle", json!({"x": x, "y": 10, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap()
        };
        let (a, b, c) = (rect(&mut app, 10), rect(&mut app, 40), rect(&mut app, 70));
        app.run("select.set", json!({"ids": [b]})).unwrap();
        let (go, hold) = channel();
        app.services.system_clipboard = Some(Box::new(OtherApps {
            bitmap: Some(Flavour { mime: PNG, data: b"not a picture".to_vec() }),
            text: None,
            converts: true,
            thread: Arc::new(Mutex::new(None)),
            holds: vec![Some(hold)],
        }));
        crate::menus::invoke(&mut app, id, json!({}));
        app.services.system_clipboard = None;
        crate::menus::invoke(&mut app, id, json!({}));
        assert_eq!(app.placing.pending.len(), 2, "{id}: both pastes wait");
        assert_eq!(app.placing.chains.len(), 1, "{id}: the second paste joined the first");
        app.run("select.set", json!({"ids": [if id.ends_with("Front") { a } else { c }]})).unwrap();
        assert!(go.send(()).is_ok());
        placed(&mut app);
        let st = app.session.doc().unwrap();
        let selected = st.selection.objects[0];
        let order: Vec<_> = st.doc.layers[0].children().unwrap().iter().map(|n| n.id).collect();
        let expected = if id.ends_with("Front") {
            vec![NodeId(source), NodeId(a), NodeId(b), selected, NodeId(c)]
        } else {
            vec![NodeId(source), NodeId(a), selected, NodeId(b), NodeId(c)]
        };
        assert_eq!(order, expected, "{id}: the internal paste stays beside the original anchor");
        assert!(matches!(st.doc.node(selected).map(|n| &n.kind), Some(vectorcraft_doc::NodeKind::Path { .. })), "{id}");
        assert!(app.session.clipboard == clipboard, "{id}: the failed bitmap did not replace the clipboard");
        assert!(app.placing.chains.is_empty() && app.placing.latest_chain.is_empty(), "{id}: the queue was released");
    }
}

/// A Copy made while a pasted bitmap is read keeps the internal clipboard it made, and a paste
/// whose document was closed meanwhile pastes nothing, with a message.
#[test]
fn a_copy_or_a_close_while_a_pasted_bitmap_is_read() {
    let bitmap = Some(Flavour { mime: PNG, data: picture(Png, 60, 40) });
    let (mut app, _) = pasting_app(true, bitmap.clone(), None, true);
    crate::menus::invoke(&mut app, "edit.paste", json!({}));
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
    app.run("edit.copy", json!({})).unwrap();
    let copied = app.session.clipboard.clone();
    placed(&mut app);
    let st = app.session.doc().unwrap();
    let pasted = st.doc.node(st.selection.objects[0]).unwrap();
    assert!(matches!(pasted.kind, vectorcraft_doc::NodeKind::Image(_)), "the bitmap is pasted");
    assert!(app.session.clipboard == copied, "the copy stays on the internal clipboard");
    // A bitmap that can't be converted pastes the internal clipboard as it was when the paste
    // was given, and the later copy stays on it.
    let (mut app, _) = pasting_app(true, None, None, true);
    let square = |app: &mut VectorcraftApp, x: u32| {
        app.run("shape.rectangle", json!({"x": x, "y": 10, "width": 20, "height": 20})).unwrap();
        app.run("edit.copy", json!({})).unwrap();
    };
    square(&mut app, 10);
    let first = app.session.clipboard.clone();
    crate::menus::invoke(&mut app, "edit.pasteInPlace", json!({}));
    square(&mut app, 200);
    let second = app.session.clipboard.clone();
    placed(&mut app);
    let st = app.session.doc().unwrap();
    let pasted = st.doc.node(st.selection.objects[0]).and_then(|n| n.geometric_bounds()).map(|b| b.x0);
    assert!(first != second && pasted == Some(10.0), "the first copy is pasted: {pasted:?}");
    assert!(app.session.clipboard == second, "the later copy stays on the internal clipboard");
    let (mut app, _) = pasting_app(true, bitmap, None, true);
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    crate::menus::invoke(&mut app, "edit.paste", json!({}));
    app.session.execute("file.close", &json!({})).unwrap();
    let left = state(&app);
    placed(&mut app);
    assert_eq!(app.ui.status, "Couldn't paste: the document it was pasted in was closed");
    assert!(state(&app) == left, "the other document is unchanged");
    assert!(app.placing.chains.is_empty() && app.placing.latest_chain.is_empty(), "no paste waits");
}

#[test]
fn deferred_fallback_keeps_its_swatch_dialog_art_and_later_copy() {
    for confirm in [true, false] {
        let (mut app, _) = pasting_app(true, None, None, true);
        app.run("swatch.new", json!({"name": "Brand", "color": "#ff0000", "global": true})).unwrap();
        let source = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap();
        app.run("paint.setFill", json!({"ids": [source], "swatch": "Brand"})).unwrap();
        app.run("edit.copy", json!({})).unwrap();
        let requested = app.session.clipboard.clone();
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        app.run("swatch.new", json!({"name": "Brand", "color": "#0000ff", "global": true})).unwrap();
        crate::menus::invoke(&mut app, "edit.pasteInPlace", json!({}));
        assert_eq!(app.placing.pending.len(), 1);
        let newer = app.run("shape.rectangle", json!({"x": 200, "y": 10, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap();
        app.run("edit.copy", json!({})).unwrap();
        let copied = app.session.clipboard.clone();
        assert!(requested != copied);
        placed(&mut app);
        assert_eq!(app.ui.dialog.as_ref().unwrap().kind, crate::dialogs::swatch_conflict::KIND);
        assert_eq!(app.ui.dialog.as_ref().unwrap().fields["conflicts"][0]["clipboard"], "#ff0000");
        assert!(app.session.clipboard == copied);
        let latest = app.run("shape.rectangle", json!({"x": 250, "y": 10, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap();
        app.run("edit.copy", json!({})).unwrap();
        let latest_copy = app.session.clipboard.clone();
        if confirm {
            app.ui.dialog.as_mut().unwrap().fields.insert("choice".into(), json!("add"));
            crate::dialogs::confirm(&mut app).unwrap();
            assert!(app.ui.dialog.is_none());
            let st = app.session.doc().unwrap();
            let pasted = st.doc.node(st.selection.objects[0]).unwrap();
            assert_eq!(pasted.geometric_bounds().unwrap().x0, 10.0);
            assert_eq!(pasted.appearance.fill_paint().color(), Some(vectorcraft_color::Color::from_hex("#ff0000").unwrap()));
        } else {
            crate::dialogs::cancel(&mut app);
            assert!(app.ui.dialog.is_none());
            assert_eq!(app.session.doc().unwrap().doc.layers[0].children().unwrap().len(), 2);
        }
        assert!(app.session.clipboard == latest_copy);
        app.run("edit.pasteInPlace", json!({})).unwrap();
        let st = app.session.doc().unwrap();
        assert_eq!(st.doc.node(st.selection.objects[0]).unwrap().geometric_bounds().unwrap().x0, 250.0);
        assert!(st.doc.node(NodeId(newer)).is_some() && st.doc.node(NodeId(latest)).is_some());
    }
}

#[test]
fn a_synchronous_paste_replaces_a_deferred_swatch_dialog() {
    let (mut app, _) = pasting_app(true, None, None, true);
    app.run("swatch.new", json!({"name": "Brand", "color": "#ff0000", "global": true})).unwrap();
    let first = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap()["id"].clone();
    app.run("paint.setFill", json!({"ids": [first], "swatch": "Brand"})).unwrap();
    app.run("edit.copy", json!({})).unwrap();
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.run("swatch.new", json!({"name": "Brand", "color": "#0000ff", "global": true})).unwrap();
    crate::menus::invoke(&mut app, "edit.pasteInPlace", json!({}));
    placed(&mut app);
    assert_eq!(app.ui.dialog.as_ref().unwrap().fields["conflicts"][0]["clipboard"], "#ff0000");
    app.run("swatch.new", json!({"name": "Other", "color": "#ffff00", "global": true})).unwrap();
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.run("swatch.new", json!({"name": "Other", "color": "#00ff00", "global": true})).unwrap();
    let second = app.run("shape.rectangle", json!({"x": 200, "y": 10, "width": 20, "height": 20})).unwrap()["id"].clone();
    app.run("paint.setFill", json!({"ids": [second], "swatch": "Other"})).unwrap();
    app.run("edit.copy", json!({})).unwrap();
    app.session.set_active(1);
    app.sync_views();
    let result = crate::control::handle(
        &mut app,
        &egui::Context::default(),
        &crate::control::ControlRequest::new("engine.execute", json!({"command": "edit.pasteInPlace", "params": {}})).0,
    );
    let crate::control::Outcome::Done(result) = result else { panic!("expected command result") };
    assert_eq!(result["ok"], true, "{result}");
    assert_eq!(app.ui.dialog.as_ref().unwrap().fields["conflicts"][0]["clipboard"], "#00ff00");
    app.ui.dialog.as_mut().unwrap().fields.insert("choice".into(), json!("add"));
    crate::dialogs::confirm(&mut app).unwrap();
    let st = app.session.doc().unwrap();
    assert_eq!(st.doc.node(st.selection.objects[0]).unwrap().geometric_bounds().unwrap().x0, 200.0);
    assert!(app.placing.chains.is_empty());
}

#[test]
fn a_find_font_dialog_releases_a_deferred_swatch_paste_before_queued_work() {
    for id in ["edit.pasteInFront", "edit.pasteInBack"] {
        let (mut app, _) = pasting_app(true, None, None, true);
        app.run("swatch.new", json!({"name": "Brand", "color": "#ff0000", "global": true})).unwrap();
        let source = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap()["id"].clone();
        app.run("paint.setFill", json!({"ids": [source], "swatch": "Brand"})).unwrap();
        app.run("edit.copy", json!({})).unwrap();
        let requested = app.session.clipboard.clone();
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        app.run("swatch.new", json!({"name": "Brand", "color": "#0000ff", "global": true})).unwrap();
        let anchor = app.run("shape.rectangle", json!({"x": 100, "y": 10, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap();
        app.run("select.set", json!({"ids": [anchor]})).unwrap();
        crate::menus::invoke(&mut app, id, json!({}));
        crate::menus::invoke(&mut app, id, json!({}));
        let start = std::time::Instant::now();
        while app.ui.dialog.is_none() && start.elapsed().as_secs() < 60 {
            poll(&mut app);
            std::thread::yield_now();
        }
        assert_eq!(app.ui.dialog.as_ref().unwrap().kind, crate::dialogs::swatch_conflict::KIND);
        assert_eq!(app.placing.pending.len(), 1);
        let later = app.run("shape.rectangle", json!({"x": 200, "y": 10, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap();
        app.run("edit.copy", json!({})).unwrap();
        let newer = app.session.clipboard.clone();
        assert!(requested != newer);
        let result = crate::control::handle(
            &mut app,
            &egui::Context::default(),
            &crate::control::ControlRequest::new("engine.execute", json!({"command": "type.findFont", "params": {}})).0,
        );
        let crate::control::Outcome::Done(result) = result else { panic!("expected command result") };
        assert_eq!(result["ok"], true, "{result}");
        assert_eq!(app.ui.dialog.as_ref().unwrap().kind, "findFont");
        assert!(app.placing.swatch_paste.is_none());
        assert_eq!(app.placing.chains.values().map(|ch| ch.waiting).collect::<Vec<_>>(), [1]);
        assert_eq!(app.session.doc().unwrap().doc.layers[0].children().unwrap().len(), 2);
        crate::dialogs::cancel(&mut app);
        placed(&mut app);
        assert_eq!(app.ui.dialog.as_ref().unwrap().kind, crate::dialogs::swatch_conflict::KIND);
        app.ui.dialog.as_mut().unwrap().fields.insert("choice".into(), json!("add"));
        crate::dialogs::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
        let st = app.session.doc().unwrap();
        let pasted_id = st.selection.objects[0];
        let pasted = st.doc.node(pasted_id).unwrap();
        assert_eq!(pasted.geometric_bounds().unwrap().x0, 10.0, "{id}");
        assert_eq!(pasted.appearance.fill_paint().color(), Some(vectorcraft_color::Color::from_hex("#ff0000").unwrap()));
        let order: Vec<_> = st.doc.layers[0].children().unwrap().iter().map(|n| n.id).collect();
        let anchor_pos = order.iter().position(|n| *n == NodeId(anchor)).unwrap();
        let pasted_pos = order.iter().position(|n| *n == pasted_id).unwrap();
        assert_eq!(pasted_pos, if id.ends_with("Front") { anchor_pos + 1 } else { anchor_pos - 1 }, "{id}");
        assert!(st.doc.node(NodeId(later)).is_some());
        assert!(app.session.clipboard == newer);
        assert!(app.placing.chains.is_empty() && app.placing.latest_chain.is_empty());
    }
}

#[test]
fn a_deferred_swatch_paste_rejects_a_changed_token_or_closed_document() {
    for close_document in [false, true] {
        let (mut app, _) = pasting_app(true, None, None, true);
        app.run("swatch.new", json!({"name": "Brand", "color": "#ff0000", "global": true})).unwrap();
        let source = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap()["id"].clone();
        app.run("paint.setFill", json!({"ids": [source], "swatch": "Brand"})).unwrap();
        app.run("edit.copy", json!({})).unwrap();
        let clipboard = app.session.clipboard.clone();
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        app.run("swatch.new", json!({"name": "Brand", "color": "#0000ff", "global": true})).unwrap();
        crate::menus::invoke(&mut app, "edit.pasteInPlace", json!({}));
        placed(&mut app);
        assert_eq!(app.ui.dialog.as_ref().unwrap().kind, crate::dialogs::swatch_conflict::KIND);
        if close_document {
            app.session.execute("file.close", &json!({})).unwrap();
        } else {
            let result = crate::control::handle(
                &mut app,
                &egui::Context::default(),
                &crate::control::ControlRequest::new("ui.dialog.set", json!({"field": "__deferredPaste", "value": 999999})).0,
            );
            let crate::control::Outcome::Done(result) = result else { panic!("expected dialog result") };
            assert_eq!(result["ok"], true, "{result}");
        }
        let error = crate::dialogs::confirm(&mut app).unwrap_err();
        assert!(
            error.contains(if close_document { "document it was pasted in was closed" } else { "deferred paste is no longer available" }),
            "{error}"
        );
        assert!(app.ui.dialog.is_none());
        assert!(app.placing.swatch_paste.is_none());
        assert!(app.placing.chains.is_empty() && app.placing.latest_chain.is_empty());
        assert!(app.session.clipboard == clipboard);
        let st = app.session.doc().unwrap();
        assert_eq!(st.doc.layers[0].children().unwrap().len(), if close_document { 1 } else { 0 });
    }
}

#[test]
fn a_deferred_swatch_paste_uses_its_original_layer() {
    let (mut app, _) = pasting_app(true, None, None, true);
    app.run("swatch.new", json!({"name": "Brand", "color": "#ff0000", "global": true})).unwrap();
    let source = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap()["id"].clone();
    app.run("paint.setFill", json!({"ids": [source], "swatch": "Brand"})).unwrap();
    app.run("edit.copy", json!({})).unwrap();
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.run("swatch.new", json!({"name": "Brand", "color": "#0000ff", "global": true})).unwrap();
    let first = app.session.doc().unwrap().doc.layers[0].id;
    let selected = app.run("shape.rectangle", json!({"x": 100, "y": 10, "width": 20, "height": 20})).unwrap()["id"].clone();
    app.run("select.set", json!({"ids": [selected]})).unwrap();
    crate::menus::invoke(&mut app, "edit.pasteInPlace", json!({}));
    app.run("layer.new", json!({})).unwrap();
    let second = app.session.doc().unwrap().current_layer().unwrap();
    placed(&mut app);
    assert_eq!(app.ui.dialog.as_ref().unwrap().kind, crate::dialogs::swatch_conflict::KIND);
    app.ui.dialog.as_mut().unwrap().fields.insert("choice".into(), json!("add"));
    crate::dialogs::confirm(&mut app).unwrap();
    let st = app.session.doc().unwrap();
    assert_eq!(st.doc.layer_containing(st.selection.objects[0]), Some(first));
    assert_eq!(st.current_layer(), Some(second));
}

#[test]
fn a_queued_paste_stacks_after_a_deferred_swatch_confirmation() {
    for id in ["edit.pasteInFront", "edit.pasteInBack"] {
        let (mut app, _) = pasting_app(true, None, None, true);
        app.run("swatch.new", json!({"name": "Brand", "color": "#ff0000", "global": true})).unwrap();
        let source = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap()["id"].clone();
        app.run("paint.setFill", json!({"ids": [source], "swatch": "Brand"})).unwrap();
        app.run("edit.copy", json!({})).unwrap();
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        app.run("swatch.new", json!({"name": "Brand", "color": "#0000ff", "global": true})).unwrap();
        let anchor = app.run("shape.rectangle", json!({"x": 100, "y": 10, "width": 20, "height": 20})).unwrap()["id"].clone();
        app.run("select.set", json!({"ids": [anchor]})).unwrap();
        crate::menus::invoke(&mut app, id, json!({}));
        crate::menus::invoke(&mut app, id, json!({}));
        let start = std::time::Instant::now();
        while app.ui.dialog.is_none() && start.elapsed().as_secs() < 60 {
            poll(&mut app);
            std::thread::yield_now();
        }
        assert_eq!(app.placing.pending.len(), 1);
        app.ui.dialog.as_mut().unwrap().fields.insert("choice".into(), json!("add"));
        crate::dialogs::confirm(&mut app).unwrap();
        placed(&mut app);
        assert_eq!(app.ui.dialog.as_ref().unwrap().kind, crate::dialogs::swatch_conflict::KIND);
        app.ui.dialog.as_mut().unwrap().fields.insert("choice".into(), json!("add"));
        crate::dialogs::confirm(&mut app).unwrap();
        let st = app.session.doc().unwrap();
        let xs: Vec<_> = st.doc.layers[0].children().unwrap().iter().map(|n| n.geometric_bounds().unwrap().x0).collect();
        let expected = if id.ends_with("Front") { vec![100.0, 10.0, 10.0] } else { vec![10.0, 10.0, 100.0] };
        assert_eq!(xs, expected, "{id}");
    }
}

/// A paste given while a pasted bitmap is read waits for it, also when it pastes the internal
/// clipboard or another app's text at once: it is pasted above the picture, and what it pasted
/// stays on the internal clipboard.
#[test]
fn a_paste_given_while_a_pasted_bitmap_is_read_waits_for_it() {
    for text in [None, Some("hello")] {
        let (mut app, _) = pasting_app(true, None, None, true);
        let (go, hold) = channel();
        let bitmap = Some(Flavour { mime: PNG, data: picture(Png, 60, 40) });
        let clip = OtherApps { bitmap, text: None, converts: true, thread: Arc::new(Mutex::new(None)), holds: vec![Some(hold)] };
        app.services.system_clipboard = Some(Box::new(clip));
        crate::menus::invoke(&mut app, "edit.paste", json!({}));
        assert_eq!(app.placing.pending.len(), 1);
        // The internal clipboard: a square copied meanwhile. Text: another app's text.
        app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
        app.run("edit.copy", json!({})).unwrap();
        let text = text.map(|t| Flavour { mime: TEXT, data: t.as_bytes().to_vec() });
        let clip = OtherApps { bitmap: None, text: text.clone(), converts: false, thread: Arc::new(Mutex::new(None)), holds: vec![] };
        app.services.system_clipboard = Some(Box::new(clip));
        crate::menus::invoke(&mut app, "edit.paste", json!({}));
        let second = app.session.clipboard.clone();
        assert_eq!(app.placing.pending.len(), 2, "text: {}: the second paste waits", text.is_some());
        let _ = go.send(());
        placed(&mut app);
        let st = app.session.doc().unwrap();
        let kinds: Vec<&str> = st.doc.layers[0]
            .children()
            .unwrap()
            .iter()
            .map(|n| match n.kind {
                vectorcraft_doc::NodeKind::Image(_) => "picture",
                vectorcraft_doc::NodeKind::Text(_) => "text",
                _ => "path",
            })
            .collect();
        let pasted = if text.is_some() { "text" } else { "path" };
        assert_eq!(kinds, ["path", "picture", pasted], "the second paste is above the picture");
        assert!(app.session.clipboard == second, "text: {}: what the second paste pasted stays on the internal clipboard", text.is_some());
    }
}

/// Paste in Front and Paste in Back put a bitmap decoded off the UI thread next to the objects
/// selected when the paste was given, as at once, when another object was selected meanwhile;
/// a second one given before the first is pasted goes next to the first, as at once.
#[test]
fn a_paste_in_front_read_off_the_ui_thread_keeps_its_place() {
    let bitmap = Some(Flavour { mime: PNG, data: picture(Png, 60, 40) });
    for id in ["edit.pasteInFront", "edit.pasteInBack"] {
        for twice in [false, true] {
            let (mut at_once, _) = pasting_app(false, bitmap.clone(), None, true);
            let (mut app, _) = pasting_app(true, bitmap.clone(), None, true);
            for app in [&mut at_once, &mut app] {
                let rect = |app: &mut VectorcraftApp, x: u32| {
                    app.run("shape.rectangle", json!({"x": x, "y": 10, "width": 20, "height": 20})).unwrap()["id"].clone()
                };
                let (a, b, c) = (rect(app, 10), rect(app, 40), rect(app, 70));
                app.run("select.set", json!({"ids": [b]})).unwrap();
                crate::menus::invoke(app, id, json!({}));
                if twice {
                    crate::menus::invoke(app, id, json!({}));
                } else {
                    let other = if id.ends_with("Front") { a } else { c };
                    app.run("select.set", json!({"ids": [other]})).unwrap();
                }
                placed(app);
            }
            // Each object in paint order: `None` for a square, the paste order for a picture.
            let order = |app: &VectorcraftApp| {
                let st = app.session.doc().unwrap();
                let children = st.doc.layers[0].children().unwrap();
                let mut pictures: Vec<NodeId> =
                    children.iter().filter(|n| matches!(n.kind, vectorcraft_doc::NodeKind::Image(_))).map(|n| n.id).collect();
                pictures.sort();
                children.iter().map(|n| pictures.iter().position(|p| *p == n.id)).collect::<Vec<_>>()
            };
            assert_eq!(order(&app), order(&at_once), "{id}, twice: {twice}");
        }
    }
}

/// A second Paste in Front given before the first is pasted goes next to the first one's
/// picture, also when another object was selected between the two pastes' ends, and a third
/// given after the first was pasted, while the second waits, goes next to the second's.
#[test]
fn a_second_paste_in_front_follows_the_first() {
    for third in [false, true] {
        let (mut app, _) = pasting_app(true, None, None, true);
        let (go, hold) = channel();
        let thread = Arc::new(Mutex::new(None));
        let bitmap = Some(Flavour { mime: PNG, data: picture(Png, 60, 40) });
        app.services.system_clipboard = Some(Box::new(OtherApps { bitmap, text: None, converts: true, thread, holds: vec![None, Some(hold)] }));
        let rect =
            |app: &mut VectorcraftApp, x: u32| app.run("shape.rectangle", json!({"x": x, "y": 10, "width": 20, "height": 20})).unwrap()["id"].clone();
        let (a, b) = (rect(&mut app, 10), rect(&mut app, 40));
        app.run("select.set", json!({"ids": [b]})).unwrap();
        crate::menus::invoke(&mut app, "edit.pasteInFront", json!({}));
        crate::menus::invoke(&mut app, "edit.pasteInFront", json!({}));
        let t0 = std::time::Instant::now();
        while app.placing.pending.len() > 1 && t0.elapsed().as_secs() < 60 {
            poll(&mut app);
            std::thread::yield_now();
        }
        assert_eq!(app.placing.pending.len(), 1, "the first paste ran");
        if third {
            crate::menus::invoke(&mut app, "edit.pasteInFront", json!({}));
        } else {
            app.run("select.set", json!({"ids": [a]})).unwrap();
        }
        let _ = go.send(());
        placed(&mut app);
        let st = app.session.doc().unwrap();
        let kinds: Vec<Option<NodeId>> =
            st.doc.layers[0].children().unwrap().iter().map(|n| matches!(n.kind, vectorcraft_doc::NodeKind::Image(_)).then_some(n.id)).collect();
        let pictures: Vec<NodeId> = kinds.iter().flatten().copied().collect();
        assert!(pictures.len() == 2 + usize::from(third) && pictures.is_sorted(), "each picture is above the one before: {kinds:?}");
        assert_eq!(kinds.iter().position(Option::is_some), Some(2), "the pictures are above the square they were pasted on: {kinds:?}");
    }
}

/// A paste given beside the picture an independent paste left, while another paste waits, goes
/// beside that picture.
#[test]
fn a_paste_beside_a_finished_picture_does_not_follow_another_chain() {
    let (mut app, _) = pasting_app(true, None, None, true);
    let (go, hold) = channel();
    let bitmap = Some(Flavour { mime: PNG, data: picture(Png, 60, 40) });
    let clip = OtherApps { bitmap, text: None, converts: true, thread: Arc::new(Mutex::new(None)), holds: vec![None, Some(hold)] };
    app.services.system_clipboard = Some(Box::new(clip));
    let rect =
        |app: &mut VectorcraftApp, x: u32| app.run("shape.rectangle", json!({"x": x, "y": 10, "width": 20, "height": 20})).unwrap()["id"].clone();
    let (x, y) = (rect(&mut app, 10), rect(&mut app, 40));
    app.run("select.set", json!({"ids": [x]})).unwrap();
    crate::menus::invoke(&mut app, "edit.pasteInFront", json!({}));
    app.run("select.set", json!({"ids": [y]})).unwrap();
    crate::menus::invoke(&mut app, "edit.pasteInFront", json!({}));
    let t0 = std::time::Instant::now();
    while app.placing.pending.len() > 1 && t0.elapsed().as_secs() < 60 {
        poll(&mut app);
        std::thread::yield_now();
    }
    assert_eq!(app.placing.pending.len(), 1, "the first paste ran");
    // The first picture is selected.
    crate::menus::invoke(&mut app, "edit.pasteInFront", json!({}));
    let _ = go.send(());
    placed(&mut app);
    let st = app.session.doc().unwrap();
    let ids: Vec<Option<NodeId>> =
        st.doc.layers[0].children().unwrap().iter().map(|n| matches!(n.kind, vectorcraft_doc::NodeKind::Image(_)).then_some(n.id)).collect();
    let last = ids.iter().flatten().max().copied();
    assert_eq!(ids.iter().position(|id| *id == last && last.is_some()), Some(2), "the third picture is above the first: {ids:?}");
}

/// Pastes that follow each other in one document follow each other there, also when a paste in
/// another document runs between them.
#[test]
fn pastes_in_two_documents_follow_their_own() {
    let (mut app, _) = pasting_app(true, None, None, true);
    let (go, hold) = channel();
    let bitmap = Some(Flavour { mime: PNG, data: picture(Png, 60, 40) });
    let clip = OtherApps { bitmap, text: None, converts: true, thread: Arc::new(Mutex::new(None)), holds: vec![None, Some(hold)] };
    app.services.system_clipboard = Some(Box::new(clip));
    let rect =
        |app: &mut VectorcraftApp, x: u32| app.run("shape.rectangle", json!({"x": x, "y": 10, "width": 20, "height": 20})).unwrap()["id"].clone();
    let (_, b) = (rect(&mut app, 10), rect(&mut app, 40));
    app.run("select.set", json!({"ids": [b]})).unwrap();
    crate::menus::invoke(&mut app, "edit.pasteInFront", json!({}));
    crate::menus::invoke(&mut app, "edit.pasteInFront", json!({}));
    let t0 = std::time::Instant::now();
    while app.placing.pending.len() > 1 && t0.elapsed().as_secs() < 60 {
        poll(&mut app);
        std::thread::yield_now();
    }
    assert_eq!(app.placing.pending.len(), 1, "the first paste ran");
    // A paste in another document, then one more in the first.
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    let x = rect(&mut app, 10);
    app.run("select.set", json!({"ids": [x]})).unwrap();
    crate::menus::invoke(&mut app, "edit.pasteInFront", json!({}));
    app.session.set_active(0);
    app.sync_views();
    crate::menus::invoke(&mut app, "edit.pasteInFront", json!({}));
    let _ = go.send(());
    placed(&mut app);
    let st = &app.session.documents()[0];
    let kinds: Vec<Option<NodeId>> =
        st.doc.layers[0].children().unwrap().iter().map(|n| matches!(n.kind, vectorcraft_doc::NodeKind::Image(_)).then_some(n.id)).collect();
    let pictures: Vec<NodeId> = kinds.iter().flatten().copied().collect();
    assert!(pictures.len() == 3 && pictures.is_sorted(), "each picture is above the one before: {kinds:?}");
    assert_eq!(kinds.iter().position(Option::is_some), Some(2), "the pictures are above the square they were pasted on: {kinds:?}");
}

/// In Draw Inside in an isolated path, a bitmap decoded off the UI thread is pasted inside the path,
/// whose new clipping group stays isolated, when the user went back to Draw Normal meanwhile.
#[test]
fn a_bitmap_pasted_inside_an_isolated_path_stays_isolated() {
    let (mut app, _) = pasting_app(true, Some(Flavour { mime: PNG, data: picture(Png, 60, 40) }), None, true);
    let path = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 200, "height": 200})).unwrap()["id"].clone();
    app.run("object.isolate", json!({"id": path})).unwrap();
    app.run("select.set", json!({"ids": [path]})).unwrap();
    app.run("view.drawMode", json!({"mode": "inside"})).unwrap();
    crate::menus::invoke(&mut app, "edit.paste", json!({}));
    app.run("view.drawMode", json!({"mode": "normal"})).unwrap();
    placed(&mut app);
    let st = app.session.doc().unwrap();
    let pasted = st.selection.objects[0];
    let group = st.doc.parent_of(pasted);
    assert!(
        group.is_some_and(|g| st.doc.node(g).is_some_and(|n| n.children().is_some_and(|c| c.iter().any(|n| n.id.0 == path.as_u64().unwrap())))),
        "pasted inside the path"
    );
    assert_eq!(st.isolation, group, "the clipping group is isolated");
    assert_eq!(app.session.draw_mode, vectorcraft_engine::DrawMode::Normal, "Draw Normal stays");
}

/// A bitmap decoded off the UI thread is pasted into the layer that was current when the paste
/// was given, and a layer made current meanwhile stays current.
#[test]
fn a_bitmap_read_off_the_ui_thread_is_pasted_into_its_layer() {
    let (mut app, _) = pasting_app(true, Some(Flavour { mime: PNG, data: picture(Png, 60, 40) }), None, true);
    let first = app.session.doc().unwrap().doc.layers[0].id;
    app.run("layer.new", json!({})).unwrap();
    let second = app.session.doc().unwrap().current_layer().unwrap();
    app.run("layer.setCurrent", json!({"id": first.0})).unwrap();
    crate::menus::invoke(&mut app, "edit.paste", json!({}));
    app.run("layer.setCurrent", json!({"id": second.0})).unwrap();
    placed(&mut app);
    let st = app.session.doc().unwrap();
    let pasted = st.selection.objects[0];
    assert_eq!(st.doc.layer_containing(pasted), Some(first), "pasted into the layer current at the paste");
    assert_eq!(st.current_layer(), Some(second), "the layer made current meanwhile stays current");
    // With nothing selected, a Paste in Front given in another layer while the first waits
    // goes into that layer.
    app.run("layer.setCurrent", json!({"id": first.0})).unwrap();
    app.run("select.none", json!({})).unwrap();
    crate::menus::invoke(&mut app, "edit.pasteInFront", json!({}));
    app.run("layer.setCurrent", json!({"id": second.0})).unwrap();
    app.run("select.none", json!({})).unwrap();
    crate::menus::invoke(&mut app, "edit.pasteInFront", json!({}));
    assert_eq!(app.placing.pending.len(), 2);
    placed(&mut app);
    let st = app.session.doc().unwrap();
    let tops: Vec<_> = [first, second]
        .iter()
        .map(|l| st.doc.node(*l).and_then(|n| n.children()).and_then(|c| c.last()).map(|n| matches!(n.kind, vectorcraft_doc::NodeKind::Image(_))))
        .collect();
    assert_eq!(tops, [Some(true), Some(true)], "each layer has a picture on top");
    // A group isolated while the paste waits keeps its isolation, and the paste goes into the
    // layer, where it was given.
    let squares: Vec<Value> =
        (0..2).map(|i| app.run("shape.rectangle", json!({"x": 10 + 30 * i, "y": 10, "width": 20, "height": 20})).unwrap()["id"].clone()).collect();
    app.run("select.set", json!({"ids": squares})).unwrap();
    let group = NodeId(app.run("object.group", json!({})).unwrap()["id"].as_u64().unwrap());
    app.run("select.none", json!({})).unwrap();
    crate::menus::invoke(&mut app, "edit.paste", json!({}));
    app.run("object.isolate", json!({"id": group.0})).unwrap();
    placed(&mut app);
    let st = app.session.doc().unwrap();
    let pasted = st.selection.objects[0];
    assert_eq!(st.doc.parent_of(pasted), Some(second), "pasted into the layer, not the group isolated meanwhile");
    assert_eq!(st.isolation, Some(group), "the group stays isolated");
    // An object selected in another layer while the paste waits makes that layer current, and
    // it stays current after the paste.
    app.run("object.exitIsolation", json!({})).unwrap();
    let in_second = app.run("shape.rectangle", json!({"x": 300, "y": 10, "width": 20, "height": 20})).unwrap()["id"].clone();
    app.run("layer.setCurrent", json!({"id": first.0})).unwrap();
    let in_first = app.run("shape.rectangle", json!({"x": 300, "y": 100, "width": 20, "height": 20})).unwrap()["id"].clone();
    app.run("select.set", json!({"ids": [in_first]})).unwrap();
    crate::menus::invoke(&mut app, "edit.pasteInFront", json!({}));
    app.run("select.set", json!({"ids": [in_second]})).unwrap();
    assert_eq!(app.session.doc().unwrap().current_layer(), Some(second));
    placed(&mut app);
    let st = app.session.doc().unwrap();
    assert_eq!(st.doc.layer_containing(st.selection.objects[0]), Some(first), "pasted next to the object selected at the paste");
    assert_eq!(st.current_layer(), Some(second), "the layer of the object selected meanwhile stays current");
}

/// The Place dialog's OK with one file, with several (the place cursor and its thumbnails), a
/// drop on the canvas and [`crate::io::place_bytes`] read their files off the UI thread;
/// `file.place` and `file.place.queue` from MCP and scripts read them at once, with the replies
/// they have always had.
#[test]
fn what_the_user_places_is_read_off_the_ui_thread_and_commands_read_at_once() {
    let mut app = app(true);
    let (a, b) = (saved("routes", "a.tif", &picture(Tiff, 60, 40)), saved("routes", "b.bmp", &picture(Bmp, 60, 40)));
    let r = app.run("file.place", json!({"path": a})).unwrap();
    assert!(r["ids"].is_array() && app.placing.pending.is_empty(), "{r}");
    let r = app.run("file.place.queue", json!({"paths": [b]})).unwrap();
    assert!(r["count"] == 1 && app.placing.pending.is_empty(), "{r}");
    assert_eq!(app.place.thumbnails(), ["b.bmp"]);
    app.select_tool("selection");
    // The Place dialog's OK with one file.
    app.run("file.place", json!({"paths": [a]})).unwrap();
    assert_eq!(crate::dialogs::confirm(&mut app).unwrap()["background"], true);
    assert_eq!(app.placing.pending.len(), 1);
    placed(&mut app);
    assert_eq!(names(&app), ["a.tif", "a.tif"]);
    // With several: the place cursor is loaded once they are read.
    app.run("file.place", json!({"paths": [a, b]})).unwrap();
    assert_eq!(crate::dialogs::confirm(&mut app).unwrap()["background"], true);
    assert_eq!(app.ui.status, "Placing a.tif, b.bmp…");
    assert_eq!(app.session.tool_id(), "selection");
    let (inspect, _) = crate::control::ControlRequest::new("ui.inspect", json!({}));
    let crate::control::Outcome::Done(r) = crate::control::handle(&mut app, &egui::Context::default(), &inspect) else { panic!("not done") };
    assert_eq!(r["result"]["placing"], json!(["a.tif, b.bmp"]), "ui.inspect lists the files being read");
    placed(&mut app);
    assert_eq!((app.session.tool_id(), app.session.tool_options()["count"].as_u64()), ("place", Some(2)));
    assert_eq!(app.place.thumbnails(), ["a.tif", "b.bmp"]);
    app.select_tool("selection");
    // A drop and bytes placed without a path.
    let doc = app.session.doc().unwrap().uid;
    let drop = DropTarget::Place(DropAt { doc, at: vectorcraft_geom::Point::new(100.0, 100.0), embed: false });
    crate::place::drop_files(&mut app, vec![(drop, ("b.bmp".into(), Some(b.clone()), vec![]))]);
    crate::io::place_bytes(&mut app, "c.png", &picture(Png, 10, 10)).unwrap();
    assert_eq!(app.placing.pending.len(), 2);
    placed(&mut app);
    assert_eq!(names(&app), ["a.tif", "a.tif", "b.bmp", "c.png"]);
}

/// A click and a drag of the place cursor read their files off the UI thread, and place them as
/// a click and a drag place them when they are read at once.
#[test]
fn the_place_cursors_clicks_read_their_files_off_the_ui_thread() {
    use vectorcraft_tools::{PointerEvent, PointerKind};
    let paths = [saved("cursor", "a.tif", &picture(Tiff, 60, 40)), saved("cursor", "b.bmp", &picture(Bmp, 60, 40))];
    let view = vectorcraft_engine::ViewInfo::default();
    let press = |app: &mut VectorcraftApp, from: (f64, f64), to: (f64, f64)| {
        for (kind, (x, y)) in [(PointerKind::Down, from), (PointerKind::Drag, to), (PointerKind::Up, to)] {
            crate::canvas::dispatch(app, &PointerEvent::new(kind, x, y), view);
        }
    };
    let (mut at_once, mut app) = (app(false), app(true));
    for app in [&mut at_once, &mut app] {
        app.run("file.place.queue", json!({"paths": paths})).unwrap();
        press(app, (10.0, 10.0), (10.0, 10.0));
    }
    assert_eq!(names(&at_once), ["a.tif"]);
    assert!(names(&app).is_empty() && app.placing.pending.len() == 1, "nothing is placed until the file is read");
    assert_eq!(app.ui.status, "Placing a.tif…");
    placed(&mut app);
    assert!(state(&app) == state(&at_once), "the click places as at once");
    // A drag places the last file at the dragged size, and the previous tool comes back.
    for app in [&mut at_once, &mut app] {
        press(app, (100.0, 100.0), (180.0, 110.0));
    }
    placed(&mut app);
    assert!(state(&app) == state(&at_once), "the drag places as at once");
    assert_eq!(names(&app), ["a.tif", "b.bmp"]);
    assert_eq!(app.session.tool_id(), at_once.session.tool_id());
}

/// The Place dialog's line about each file is read off the UI thread: the lines are empty when
/// the dialog opens, and then show what the lines read at once show.
#[test]
fn the_place_dialogs_lines_are_read_off_the_ui_thread() {
    let p = json!({"paths": [saved("lines", "a.tif", &picture(Tiff, 60, 40)), saved("lines", "notes.xyz", b"hello")]});
    let mut at_once = app(false);
    at_once.run("file.place", p.clone()).unwrap();
    let want = at_once.ui.dialog.as_ref().unwrap().fields["__info"].clone();
    assert!(want[0].as_str().is_some_and(|l| l.starts_with("60 × 40 px")), "{want}");
    let mut app = app(true);
    app.run("file.place", p).unwrap();
    assert_eq!(app.ui.dialog.as_ref().unwrap().fields["__info"], json!(["", ""]));
    let t0 = std::time::Instant::now();
    loop {
        let Some(mut d) = app.ui.dialog.take() else { panic!("the dialog stays open") };
        let reading = crate::place::fill_lines(&mut app, &mut d);
        app.ui.dialog = Some(d);
        if !reading || t0.elapsed().as_secs() > 60 {
            break;
        }
        std::thread::yield_now();
    }
    assert_eq!(app.ui.dialog.as_ref().unwrap().fields["__info"], want);
}

/// Several files dropped together are placed in the order they were dropped, a large one first
/// included, as when they are read at once.
#[test]
fn files_dropped_together_are_placed_in_the_order_dropped() {
    let large = saved("order", "large.tif", &picture(Tiff, 1500, 1500));
    let small = [saved("order", "small1.png", &picture(Png, 8, 8)), saved("order", "small2.png", &picture(Png, 8, 8))];
    let mut app = app(true);
    let doc = app.session.doc().unwrap().uid;
    let drop = DropTarget::Place(DropAt { doc, at: vectorcraft_geom::Point::new(200.0, 150.0), embed: false });
    let files = [&large, &small[0], &small[1]].map(|p| (drop, (vectorcraft_engine::cmd::fileio::file_name(p), Some(p.clone()), vec![])));
    crate::place::drop_files(&mut app, files.into());
    assert_eq!(app.placing.pending.len(), 3);
    placed(&mut app);
    assert_eq!(names(&app), ["large.tif", "small1.png", "small2.png"]);
}

/// Files dropped together are read one at a time, as when they are read at once, so a batch of
/// large pictures is not decoded all at once.
#[test]
fn files_dropped_together_are_read_one_at_a_time() {
    let files: Vec<String> = (0..4).map(|i| saved("one-at-a-time", &format!("large{i}.tif"), &picture(Tiff, 1500, 1500))).collect();
    let mut app = app(true);
    let doc = app.session.doc().unwrap().uid;
    let drop = DropTarget::Place(DropAt { doc, at: vectorcraft_geom::Point::new(200.0, 150.0), embed: false });
    let dropped = files.iter().map(|p| (drop, (vectorcraft_engine::cmd::fileio::file_name(p), Some(p.clone()), vec![]))).collect();
    crate::place::drop_files(&mut app, dropped);
    placed(&mut app);
    assert_eq!(names(&app).len(), 4);
    assert_eq!(app.placing.reading[1].load(std::sync::atomic::Ordering::SeqCst), 1, "the most reads at once");
}

/// A PNG read off the UI thread is decoded there once: its pixels are in the image store the
/// renderers share before the canvas draws it, and the canvas's first frame with it decodes
/// nothing on the UI thread.
#[test]
fn a_placed_image_is_drawn_without_decoding_it_on_the_ui_thread() {
    // The image has pixels no other test uses, since every test shares the image store.
    let noise = image::RgbImage::from_fn(2000, 2000, |x, y| image::Rgb([(x % 251) as u8, (y % 241) as u8, ((x + y) % 239) as u8]));
    let mut png = vec![];
    noise.write_to(&mut std::io::Cursor::new(&mut png), Png).unwrap();
    let path = saved("seeded", "photo.png", &png);
    let mut app = app(true);
    start(&mut app, &json!({"path": path}), false).unwrap();
    placed(&mut app);
    assert_eq!(names(&app), ["photo.png"]);
    let st = app.session.doc().unwrap();
    let blob = st.doc.images.values().next().unwrap().clone();
    assert!(vectorcraft_render::image_cached(&blob), "kept before the canvas draws it");
    let decodes = vectorcraft_render::image_decodes();
    let ctx = egui::Context::default();
    let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0))), ..Default::default() };
    let mut out = ctx.run_ui(raw, |ui| crate::canvas::show(&mut app, ui));
    out.textures_delta.clear();
    assert!(app.canvas_rect.is_some(), "the canvas drew a frame");
    assert_eq!(vectorcraft_render::image_decodes(), decodes, "no decode on the UI thread");
}

/// A file that can't be read shows the message it shows when read at once, dropped or placed
/// from the dialog, and nothing is placed: a PNG cut to half its length, whose header reads but
/// whose pixels don't, too.
#[test]
fn a_file_that_cant_be_read_off_the_ui_thread_reports_as_at_once() {
    let gone = saved("unread", "gone.tif", b"");
    std::fs::remove_file(&gone).unwrap();
    let unknown = saved("unread", "notes.xyz", b"hello");
    let noise = image::RgbImage::from_fn(600, 300, |x, y| {
        let v = x.wrapping_mul(2_654_435_761) ^ y.wrapping_mul(40_503).rotate_left(13);
        image::Rgb([v as u8, (v >> 8) as u8, (v >> 16) as u8])
    });
    let mut png = vec![];
    noise.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    png.truncate(png.len() / 2);
    let cut = saved("unread", "cut.png", &png);
    for path in [gone, unknown, cut] {
        let p = json!({"path": path});
        let mut at_once = app(false);
        crate::place::run_for_user(&mut at_once, &p).unwrap_err();
        let mut app = app(true);
        let before = state(&app);
        crate::place::run_for_user(&mut app, &p).unwrap();
        placed(&mut app);
        assert_eq!(app.ui.status, at_once.ui.status);
        assert!(state(&app) == before, "{path}: nothing placed");
        // Dropped on the canvas.
        let name = vectorcraft_engine::cmd::fileio::file_name(&path);
        for app in [&mut at_once, &mut app] {
            let doc = app.session.doc().unwrap().uid;
            let drop = DropTarget::Place(DropAt { doc, at: vectorcraft_geom::Point::new(100.0, 100.0), embed: false });
            crate::place::drop_files(app, vec![(drop, (name.clone(), Some(path.clone()), vec![]))]);
            placed(app);
        }
        assert_eq!(app.ui.status, at_once.ui.status);
        assert!(app.ui.status.starts_with(&format!("Couldn't place {name}: ")), "{}", app.ui.status);
        if name == "cut.png" {
            assert_eq!(app.ui.status, "Couldn't place cut.png: the image's pixels can't be read");
        }
    }
}

/// A file whose document closed before it was read is not placed, and the status bar says why.
#[test]
fn a_file_whose_document_closed_meanwhile_is_not_placed() {
    let path = saved("closed", "photo.tif", &picture(Tiff, 60, 40));
    for dropped in [false, true] {
        let mut app = app(true);
        app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
        start(&mut app, &json!({"path": path}), dropped).unwrap();
        app.session.execute("file.close", &json!({})).unwrap();
        let left = state(&app);
        placed(&mut app);
        let why = if dropped { "dropped on" } else { "placed in" };
        assert_eq!(app.ui.status, format!("Couldn't place photo.tif: the document it was {why} was closed"));
        assert!(state(&app) == left, "the other document is unchanged");
    }
}

/// Replace swaps the object selected when Place started; when that object was deleted before the
/// file was read, nothing is placed and the status bar says why.
#[test]
fn a_replace_whose_object_was_deleted_meanwhile_places_nothing() {
    let path = saved("replace", "photo.tif", &picture(Tiff, 60, 40));
    let mut app = app(true);
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
    crate::place::run_for_user(&mut app, &json!({"path": path, "replace": true})).unwrap();
    app.run("edit.clear", json!({})).unwrap();
    let cleared = state(&app);
    placed(&mut app);
    assert!(app.ui.status.contains("the object to replace was changed or deleted"), "{}", app.ui.status);
    assert!(state(&app) == cleared, "nothing placed");
}

/// A file read while a dialog is open waits for the dialog to close, as a document that opens
/// does (#1694): the dialog's OK acts on the active document.
#[test]
fn a_file_read_while_a_dialog_is_open_waits_for_it_to_close() {
    let path = saved("dialog", "photo.tif", &picture(Tiff, 60, 40));
    let mut app = app(true);
    start(&mut app, &json!({"path": path}), false).unwrap();
    app.ui.dialog = Some(crate::state::Dialog::new(crate::dialogs::save_options::KIND, json!({"path": "/art/First.vectorcraft"})));
    let t0 = std::time::Instant::now();
    while t0.elapsed().as_millis() < 400 {
        poll(&mut app);
        std::thread::yield_now();
    }
    assert!(names(&app).is_empty() && app.placing.pending.len() == 1, "the file waits for the dialog");
    app.ui.dialog = None;
    placed(&mut app);
    assert_eq!(names(&app), ["photo.tif"]);
}

/// A file read while an object is being dragged, in its document or in another one, waits for
/// the drag to end: the drag is not canceled, and the placed art stays.
#[test]
fn a_file_read_during_a_drag_waits_for_it_to_end() {
    use vectorcraft_tools::{PointerEvent, PointerKind};
    let path = saved("drag", "photo.tif", &picture(Tiff, 60, 40));
    for other_document in [false, true] {
        let mut app = app(true);
        start(&mut app, &json!({"path": path, "at": [300, 200]}), false).unwrap();
        if other_document {
            app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        }
        let square = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap();
        let dragged = app.session.active_index();
        app.select_tool("selection");
        let view = vectorcraft_engine::ViewInfo::default();
        for (kind, x) in [(PointerKind::Down, 20.0), (PointerKind::Drag, 40.0), (PointerKind::Drag, 60.0)] {
            app.session.pointer(&PointerEvent::new(kind, x, 20.0), view).unwrap();
        }
        assert!(app.session.in_interaction());
        let t0 = std::time::Instant::now();
        while t0.elapsed().as_millis() < 400 {
            poll(&mut app);
            std::thread::yield_now();
        }
        assert!(app.placing.pending.len() == 1 && app.session.in_interaction(), "other document: {other_document}");
        // The drag moves on before it ends.
        for kind in [PointerKind::Drag, PointerKind::Up] {
            app.session.pointer(&PointerEvent::new(kind, 60.0, 20.0), view).unwrap();
        }
        placed(&mut app);
        let moved = app.session.documents()[dragged.unwrap()].doc.node(NodeId(square)).and_then(|n| n.geometric_bounds());
        assert_eq!(moved.map(|b| (b.x0, b.y0)), Some((50.0, 10.0)), "other document: {other_document}");
        assert_eq!(names(&app), ["photo.tif"], "other document: {other_document}");
    }
}

/// A file read for one document while the place cursor is dragged in another waits for the drag
/// to end, which the place cursor keeps to itself: the drag still places its file.
#[test]
fn a_file_read_during_a_place_cursor_drag_waits_for_it_to_end() {
    use vectorcraft_tools::{PointerEvent, PointerKind};
    let path = saved("cursor-drag", "photo.tif", &picture(Tiff, 60, 40));
    let cursor = [saved("cursor-drag", "a.png", &picture(Png, 8, 8)), saved("cursor-drag", "b.png", &picture(Png, 8, 8))];
    let mut app = app(true);
    start(&mut app, &json!({"path": path, "at": [300, 200]}), false).unwrap();
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.run("file.place.queue", json!({"paths": cursor})).unwrap();
    let view = vectorcraft_engine::ViewInfo::default();
    for (kind, x) in [(PointerKind::Down, 20.0), (PointerKind::Drag, 60.0)] {
        crate::canvas::dispatch(&mut app, &PointerEvent::new(kind, x, 20.0), view);
    }
    assert!(app.session.tool_busy() && !app.session.in_interaction());
    let t0 = std::time::Instant::now();
    while t0.elapsed().as_millis() < 400 {
        poll(&mut app);
        std::thread::yield_now();
    }
    assert!(app.placing.pending.len() == 1 && app.session.tool_busy(), "the file waits for the drag");
    crate::canvas::dispatch(&mut app, &PointerEvent::new(PointerKind::Up, 60.0, 20.0), view);
    placed(&mut app);
    let placed_in = |i: usize| -> Vec<(String, Option<vectorcraft_geom::Rect>)> {
        let st = &app.session.documents()[i];
        st.doc
            .layers
            .iter()
            .flat_map(|l| l.children().into_iter().flatten())
            .map(|n| (n.name.clone().unwrap_or_default(), n.geometric_bounds()))
            .collect()
    };
    assert_eq!(placed_in(0).iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(), ["photo.tif"]);
    assert_eq!(placed_in(1), [("a.png".to_string(), Some(vectorcraft_geom::Rect::new(20.0, 20.0, 60.0, 60.0)))], "the drag placed its file");
}

/// A file read while a tool waits between its clicks (the Dimension tool after its first point),
/// in the file's document or in another one, is placed without waiting for the next click. In
/// the same document the tool keeps its point; in another one it starts afresh, as when the user
/// switches documents.
#[test]
fn a_file_read_between_a_tools_clicks_is_placed() {
    use vectorcraft_tools::{PointerEvent, PointerKind};
    let path = saved("clicks", "photo.tif", &picture(Tiff, 60, 40));
    for other_document in [false, true] {
        let mut app = app(true);
        start(&mut app, &json!({"path": path, "at": [300, 200]}), false).unwrap();
        if other_document {
            app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        }
        app.select_tool("dimension");
        let view = vectorcraft_engine::ViewInfo::default();
        for kind in [PointerKind::Down, PointerKind::Up] {
            crate::canvas::dispatch(&mut app, &PointerEvent::new(kind, 20.0, 20.0), view);
        }
        assert!(app.session.tool_busy() && !app.session.in_interaction(), "other document: {other_document}");
        placed(&mut app);
        assert_eq!(names(&app), ["photo.tif"], "other document: {other_document}");
        assert_eq!(app.session.tool_busy(), !other_document, "other document: {other_document}");
    }
}
