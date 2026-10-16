//! The system clipboard's formats through a headless fake service (M4.57): what Copy publishes,
//! what Paste takes from other apps (bitmaps, PDF, text), the Paste menu and keyboard paste.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::mpsc::{Sender, channel};

use serde_json::{Value, json};
use vectorcraft_doc::{Node, NodeKind};
use vectorcraft_engine::Session;
use vectorcraft_engine::cmd::clipboard::{BITMAP, Flavour, PDF, PNG, SVG, TEXT};
use vectorcraft_geom::Point;
use vectorcraft_tools::{PointerEvent, PointerKind};

use crate::{Services, SystemClipboard, VectorcraftApp, background, menus};

/// The fake clipboard's contents: what the app wrote, or what "another app" copied.
#[derive(Default)]
pub(super) struct Board {
    flavours: Vec<Flavour>,
    ours: bool,
    /// How many times the app wrote it.
    writes: usize,
}

struct Fake(Rc<RefCell<Board>>);

impl Fake {
    fn find(&self, mimes: &[&'static str]) -> Option<Flavour> {
        let b = self.0.borrow();
        let matches = |m: &str, f: &Flavour| f.mime == m || (m == BITMAP && f.mime.starts_with("image/") && f.mime != SVG);
        mimes.iter().find_map(|m| b.flavours.iter().find(|f| matches(m, f)).cloned())
    }
}

impl SystemClipboard for Fake {
    fn write(&mut self, flavours: &[Flavour]) -> Result<(), String> {
        let mut b = self.0.borrow_mut();
        *b = Board { flavours: flavours.to_vec(), ours: true, writes: b.writes + 1 };
        Ok(())
    }
    fn holds_ours(&mut self) -> bool {
        self.0.borrow().ours
    }
    fn read(&mut self, mimes: &[&'static str]) -> Option<Flavour> {
        self.find(mimes)
    }
    fn has(&mut self, mimes: &[&'static str]) -> bool {
        self.find(mimes).is_some()
    }
}

/// An app with a document and a fake system clipboard, the view centred on (250, 180).
pub(super) fn app() -> (VectorcraftApp, Rc<RefCell<Board>>) {
    let board = Rc::new(RefCell::new(Board::default()));
    let services = Services { system_clipboard: Some(Box::new(Fake(board.clone()))), ..Default::default() };
    let mut app = VectorcraftApp::new(Session::new(), services);
    run(&mut app, "file.new", json!({"width": 400, "height": 300}));
    let v = app.view_mut().unwrap();
    (v.center, v.fitted) = (Point::new(250.0, 180.0), true);
    (app, board)
}

/// Another app copies `flavours`.
pub(super) fn copy_elsewhere(board: &Rc<RefCell<Board>>, flavours: Vec<(&'static str, Vec<u8>)>) {
    let flavours = flavours.into_iter().map(|(mime, data)| Flavour { mime, data }).collect();
    let mut b = board.borrow_mut();
    *b = Board { flavours, ours: false, writes: b.writes };
}

pub(super) fn run(app: &mut VectorcraftApp, id: &str, p: Value) -> Value {
    app.run(id, p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

pub(super) fn copy_rect(app: &mut VectorcraftApp) {
    let id = run(app, "shape.rectangle", json!({"x": 10, "y": 10, "width": 40, "height": 20}))["id"].clone();
    run(app, "paint.setFill", json!({"color": "#ff0000"}));
    run(app, "select.set", json!({"ids": [id]}));
    run(app, "edit.copy", json!({}));
}

fn written(board: &Rc<RefCell<Board>>) -> Vec<&'static str> {
    board.borrow().flavours.iter().map(|f| f.mime).collect()
}

/// The objects the last paste selected.
pub(super) fn pasted(app: &VectorcraftApp) -> Vec<Node> {
    let st = app.session.active().unwrap();
    st.selection.objects.iter().map(|id| st.doc.node(*id).unwrap().clone()).collect()
}

fn count(app: &VectorcraftApp) -> usize {
    app.session.active().unwrap().doc.layers[0].children().unwrap().len()
}

/// A 4 × 2 px blue PNG.
pub(super) fn blue_png() -> Vec<u8> {
    let img = image::RgbaImage::from_pixel(4, 2, image::Rgba([0, 0, 255, 255]));
    let mut png = vec![];
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    png
}

/// One headless frame of app logic (shortcuts too, from the second frame on) delivering `events`,
/// in the test's one context (in a new one the app starts its fonts over).
fn frame(app: &mut VectorcraftApp, events: Vec<egui::Event>) {
    thread_local! {
        static CTX: egui::Context = egui::Context::default();
    }
    let ctx = CTX.with(Clone::clone);
    ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| app.logic(ui.ctx())).textures_delta.clear();
}

fn key(key: egui::Key, pressed: bool, modifiers: egui::Modifiers) -> egui::Event {
    egui::Event::Key { key, physical_key: None, pressed, repeat: false, modifiers }
}

/// [`app`] with the background worker of a host that writes from any thread, as the desktop app's.
fn app_with_worker() -> (VectorcraftApp, Rc<RefCell<Board>>) {
    let (mut app, board) = app();
    app.services.write_shared = Some(Arc::new(|_: &str, _: &[u8]| Err("no files are written here".into())));
    (app, board)
}

/// [`app_with_worker`], its worker busy with a job labeled `label` (none: another Copy's formats)
/// until the sender is dropped, or for 10 s.
fn app_with_busy_worker(label: &str) -> (VectorcraftApp, Rc<RefCell<Board>>, Sender<()>) {
    let (mut app, board) = app_with_worker();
    let hold = hold_worker(&mut app, label);
    (app, board, hold)
}

/// Start a job labeled `label` that holds the worker until the sender is dropped, or for 10 s.
fn hold_worker(app: &mut VectorcraftApp, label: &str) -> Sender<()> {
    let (hold, held) = channel::<()>();
    let work = move |_: background::Writer| {
        let _ = held.recv_timeout(std::time::Duration::from_secs(10));
        Ok(Value::Null)
    };
    background::run(app, true, label.into(), None, work, |_, r| r).unwrap();
    hold
}

/// One headless frame of the whole window (app logic, then layout), in a context this test thread
/// keeps for these frames; `copy`: text copied through egui in it, as a text field or the Type
/// tool copies.
fn window_frame(app: &mut VectorcraftApp, copy: Option<&str>) {
    thread_local! {
        static CTX: egui::Context = egui::Context::default();
    }
    let ctx = CTX.with(Clone::clone);
    let screen_rect = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600.0, 850.0)));
    let mut out = ctx.run_ui(egui::RawInput { screen_rect, ..Default::default() }, |ui| {
        app.logic(ui.ctx());
        if let Some(text) = copy {
            ui.ctx().copy_text(text.into());
        }
        app.ui(ui);
    });
    out.textures_delta.clear();
}

#[test]
fn copy_publishes_pdf_to_the_service_only_with_the_preference_on() {
    let (mut app, board) = app();
    copy_rect(&mut app);
    assert_eq!(written(&board), [TEXT, SVG, PNG]);
    assert!(String::from_utf8_lossy(&board.borrow().flavours[0].data).contains("<svg"));
    run(&mut app, "prefs.set", json!({"key": "copyAsPdf", "value": true}));
    run(&mut app, "edit.cut", json!({}));
    assert_eq!(written(&board), [TEXT, SVG, PDF, PNG]);
    assert!(board.borrow().flavours[2].data.starts_with(b"%PDF"));
    // Nothing goes through egui's text output when the service is there.
    assert!(app.clipboard_out.is_none());
}

#[test]
fn our_own_copy_pastes_the_lossless_internal_clipboard() {
    let (mut app, board) = app();
    copy_rect(&mut app);
    // As if the clipboard held only the PNG: still ours, so the rectangle comes back as a path.
    board.borrow_mut().flavours.retain(|f| f.mime == PNG);
    run(&mut app, "edit.paste", json!({}));
    assert!(matches!(pasted(&app)[0].kind, NodeKind::Path { .. }));
}

/// Copy returns before the system clipboard's formats are made: the background worker makes them
/// (here after another Copy's) and they are written when it is done. A paste meanwhile takes what
/// was copied, and neither the status bar nor `ui.inspect` shows the work.
#[test]
fn copy_publishes_the_formats_the_worker_makes_and_pastes_meanwhile() {
    let (mut app, board, hold) = app_with_busy_worker("");
    app.status("Ready");
    copy_elsewhere(&board, vec![(TEXT, b"Words another app copied".to_vec())]);
    copy_rect(&mut app);
    assert_eq!(written(&board), [TEXT], "Copy wrote nothing itself");
    assert_eq!(app.ui.status, "Ready");
    let bar = crate::tests_background::status_bar_texts(&mut app);
    assert!(bar.iter().any(|t| t == "Ready") && !bar.iter().any(|t| t.contains('…')), "{bar:?}");
    assert_eq!(crate::control::inspect(&app, &egui::Context::default())["background"], json!([]));
    run(&mut app, "edit.paste", json!({}));
    assert!(matches!(pasted(&app)[0].kind, NodeKind::Path { .. }), "the copied rectangle, not the other app's text");
    drop(hold);
    background::wait_all(&mut app);
    assert_eq!(written(&board), [TEXT, SVG, PNG]);
    assert_eq!(board.borrow().flavours, app.session.clipboard_flavours());
    assert_eq!(app.ui.status, "Ready");
}

/// Two copies made before the worker gets to them: only the later one's formats are made and
/// written, and the earlier one ends without a message.
#[test]
fn a_later_copy_replaces_formats_not_written_yet() {
    let (mut app, board, hold) = app_with_busy_worker("");
    app.status("Ready");
    copy_rect(&mut app);
    let first = app.session.clipboard_flavours();
    let id = run(&mut app, "shape.ellipse", json!({"x": 100, "y": 100, "width": 30, "height": 30}))["id"].clone();
    run(&mut app, "select.set", json!({"ids": [id]}));
    run(&mut app, "edit.cut", json!({}));
    drop(hold);
    background::wait_all(&mut app);
    let b = board.borrow();
    assert_eq!(b.writes, 1);
    assert_eq!(b.flavours, app.session.clipboard_flavours());
    assert_ne!(b.flavours, first);
    assert_eq!(app.ui.status, "Ready");
}

/// While the worker saves (or exports, prints, keeps a recovery copy), Copy makes and writes the
/// formats at once: they don't wait for that work.
#[test]
fn a_copy_while_the_worker_saves_writes_the_formats_at_once() {
    let (mut app, board, hold) = app_with_busy_worker("Saving a.vectorcraft");
    copy_elsewhere(&board, vec![(TEXT, b"Words another app copied".to_vec())]);
    copy_rect(&mut app);
    assert_eq!(written(&board), [TEXT, SVG, PNG]);
    assert_eq!(board.borrow().flavours, app.session.clipboard_flavours());
    assert_eq!(app.background.jobs.len(), 1, "only the save");
    drop(hold);
    background::wait_all(&mut app);
    assert_eq!(board.borrow().writes, 1);
}

/// Text copied in a field or with the Type tool after Copy, before the formats are written: they
/// are not written, the system clipboard keeps the text, and Paste takes it.
#[test]
fn text_copied_before_the_formats_are_written_stays_on_the_clipboard() {
    let (mut app, board, hold) = app_with_busy_worker("");
    window_frame(&mut app, None);
    window_frame(&mut app, None);
    copy_rect(&mut app);
    window_frame(&mut app, Some("Words copied in a field"));
    // What the host puts on the system clipboard for that copy.
    copy_elsewhere(&board, vec![(TEXT, b"Words copied in a field".to_vec())]);
    drop(hold);
    background::wait_all(&mut app);
    assert_eq!(board.borrow().writes, 0);
    run(&mut app, "edit.paste", json!({}));
    let NodeKind::Text(t) = &pasted(&app)[0].kind else { panic!("not type: {:?}", pasted(&app)[0].kind) };
    assert_eq!(t.plain_text(), "Words copied in a field");
}

/// The Edit menu's Paste into a text field after Copy of type, before the formats are written: it
/// waits for them, not for a save started after them, and pastes the type's text, not what
/// another app copied before.
#[test]
fn the_menus_paste_into_a_field_waits_for_the_formats() {
    let (mut app, board) = app_with_worker();
    let t = run(&mut app, "text.create", json!({"x": 20, "y": 250, "text": "Words"}))["id"].clone();
    run(&mut app, "select.set", json!({"ids": [t]}));
    copy_elsewhere(&board, vec![(TEXT, b"Words another app copied".to_vec())]);
    run(&mut app, "edit.copy", json!({}));
    let hold = hold_worker(&mut app, "Saving a.vectorcraft");
    // A text field with the keyboard; in the second frame Paste is chosen from the menu.
    let (ctx, id, mut field) = (egui::Context::default(), egui::Id::new("field"), String::new());
    for paste in [false, true] {
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            if paste {
                menus::invoke_from_system_menu(&mut app, ui.ctx(), "edit.paste", json!({}));
            }
            ui.add(egui::TextEdit::singleline(&mut field).id(id)).request_focus();
        });
        out.textures_delta.clear();
    }
    assert_eq!(field, "Words");
    assert_eq!(app.background.jobs.len(), 1, "the save is still running");
    drop(hold);
    background::wait_all(&mut app);
}

/// A paste chord into type being edited, before the formats are written: the formats are written
/// first, and the paste pastes their text whether the host read another app's text for it or found
/// no text, also when the chord's key comes up a frame after they were written; with no text in
/// the formats (Include SVG Code off, a rectangle) it pastes nothing.
#[test]
fn a_paste_chord_into_type_waits_for_the_formats() {
    let (mut app, board) = app_with_worker();
    frame(&mut app, vec![]);
    frame(&mut app, vec![]);
    let texts = |app: &VectorcraftApp| -> Vec<String> {
        let layer = &app.session.active().unwrap().doc.layers[0];
        layer.children().unwrap().iter().filter_map(|n| if let NodeKind::Text(t) = &n.kind { Some(t.plain_text()) } else { None }).collect()
    };
    // The chord's release comes after the Paste event egui sends when the clipboard holds text.
    let release = || key(egui::Key::V, false, egui::Modifiers::COMMAND);
    let read = || vec![egui::Event::Paste("Words another app copied".into()), release()];
    let paste_into_new_type = |app: &mut VectorcraftApp, y: f64, frames: Vec<Vec<egui::Event>>| {
        app.select_tool("type");
        let view = app.view_info();
        for kind in [PointerKind::Down, PointerKind::Up] {
            app.session.pointer(&PointerEvent::new(kind, 100.0, y), view).unwrap();
        }
        assert!(app.session.tool_wants_text());
        for events in frames {
            frame(app, events);
        }
        app.select_tool("selection");
    };
    let t = run(&mut app, "text.create", json!({"x": 20, "y": 250, "text": "Words"}))["id"].clone();
    run(&mut app, "select.set", json!({"ids": [t]}));
    copy_elsewhere(&board, vec![(TEXT, b"Words another app copied".to_vec())]);
    run(&mut app, "edit.copy", json!({}));
    paste_into_new_type(&mut app, 100.0, vec![read()]);
    assert_eq!(texts(&app), ["Words", "Words"], "the host read another app's text");
    run(&mut app, "select.set", json!({"ids": [t]}));
    copy_elsewhere(&board, vec![(PNG, blue_png())]);
    run(&mut app, "edit.copy", json!({}));
    paste_into_new_type(&mut app, 130.0, vec![vec![release()]]);
    assert_eq!(texts(&app), ["Words", "Words", "Words"], "the host found no text");
    // The key goes down before the formats are written (the host finds no text) and comes up in
    // the frame after the one that writes them.
    run(&mut app, "select.set", json!({"ids": [t]}));
    copy_elsewhere(&board, vec![(PNG, blue_png())]);
    run(&mut app, "edit.copy", json!({}));
    paste_into_new_type(&mut app, 190.0, vec![vec![], vec![release()]]);
    assert_eq!(texts(&app), ["Words", "Words", "Words", "Words"], "the key came up a frame later");
    run(&mut app, "prefs.set", json!({"key": "copyAsSvg", "value": false}));
    copy_elsewhere(&board, vec![(TEXT, b"Words another app copied".to_vec())]);
    copy_rect(&mut app);
    paste_into_new_type(&mut app, 160.0, vec![read()]);
    assert_eq!(written(&board), [PNG]);
    assert!(!texts(&app).iter().any(|t| t.contains("another app")), "{:?}", texts(&app));
}

/// A paste key on the canvas before the formats are written pastes the internal clipboard at once,
/// without waiting for them.
#[test]
fn a_paste_key_on_the_canvas_does_not_wait_for_the_formats() {
    let (mut app, board, hold) = app_with_busy_worker("");
    frame(&mut app, vec![]);
    frame(&mut app, vec![]);
    copy_elsewhere(&board, vec![(TEXT, b"Words another app copied".to_vec())]);
    copy_rect(&mut app);
    frame(&mut app, vec![egui::Event::Paste("Words another app copied".into())]);
    assert!(matches!(pasted(&app)[0].kind, NodeKind::Path { .. }), "the copied rectangle");
    assert_eq!(written(&board), [TEXT], "the formats are not written yet");
    drop(hold);
    background::wait_all(&mut app);
}

/// The formats the worker makes from its copy of the clipboard are the ones the session makes,
/// byte for byte: a rectangle; a group with a gradient, a drop shadow and 50% opacity; an
/// embedded image; type alone (its text); and everything with On Copy: PDF.
#[test]
fn the_worker_makes_the_formats_the_session_makes() {
    let (mut app, board) = app_with_worker();
    // Copy the selection, then compare the formats the worker wrote with the session's. A PDF is
    // dated to the second it is made in, so when formats with a PDF differ and the clock has moved
    // to another second since the Copy, they are compared again after another Copy.
    let check = |app: &mut VectorcraftApp, what: &str, mimes: &[&str]| {
        for _ in 0..100 {
            let second = vectorcraft_doc::metadata::now_unix();
            run(app, "edit.copy", json!({}));
            assert_eq!(app.background.jobs.len(), 1, "{what}: made on the worker");
            background::wait_all(app);
            assert_eq!(written(&board), mimes, "{what}");
            let session = app.session.clipboard_flavours();
            let dated = mimes.contains(&PDF) && vectorcraft_doc::metadata::now_unix() != second;
            if board.borrow().flavours == session || !dated {
                assert_eq!(board.borrow().flavours, session, "{what}");
                return;
            }
        }
        panic!("{what}: the clock moved to another second during each of 100 copies");
    };
    let r = run(&mut app, "shape.rectangle", json!({"x": 10, "y": 10, "width": 40, "height": 20}))["id"].clone();
    run(&mut app, "paint.setFill", json!({"color": "#ff0000"}));
    run(&mut app, "select.set", json!({"ids": [r]}));
    check(&mut app, "a rectangle", &[TEXT, SVG, PNG]);
    let e = run(&mut app, "shape.ellipse", json!({"x": 60, "y": 60, "width": 80, "height": 50}))["id"].clone();
    run(&mut app, "paint.setFill", json!({"gradient": {"kind": "linear"}}));
    let s = run(&mut app, "shape.star", json!({"cx": 200, "cy": 100, "radius1": 40, "radius2": 20}))["id"].clone();
    run(&mut app, "select.set", json!({"ids": [e, s]}));
    run(&mut app, "object.group", json!({}));
    run(&mut app, "effect.apply", json!({"effect": "stylize.dropShadow", "params": {"x": 3}}));
    run(&mut app, "object.setProps", json!({"opacity": 50}));
    check(&mut app, "a group", &[TEXT, SVG, PNG]);
    run(&mut app, "clipboard.importImage", json!({"dataBase64": vectorcraft_format::base64_encode(&blue_png())}));
    run(&mut app, "edit.paste", json!({}));
    check(&mut app, "an embedded image", &[TEXT, SVG, PNG]);
    let t = run(&mut app, "text.create", json!({"x": 20, "y": 250, "text": "Words"}))["id"].clone();
    run(&mut app, "select.set", json!({"ids": [t]}));
    check(&mut app, "type alone", &[TEXT, SVG, PNG]);
    assert_eq!(board.borrow().flavours[0].data, b"Words");
    run(&mut app, "prefs.set", json!({"key": "copyAsPdf", "value": true}));
    run(&mut app, "select.all", json!({}));
    check(&mut app, "everything, with On Copy: PDF", &[TEXT, SVG, PDF, PNG]);
}

#[test]
fn a_bitmap_from_another_app_pastes_as_an_embedded_image_at_the_view_centre() {
    let (mut app, board) = app();
    copy_rect(&mut app);
    copy_elsewhere(&board, vec![(PNG, blue_png())]);
    run(&mut app, "edit.paste", json!({}));
    let n = &pasted(&app)[0];
    let NodeKind::Image(im) = &n.kind else { panic!("not an image: {:?}", n.kind) };
    assert!(im.link.is_none() && (im.width, im.height) == (4, 2));
    let b = n.geometric_bounds().unwrap();
    assert!((b.center() - Point::new(250.0, 180.0)).hypot() < 1e-6, "{b:?}");
}

#[test]
fn text_from_another_app_pastes_as_point_text_and_svg_text_as_art() {
    let (mut app, board) = app();
    // Word processors offer a picture of the text too: the text wins.
    copy_elsewhere(&board, vec![(TEXT, b"Pasted words".to_vec()), (PNG, blue_png())]);
    run(&mut app, "edit.paste", json!({}));
    let NodeKind::Text(t) = &pasted(&app)[0].kind else { panic!("not type") };
    assert_eq!(t.plain_text(), "Pasted words");
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg"><circle cx="5" cy="5" r="5"/><circle cx="20" cy="5" r="5"/></svg>"##;
    copy_elsewhere(&board, vec![(TEXT, svg.as_bytes().to_vec())]);
    run(&mut app, "edit.paste", json!({}));
    assert_eq!(pasted(&app).len(), 2);
    assert!(pasted(&app).iter().all(|n| matches!(n.kind, NodeKind::Path { .. })));
    // Blank text beside a picture is the picture. Text alone that can't be pasted adds nothing.
    copy_elsewhere(&board, vec![(TEXT, b"  \r\n".to_vec()), (PNG, blue_png())]);
    run(&mut app, "edit.paste", json!({}));
    assert!(matches!(pasted(&app)[0].kind, NodeKind::Image(_)), "blank text yields the picture");
    let before = count(&app);
    copy_elsewhere(&board, vec![(TEXT, b"  \r\n".to_vec())]);
    assert!(app.run("edit.paste", json!({})).is_err());
    assert!(app.ui.status.starts_with("Couldn't paste"), "{}", app.ui.status);
    assert_eq!(count(&app), before);
}

/// A browser's Copy Image: the picture with its address as text (and HTML, which Paste doesn't
/// read), or its markup as text. The picture is pasted; an address alone pastes as text, nothing
/// is fetched (#597, #1089).
#[test]
fn an_image_copied_in_a_browser_pastes_as_the_picture_not_its_address() {
    let (mut app, board) = app();
    let url = b"https://example.com/images/cat.png".to_vec();
    copy_elsewhere(&board, vec![(TEXT, url.clone()), ("text/html", b"<img src=\"https://example.com/images/cat.png\">".to_vec()), (PNG, blue_png())]);
    run(&mut app, "edit.paste", json!({}));
    let NodeKind::Image(im) = &pasted(&app)[0].kind else { panic!("not an image: {:?}", pasted(&app)[0].kind) };
    assert!(im.link.is_none() && (im.width, im.height) == (4, 2));
    // Some browsers (on Linux) put the image's markup there as its text: the picture still (#1089).
    let markup = br#"<img src="https://example.com/images/cat.png" jsaction="" class="sFlh5c" style="max-width: 351px" alt="">"#.to_vec();
    copy_elsewhere(&board, vec![(TEXT, markup), (PNG, blue_png())]);
    run(&mut app, "edit.paste", json!({}));
    assert!(matches!(pasted(&app)[0].kind, NodeKind::Image(_)), "{:?}", pasted(&app)[0].kind);
    copy_elsewhere(&board, vec![(TEXT, url)]);
    run(&mut app, "edit.paste", json!({}));
    let NodeKind::Text(t) = &pasted(&app)[0].kind else { panic!("not type") };
    assert_eq!(t.plain_text(), "https://example.com/images/cat.png");
}

#[test]
fn a_pdf_from_another_app_pastes_as_vectors_ahead_of_its_bitmap() {
    let (mut app, board) = app();
    let mut other = Session::new();
    other.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    let id = other.execute("shape.ellipse", &json!({"x": 0, "y": 0, "width": 30, "height": 30})).unwrap()["id"].clone();
    other.execute("select.set", &json!({"ids": [id]})).unwrap();
    other.execute("edit.copy", &json!({})).unwrap();
    let pdf = other.clipboard_pdf().unwrap().unwrap();
    copy_elsewhere(&board, vec![(PNG, blue_png()), (PDF, pdf)]);
    run(&mut app, "edit.paste", json!({}));
    let p = pasted(&app);
    assert!(!p.is_empty() && p.iter().all(|n| !matches!(n.kind, NodeKind::Image(_))), "{p:?}");
}

#[test]
fn a_bitmap_on_the_system_clipboard_enables_paste() {
    let (mut app, board) = app();
    copy_elsewhere(&board, vec![(PNG, blue_png())]);
    assert!(!menus::enabled(&app, "edit.paste"), "not looked at before a frame");
    frame(&mut app, vec![]);
    assert!(menus::enabled(&app, "edit.paste") && menus::enabled(&app, "edit.pasteInPlace"));
    menus::invoke(&mut app, "edit.paste", json!({}));
    assert!(matches!(pasted(&app)[0].kind, NodeKind::Image(_)));
}

#[test]
fn keyboard_paste_of_a_bitmap_without_a_paste_event() {
    let (mut app, board) = app();
    let (cmd, none) = (egui::Modifiers::COMMAND, egui::Modifiers::NONE);
    copy_elsewhere(&board, vec![(PNG, blue_png())]);
    frame(&mut app, vec![]);
    // Cmd+V: egui swallows the press and sends no Paste event (no text); only the release arrives.
    frame(&mut app, vec![key(egui::Key::V, false, cmd)]);
    assert_eq!(count(&app), 1);
    assert!(matches!(pasted(&app)[0].kind, NodeKind::Image(_)));
    // A V typed (its press reported) is no paste, whatever is released with it.
    frame(&mut app, vec![key(egui::Key::V, true, none)]);
    frame(&mut app, vec![key(egui::Key::V, false, cmd)]);
    assert_eq!(count(&app), 1);
    // Text on the clipboard: egui's Paste event pastes once, its release adds nothing.
    copy_elsewhere(&board, vec![(TEXT, b"words".to_vec())]);
    frame(&mut app, vec![egui::Event::Paste("words".into())]);
    frame(&mut app, vec![key(egui::Key::V, false, cmd)]);
    assert_eq!(count(&app), 2);
}

#[test]
fn without_the_service_plain_text_is_still_not_pasted() {
    // The web: SVG text through egui only (see shortcuts' copy_and_paste_events_use_the_system_clipboard).
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    run(&mut app, "file.new", json!({"width": 200, "height": 200}));
    frame(&mut app, vec![]);
    frame(&mut app, vec![key(egui::Key::V, false, egui::Modifiers::COMMAND)]);
    frame(&mut app, vec![egui::Event::Paste("hello".into())]);
    assert_eq!(count(&app), 0);
}

#[test]
fn a_picture_the_host_read_from_a_paste_pastes_without_the_service() {
    // The web: the page's paste event carries the picture, egui's Paste event no text.
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    run(&mut app, "file.new", json!({"width": 400, "height": 300}));
    let v = app.view_mut().unwrap();
    (v.center, v.fitted) = (Point::new(250.0, 180.0), true);
    let ctx = egui::Context::default();
    app.paste_from_host(&ctx, Flavour { mime: PNG, data: blue_png() }, egui::Modifiers::COMMAND);
    let n = &pasted(&app)[0];
    let NodeKind::Image(im) = &n.kind else { panic!("not an image: {:?}", n.kind) };
    assert!(im.link.is_none() && (im.width, im.height) == (4, 2));
    assert!((n.geometric_bounds().unwrap().center() - Point::new(250.0, 180.0)).hypot() < 1e-6);
    // Cmd+Shift+V held: Paste in Place, where the picture was put (the view's centre) again.
    app.paste_from_host(&ctx, Flavour { mime: PNG, data: blue_png() }, egui::Modifiers::COMMAND | egui::Modifiers::SHIFT);
    assert_eq!(count(&app), 2);
    // A picture that isn't one pastes nothing, the stale internal clipboard neither.
    app.paste_from_host(&ctx, Flavour { mime: PNG, data: b"not a picture".to_vec() }, egui::Modifiers::COMMAND);
    assert_eq!(count(&app), 2);
    assert!(app.ui.status.starts_with("Couldn't paste"), "{}", app.ui.status);
    // A copied SVG file pastes as art.
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg"><rect width="10" height="10"/></svg>"#;
    app.paste_from_host(&ctx, Flavour { mime: SVG, data: svg.as_bytes().to_vec() }, egui::Modifiers::COMMAND);
    assert!(matches!(pasted(&app)[0].kind, NodeKind::Path { .. }));
}
