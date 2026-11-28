//! The system clipboard's formats through a headless fake service (M4.57): what Copy publishes,
//! what Paste takes from other apps (bitmaps, PDF, text), the Paste menu and keyboard paste.

use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex, MutexGuard};

use craft::platform::clipboard::{ClipboardBackend, ClipboardError, ClipboardImage, EventClipboard, Flavor, RGBA8};
use serde_json::{Value, json};
use vectorcraft_doc::{Node, NodeKind};
use vectorcraft_engine::Session;
use vectorcraft_engine::cmd::clipboard::{BITMAP, Flavour, PDF, PNG, SVG, TEXT};
use vectorcraft_geom::Point;
use vectorcraft_tools::{PointerEvent, PointerKind};

use crate::sysclip::engine_flavour;
use crate::{Services, VectorcraftApp, background};

/// The fake clipboard's contents: what the app wrote, or what "another app" copied.
#[derive(Default)]
pub(super) struct Board {
    flavours: Vec<Flavour>,
    /// The app wrote what the clipboard holds (the platform's owner marker): nothing was copied since.
    ours: bool,
    /// How many times the app wrote it.
    writes: usize,
    /// Another app has the clipboard open: the app's writes fail and change nothing, and reads
    /// find nothing.
    locked: bool,
    /// The app's writes empty the clipboard and then fail.
    partial: bool,
    /// With `partial`, the formats other than text are written before the failure.
    partial_keeps_others: bool,
    /// Another app copies this while the app's write tries to open the clipboard, and the write
    /// fails.
    race: Option<Flavour>,
    /// With `race`, the other app's copy also carries the app's formats other than text.
    race_with_our_formats: bool,
    /// The app's writes fail and change nothing; reads still work.
    refuse: bool,
    /// `has` finds text only, as the Linux clipboard's does.
    text_only_has: bool,
}

/// A [`ClipboardBackend`] over a shared [`Board`]. It is `Send`, so the board is shared through a
/// mutex: the test keeps the other end to look at what the app wrote and to copy "elsewhere".
pub(super) struct Fake(Arc<Mutex<Board>>);

impl Fake {
    fn board(&self) -> MutexGuard<'_, Board> {
        self.0.lock().unwrap()
    }

    fn find(&self, mimes: &[&str]) -> Option<Flavour> {
        let b = self.board();
        if b.locked {
            return None;
        }
        let matches = |m: &str, f: &Flavour| f.mime == m || (m == BITMAP && f.mime.starts_with("image/") && f.mime != SVG);
        mimes.iter().find_map(|m| b.flavours.iter().find(|f| matches(m, f)).cloned())
    }
}

/// `png` as the raw bitmap a backend reads a bitmap as ([`RGBA8`]): width and height as little-endian
/// `u32`s, then straight RGBA8 pixels.
fn raw_bitmap(png: &[u8]) -> Option<Vec<u8>> {
    let img = image::load_from_memory(png).ok()?.to_rgba8();
    let mut raw = [img.width().to_le_bytes(), img.height().to_le_bytes()].concat();
    raw.extend_from_slice(img.as_raw());
    Some(raw)
}

impl ClipboardBackend for Fake {
    fn write(&mut self, flavors: &[Flavor]) -> Result<usize, ClipboardError> {
        let ours: Vec<Flavour> = flavors.iter().cloned().filter_map(engine_flavour).collect();
        let mut b = self.board();
        if b.locked || b.refuse {
            return Err(ClipboardError::Other("another app has the clipboard open".into()));
        }
        if let Some(copied) = b.race.take() {
            b.flavours = vec![copied];
            if b.race_with_our_formats {
                b.flavours.extend(ours.into_iter().filter(|f| f.mime != TEXT));
            }
            b.ours = false;
            return Err(ClipboardError::Other("another app has the clipboard open".into()));
        }
        if b.partial {
            b.flavours = if b.partial_keeps_others { ours.into_iter().filter(|f| f.mime != TEXT).collect() } else { vec![] };
            return Err(ClipboardError::Other("a format could not be written".into()));
        }
        *b = Board { flavours: ours, ours: true, writes: b.writes + 1, ..std::mem::take(&mut *b) };
        Ok(0)
    }

    fn read(&mut self, mimes: &[&str], _limit: u64) -> Result<Option<Flavor>, ClipboardError> {
        Ok(self.find(mimes).map(|f| {
            // A bitmap found as "any bitmap" comes back raw, as a backend reads one.
            let any_bitmap = f.mime.starts_with("image/") && f.mime != SVG && mimes.contains(&BITMAP) && !mimes.contains(&f.mime);
            match any_bitmap.then(|| raw_bitmap(&f.data)).flatten() {
                Some(raw) => Flavor::new(RGBA8, raw),
                None => Flavor::new(f.mime, f.data),
            }
        }))
    }

    fn read_image(&mut self, _limit: u64) -> Result<Option<ClipboardImage>, ClipboardError> {
        let img = self.find(&[BITMAP]).and_then(|f| image::load_from_memory(&f.data).ok()).map(|img| img.to_rgba8());
        Ok(img.map(|img| ClipboardImage { width: img.width(), height: img.height(), rgba: img.into_raw() }))
    }

    fn read_files(&mut self, _limit: usize) -> Result<Option<Vec<std::path::PathBuf>>, ClipboardError> {
        Ok(None)
    }

    fn owns(&mut self) -> Option<bool> {
        Some(self.board().ours)
    }

    fn has(&mut self, mimes: &[&str]) -> bool {
        if self.board().text_only_has {
            return mimes.contains(&TEXT) && self.find(&[TEXT]).is_some();
        }
        self.find(mimes).is_some()
    }
}

/// A fake clipboard and the board the test looks at it through. A test that builds its own app
/// installs the fake with `Services::clipboard`.
pub(super) fn fake() -> (Fake, Arc<Mutex<Board>>) {
    let board = Arc::new(Mutex::new(Board::default()));
    (Fake(board.clone()), board)
}

/// Services with a fake system clipboard, for a test elsewhere that reads what a Copy wrote
/// through [`VectorcraftApp::written_clipboard_text`].
pub(crate) fn fake_clipboard_services() -> Services {
    Services { clipboard: Some(Box::new(fake().0)), ..Default::default() }
}

/// An app with a document and a fake system clipboard, the view centred on (250, 180).
pub(super) fn app() -> (VectorcraftApp, Arc<Mutex<Board>>) {
    let (fake, board) = fake();
    let services = Services { clipboard: Some(Box::new(fake)), ..Default::default() };
    let mut app = VectorcraftApp::new(Session::new(), services);
    run(&mut app, "file.new", json!({"width": 400, "height": 300}));
    let v = app.view_mut().unwrap();
    (v.center, v.fitted) = (Point::new(250.0, 180.0), true);
    (app, board)
}

/// Another app copies `flavours`.
pub(super) fn copy_elsewhere(board: &Arc<Mutex<Board>>, flavours: Vec<(&'static str, Vec<u8>)>) {
    let flavours = flavours.into_iter().map(|(mime, data)| Flavour { mime, data }).collect();
    let mut b = board.lock().unwrap();
    *b = Board { flavours, ours: false, ..std::mem::take(&mut *b) };
}

/// Another app opens the clipboard (`true`) or closes it.
pub(super) fn lock(board: &Arc<Mutex<Board>>, locked: bool) {
    board.lock().unwrap().locked = locked;
}

/// The fake clipboard's `has` finds text only, as the Linux clipboard's does.
pub(super) fn has_text_only(board: &Arc<Mutex<Board>>) {
    board.lock().unwrap().text_only_has = true;
}

/// The app's writes fail (`true`) and change nothing, while reads still work.
pub(super) fn refuse_writes(board: &Arc<Mutex<Board>>, refuse: bool) {
    board.lock().unwrap().refuse = refuse;
}

/// The app's writes empty the clipboard and then fail, as when one format can't be written; with
/// `keep_others`, the formats other than text are written.
pub(super) fn fail_writes_after_emptying(board: &Arc<Mutex<Board>>, keep_others: bool) {
    let mut b = board.lock().unwrap();
    b.partial = true;
    b.partial_keeps_others = keep_others;
}

/// Another app copies `data` of type `mime` while the app's next write tries to open the
/// clipboard, and that write fails. With `our_formats`, its copy also carries the app's formats
/// other than text.
pub(super) fn copy_during_next_write(board: &Arc<Mutex<Board>>, mime: &'static str, data: Vec<u8>, our_formats: bool) {
    let mut b = board.lock().unwrap();
    b.race = Some(Flavour { mime, data });
    b.race_with_our_formats = our_formats;
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

/// How many times the app wrote the clipboard.
pub(super) fn writes(board: &Arc<Mutex<Board>>) -> usize {
    board.lock().unwrap().writes
}

pub(super) fn written(board: &Arc<Mutex<Board>>) -> Vec<&'static str> {
    board.lock().unwrap().flavours.iter().map(|f| f.mime).collect()
}

/// The objects the last paste selected.
pub(super) fn pasted(app: &VectorcraftApp) -> Vec<Node> {
    let st = app.session.active().unwrap();
    st.selection.objects.iter().map(|id| st.doc.node(*id).unwrap().clone()).collect()
}

/// The objects on the first layer (its ruler guides left out).
pub(super) fn count(app: &VectorcraftApp) -> usize {
    app.session.active().unwrap().doc.layers[0].children().unwrap().iter().filter(|c| !c.is_ruler_guide()).count()
}

/// An egui `PasteImage` from the host is the signal to read the rich system clipboard: it must
/// paste the bitmap even when no paste chord or text event came with it.
#[test]
fn an_image_only_paste_event_imports_the_system_image_once() {
    let (mut app, board) = app();
    copy_elsewhere(&board, vec![(PNG, blue_png())]);
    frame(&mut app, vec![egui::Event::PasteImage(Arc::new(egui::ColorImage::filled([4, 2], egui::Color32::BLUE)))]);
    frame(&mut app, vec![]);
    assert_eq!(count(&app), 1, "one image object, not zero or two");
    assert!(matches!(pasted(&app)[0].kind, NodeKind::Image { .. }));
}

/// The image-only paste runs the paste of the chord held when the image arrived (Paste in Place
/// for Cmd+Shift), not of the next pass's modifiers, and pastes once.
#[test]
fn an_image_only_paste_keeps_the_paste_time_chord() {
    let (mut app, board) = app();
    copy_elsewhere(&board, vec![(PNG, blue_png())]);
    // The first pass builds the keymap.
    frame(&mut app, vec![]);
    let held = egui::Modifiers::COMMAND | egui::Modifiers::SHIFT;
    frame(
        &mut app,
        vec![egui::Event::ModifiersChanged(held), egui::Event::PasteImage(Arc::new(egui::ColorImage::filled([4, 2], egui::Color32::BLUE)))],
    );
    // The next pass carries the keyboard's modifiers back as None.
    frame(&mut app, vec![egui::Event::ModifiersChanged(egui::Modifiers::NONE)]);
    assert_eq!(count(&app), 1, "the image pasted once");
    let label = app.session.active().unwrap().history.undo.last().map(|step| step.label.clone());
    assert_eq!(label.as_deref(), Some("Paste in Place"), "the paste-time chord picks the paste");
}

/// A text paste alongside `PasteImage` takes the text path only: the image signal must not read
/// and paste the clipboard a second time.
#[test]
fn text_and_image_paste_events_paste_once_as_text() {
    let (mut app, board) = app();
    copy_elsewhere(&board, vec![(TEXT, b"Words".to_vec()), (PNG, blue_png())]);
    frame(
        &mut app,
        vec![egui::Event::Paste("Words".into()), egui::Event::PasteImage(Arc::new(egui::ColorImage::filled([4, 2], egui::Color32::BLUE)))],
    );
    frame(&mut app, vec![]);
    assert_eq!(count(&app), 1, "one paste, not text and picture");
    assert!(matches!(pasted(&app)[0].kind, NodeKind::Text(_)), "{:?}", pasted(&app)[0].kind);
}

/// A 4 × 2 px blue PNG.
pub(crate) fn blue_png() -> Vec<u8> {
    let img = image::RgbaImage::from_pixel(4, 2, image::Rgba([0, 0, 255, 255]));
    let mut png = vec![];
    {
        let __dyn: image::DynamicImage = img.into();
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
    png
}

/// One headless frame of the app delivering `events` (its tick, then its pass: shortcuts too, from
/// the second frame on), then the commands the frame fired, which the window runs before its next
/// frame; in the test's one context (in a new one the app starts its fonts over).
pub(super) fn frame(app: &mut VectorcraftApp, events: Vec<egui::Event>) -> egui::FullOutput {
    thread_local! {
        static CTX: egui::Context = crate::testing::ui_context();
    }
    let ctx = CTX.with(Clone::clone);
    crate::testing::app_frame(app, &ctx, egui::RawInput { events, ..Default::default() })
}

/// The paste egui sends for a clipboard with a picture and no text.
fn picture_paste() -> egui::Event {
    egui::Event::PasteImage(std::sync::Arc::new(egui::ColorImage::filled([1, 1], egui::Color32::WHITE)))
}

fn key(key: egui::Key, pressed: bool, modifiers: egui::Modifiers) -> egui::Event {
    egui::Event::Key { key, physical_key: None, pressed, repeat: false, modifiers }
}

/// [`app`] with the background worker of a host that writes from any thread, as the desktop app's.
fn app_with_worker() -> (VectorcraftApp, Arc<Mutex<Board>>) {
    let (mut app, board) = app();
    app.services.write_shared = Some(Arc::new(|_: &str, _: &[u8]| Err("no files are written here".into())));
    (app, board)
}

/// [`app_with_worker`], its worker busy with a job labeled `label` (none: another Copy's formats)
/// until the sender is dropped.
pub(super) fn app_with_busy_worker(label: &str) -> (VectorcraftApp, Arc<Mutex<Board>>, Sender<()>) {
    let (mut app, board) = app_with_worker();
    let hold = hold_worker(&mut app, label);
    (app, board, hold)
}

/// Start a job labeled `label` that holds the worker until the sender is dropped (also when the
/// test panics). No time limit: a test slowed past one by a loaded machine would see the held
/// work run early.
fn hold_worker(app: &mut VectorcraftApp, label: &str) -> Sender<()> {
    let (hold, held) = channel::<()>();
    let work = move |_: background::Writer| {
        let _ = held.recv();
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
        static CTX: egui::Context = crate::testing::ui_context();
    }
    let ctx = CTX.with(Clone::clone);
    let screen_rect = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600.0, 850.0)));
    let mut out = ctx.run_ui(egui::RawInput { screen_rect, ..Default::default() }, |ui| {
        app.tick(ui.ctx());
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
    assert!(String::from_utf8_lossy(&board.lock().unwrap().flavours[0].data).contains("<svg"));
    run(&mut app, "prefs.set", json!({"key": "copyAsPdf", "value": true}));
    run(&mut app, "edit.cut", json!({}));
    assert_eq!(written(&board), [TEXT, SVG, PDF, PNG]);
    assert!(board.lock().unwrap().flavours[2].data.starts_with(b"%PDF"));
    // Nothing goes through egui's text output when the service is there.
    assert!(app.clipboard_out.is_none());
}

#[test]
fn our_own_copy_pastes_the_lossless_internal_clipboard() {
    let (mut app, board) = app();
    copy_rect(&mut app);
    // As if the clipboard held only the PNG: still ours, so the rectangle comes back as a path.
    board.lock().unwrap().flavours.retain(|f| f.mime == PNG);
    run(&mut app, "edit.paste", json!({}));
    assert!(matches!(pasted(&app)[0].kind, NodeKind::Path { .. }));
}

/// SVG markup of three closed paths, a group of two more and a compound path with a hole: five
/// objects.
fn closed_paths_svg() -> String {
    let mut svg = String::from(r#"<svg xmlns="http://www.w3.org/2000/svg" width="400" height="300">"#);
    for x in [10, 20, 30] {
        svg.push_str(&format!(r##"<path d="M{x} 10h4v4h-4z" fill="#336699"/>"##));
    }
    svg.push_str(r#"<g><path d="M200 200h2v2h-2z"/><path d="M204 200h2v2h-2z"/></g>"#);
    svg.push_str(r#"<path fill-rule="evenodd" d="M210 200h6v6h-6zM212 202h2v2h-2z"/></svg>"#);
    svg
}

/// Paste the objects of [`closed_paths_svg`] from another app and select them all → their count.
fn paste_closed_paths(app: &mut VectorcraftApp, board: &Arc<Mutex<Board>>) -> usize {
    copy_elsewhere(board, vec![(SVG, closed_paths_svg().into_bytes())]);
    run(app, "edit.paste", json!({}));
    run(app, "select.all", json!({}));
    let objects = count(app);
    assert_eq!(app.session.active().unwrap().selection.objects.len(), objects);
    objects
}

/// A Copy or Cut whose formats can't be written to the system clipboard (another app holds it
/// open, or refuses the write) stays what every Paste command takes while nothing else is copied,
/// also when its formats have no text; the clipboard's older text does not replace it, and a paste
/// into text takes the copy's text, or nothing. A later copy in another app still wins. A later copy is
/// noticed only when it changes the clipboard's text, so when the clipboard held only a picture,
/// or could not be read (another app holds it open), Paste takes what the clipboard holds.
#[test]
fn a_copy_the_system_clipboard_refused_still_pastes_the_copied_art() {
    type Fail = fn(&Arc<Mutex<Board>>, bool);
    // Case, Copy or Cut, how the write fails, On Copy: Include SVG Code, the other app's copy
    // before it is a picture, Paste takes the copied art.
    let cases: [(&str, &str, Fail, bool, bool, bool); 5] = [
        ("refused", "edit.cut", refuse_writes, true, false, true),
        ("refused, no text", "edit.cut", refuse_writes, false, false, true),
        ("refused copy", "edit.copy", refuse_writes, true, false, true),
        ("locked", "edit.cut", lock, true, false, false),
        ("refused, over a picture", "edit.cut", refuse_writes, true, true, false),
    ];
    let pastes =
        ["edit.paste", "edit.pasteInFront", "edit.pasteInBack", "edit.pasteInPlace", "edit.pasteOnAllArtboards", "edit.pasteWithoutFormatting"];
    for (case, op, fail, svg_text, over_picture, kept) in cases {
        let (mut app, board) = app();
        run(&mut app, "prefs.set", json!({"key": "copyAsSvg", "value": svg_text}));
        let objects = paste_closed_paths(&mut app, &board);
        if over_picture {
            copy_elsewhere(&board, vec![(PNG, blue_png())]);
        } else {
            copy_elsewhere(&board, vec![(TEXT, b"Words another app copied".to_vec())]);
        }
        fail(&board, true);
        run(&mut app, op, json!({}));
        background::wait_all(&mut app);
        fail(&board, false);
        assert!(app.ui.status.starts_with("Couldn't copy to the system clipboard"), "{case}: {}", app.ui.status);
        let left = if op == "edit.cut" { 0 } else { objects };
        assert_eq!(count(&app), left, "{case}");
        if !kept {
            run(&mut app, "edit.paste", json!({}));
            match &pasted(&app)[0].kind {
                NodeKind::Image { .. } => assert!(over_picture, "{case}: the picture"),
                NodeKind::Text(t) => assert_eq!((over_picture, t.plain_text().as_str()), (false, "Words another app copied"), "{case}"),
                kind => panic!("{case}: not what the clipboard holds: {kind:?}"),
            }
            continue;
        }
        let text = app.written_clipboard_text();
        assert_eq!(text.is_some(), svg_text, "{case}: a paste into text takes the copy's text, or nothing: {text:?}");
        assert!(text.is_none_or(|t| t.contains("<svg")), "{case}: the copy's text");
        for (i, paste) in pastes.into_iter().enumerate() {
            run(&mut app, paste, json!({}));
            let p = pasted(&app);
            assert_eq!(p.len(), objects, "{case}, {paste}: the copied art, not the other app's copy");
            assert!(p.iter().all(|n| !matches!(n.kind, NodeKind::Text(_) | NodeKind::Image { .. })), "{case}, {paste}");
            assert_eq!(count(&app), left + objects * (i + 1), "{case}, {paste}");
        }
        copy_elsewhere(&board, vec![(TEXT, b"Newer words".to_vec())]);
        run(&mut app, "edit.paste", json!({}));
        let NodeKind::Text(t) = &pasted(&app)[0].kind else { panic!("{case}: not the newer copy") };
        assert_eq!(t.plain_text(), "Newer words", "{case}");
    }
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
    assert_eq!(crate::control::inspect(&app, &crate::testing::ui_context())["background"], json!([]));
    run(&mut app, "edit.paste", json!({}));
    assert!(matches!(pasted(&app)[0].kind, NodeKind::Path { .. }), "the copied rectangle, not the other app's text");
    drop(hold);
    background::wait_all(&mut app);
    assert_eq!(written(&board), [TEXT, SVG, PNG]);
    assert_eq!(board.lock().unwrap().flavours, app.session.clipboard_flavours());
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
    let b = board.lock().unwrap();
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
    assert_eq!(board.lock().unwrap().flavours, app.session.clipboard_flavours());
    assert_eq!(app.background.jobs.len(), 1, "only the save");
    drop(hold);
    background::wait_all(&mut app);
    assert_eq!(board.lock().unwrap().writes, 1);
}

/// A PNG whose header reads but whose pixel data is cut short, which the renderer can't decode.
pub(super) fn cut_short_png() -> Vec<u8> {
    let mut png = Vec::new();
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
    png
}

/// The PNG's note on an image the renderer can't decode.
pub(super) const UNDECODED: &str = "an image couldn't be decoded and is missing from the picture";

/// Copy of an image the renderer can't decode: the PNG leaves it out, and once the formats are
/// written the status bar says the Copy (or Cut) has 1 note and shows it, made at once or on the
/// worker. The note is kept for the message's hover, and a Copy without notes leaves the message.
#[test]
fn copy_says_in_the_status_bar_when_an_image_could_not_be_decoded() {
    let png = cut_short_png();
    for worker in [false, true] {
        let (mut app, board) = if worker { app_with_worker() } else { app() };
        app.status("Ready");
        copy_rect(&mut app);
        background::wait_all(&mut app);
        assert_eq!((app.ui.status.as_str(), &app.ui.status_notes), ("Ready", &None), "worker {worker}: no note");
        for (id, said) in [("edit.copy", "Copied"), ("artboard.copy", "Copied"), ("edit.cut", "Cut")] {
            vectorcraft_testkit::fixtures::place_cut_png(&mut app.session, &png, json!({}));
            app.status("Ready");
            let p = if id == "artboard.copy" { json!({"index": 0}) } else { json!({}) };
            run(&mut app, id, p);
            background::wait_all(&mut app);
            assert_eq!(written(&board), [TEXT, SVG, PNG], "worker {worker}: {id}");
            let status = format!("{said} with 1 note: {UNDECODED}");
            assert_eq!(app.ui.status, status, "worker {worker}: {id}");
            assert_eq!(app.ui.status_notes, Some((status, vec![UNDECODED.to_string()])), "worker {worker}: {id}");
        }
    }
}

/// The status bar names how many notes a Copy's or Cut's formats have and the first of them, and
/// keeps them all, in order, for the message's hover. Without notes it keeps its message.
#[test]
fn copy_notes_are_counted_and_kept_in_order() {
    let (mut app, _) = app();
    let notes: Vec<String> = ["first", "second", "third"].map(String::from).to_vec();
    for (cut, said) in [(false, "Copied"), (true, "Cut")] {
        crate::sysclip::show_copy_notes(&mut app, cut, notes.clone());
        let status = format!("{said} with 3 notes: first");
        assert_eq!(app.ui.status, status);
        assert_eq!(app.ui.status_notes, Some((status, notes.clone())));
    }
    app.status("Ready");
    app.ui.status_notes = None;
    crate::sysclip::show_copy_notes(&mut app, false, vec![]);
    assert_eq!((app.ui.status.as_str(), &app.ui.status_notes), ("Ready", &None));
}

/// A message shown while a Copy's formats are made on the worker stays: the formats are written
/// once they are ready, and their note does not replace the newer message.
#[test]
fn a_copy_note_does_not_replace_a_newer_message() {
    let (mut app, board, hold) = app_with_busy_worker("");
    vectorcraft_testkit::fixtures::place_cut_png(&mut app.session, &cut_short_png(), json!({}));
    app.status("Ready");
    run(&mut app, "edit.copy", json!({}));
    app.status("Saved /out/a.vectorcraft");
    drop(hold);
    background::wait_all(&mut app);
    assert_eq!(written(&board), [TEXT, SVG, PNG]);
    assert_eq!((app.ui.status.as_str(), &app.ui.status_notes), ("Saved /out/a.vectorcraft", &None));
}

/// A message set while a Copy's formats are made on the worker stays even when its text is the
/// Copy's: an empty status bar set again, or File › New clearing it.
#[test]
fn a_copy_note_does_not_replace_a_message_with_the_same_text() {
    for then in ["status", "file.new"] {
        let (mut app, board, hold) = app_with_busy_worker("");
        vectorcraft_testkit::fixtures::place_cut_png(&mut app.session, &cut_short_png(), json!({}));
        app.status("");
        run(&mut app, "edit.copy", json!({}));
        if then == "status" {
            app.status("");
        } else {
            run(&mut app, "file.new", json!({}));
        }
        drop(hold);
        background::wait_all(&mut app);
        assert_eq!(written(&board), [TEXT, SVG, PNG], "{then}");
        assert_eq!((app.ui.status.as_str(), &app.ui.status_notes), ("", &None), "{then}");
    }
}

/// A Copy without notes clears the message an earlier Copy's notes left, with its hover notes,
/// made at once or on the worker. It leaves any other message, one with the same text set since
/// included.
#[test]
fn a_copy_without_notes_clears_an_earlier_copys_notes() {
    let earlier = format!("Copied with 1 note: {UNDECODED}");
    for worker in [false, true] {
        for (then, kept) in [(None, ""), (Some("Ready"), "Ready"), (Some(earlier.as_str()), earlier.as_str())] {
            let (mut app, _) = if worker { app_with_worker() } else { app() };
            vectorcraft_testkit::fixtures::place_cut_png(&mut app.session, &cut_short_png(), json!({}));
            run(&mut app, "edit.copy", json!({}));
            background::wait_all(&mut app);
            assert_eq!(app.ui.status, earlier, "worker {worker}, {then:?}");
            if let Some(then) = then {
                app.status(then);
                app.ui.status_notes = None;
            }
            copy_rect(&mut app);
            background::wait_all(&mut app);
            assert_eq!((app.ui.status.as_str(), &app.ui.status_notes), (kept, &None), "worker {worker}, {then:?}");
        }
    }
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
    assert_eq!(board.lock().unwrap().writes, 0);
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
    let (ctx, id, mut field) = (crate::testing::ui_context(), egui::Id::unique("field"), String::new());
    for paste in [false, true] {
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            if paste {
                craft::ui::runtime::commands::fire(ui.ctx(), "edit.paste", craft::ui::runtime::commands::FireSource::Menu);
                for fired in &craft::ui::runtime::frame::take_output(ui.ctx()).fired {
                    crate::shortcuts::run_fired(&mut app, ui.ctx(), fired);
                }
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

/// The Edit menu's Paste into a text field after a Copy whose formats have no text (On Copy:
/// Include SVG Code off) and could not be written: the field takes nothing, not the text another
/// app left on the clipboard, and the copied art is not pasted onto the canvas.
#[test]
fn the_menus_paste_into_a_field_after_a_refused_textless_copy_takes_nothing() {
    let (mut app, board) = app();
    run(&mut app, "prefs.set", json!({"key": "copyAsSvg", "value": false}));
    let rect = run(&mut app, "shape.rectangle", json!({"x": 10, "y": 10, "width": 40, "height": 20}))["id"].clone();
    run(&mut app, "select.set", json!({"ids": [rect]}));
    copy_elsewhere(&board, vec![(TEXT, b"Old".to_vec())]);
    refuse_writes(&board, true);
    run(&mut app, "edit.copy", json!({}));
    background::wait_all(&mut app);
    refuse_writes(&board, false);
    let objects = count(&app);
    // A text field with the keyboard; in the second frame Paste is chosen from the menu.
    let (ctx, id, mut field) = (crate::testing::ui_context(), egui::Id::unique("field"), String::new());
    for paste in [false, true] {
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            if paste {
                craft::ui::runtime::commands::fire(ui.ctx(), "edit.paste", craft::ui::runtime::commands::FireSource::Menu);
                for fired in &craft::ui::runtime::frame::take_output(ui.ctx()).fired {
                    crate::shortcuts::run_fired(&mut app, ui.ctx(), fired);
                }
            }
            ui.add(egui::TextEdit::singleline(&mut field).id(id)).request_focus();
        });
        out.textures_delta.clear();
    }
    assert_eq!(field, "", "the field takes nothing");
    assert_eq!(count(&app), objects, "nothing is pasted onto the canvas");
}

/// A Copy whose formats could not be written stops being what Paste takes once different text is
/// seen on the clipboard: when another app later puts the older text back, Paste takes that text,
/// not the copied art.
#[test]
fn a_refused_copy_stays_replaced_when_the_older_text_comes_back() {
    let (mut app, board) = app();
    let rect = run(&mut app, "shape.rectangle", json!({"x": 10, "y": 10, "width": 40, "height": 20}))["id"].clone();
    run(&mut app, "select.set", json!({"ids": [rect]}));
    copy_elsewhere(&board, vec![(TEXT, b"Old".to_vec())]);
    refuse_writes(&board, true);
    run(&mut app, "edit.copy", json!({}));
    background::wait_all(&mut app);
    refuse_writes(&board, false);

    copy_elsewhere(&board, vec![(TEXT, b"New".to_vec())]);
    assert_eq!(app.written_clipboard_text().as_deref(), Some("New"));
    copy_elsewhere(&board, vec![(TEXT, b"Old".to_vec())]);
    assert_eq!(app.written_clipboard_text().as_deref(), Some("Old"), "a paste into text");
    run(&mut app, "edit.paste", json!({}));
    let NodeKind::Text(t) = &pasted(&app)[0].kind else { panic!("the copied art, not the clipboard's text") };
    assert_eq!(t.plain_text(), "Old");
}

/// A paste chord into type being edited, before the formats are written: the formats are written
/// first, and the paste pastes their text whether the host read another app's text for it or found
/// no text (a picture), also when that paste comes a frame after they were written; with no text in
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
    let read = || vec![egui::Event::Paste("Words another app copied".into())];
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
        // The window draws the frame without the edit's editor, as a host does before the next click starts one.
        frame(app, vec![]);
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
    paste_into_new_type(&mut app, 130.0, vec![vec![picture_paste()]]);
    assert_eq!(texts(&app), ["Words", "Words", "Words"], "the host found no text");
    // The paste with no text comes in the frame after the one that writes the formats.
    run(&mut app, "select.set", json!({"ids": [t]}));
    copy_elsewhere(&board, vec![(PNG, blue_png())]);
    run(&mut app, "edit.copy", json!({}));
    paste_into_new_type(&mut app, 190.0, vec![vec![], vec![picture_paste()]]);
    assert_eq!(texts(&app), ["Words", "Words", "Words", "Words"], "the paste came a frame later");
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
            let second = vectorcraft_doc::metadata::output_time();
            run(app, "edit.copy", json!({}));
            assert_eq!(app.background.jobs.len(), 1, "{what}: made on the worker");
            background::wait_all(app);
            assert_eq!(written(&board), mimes, "{what}");
            let session = app.session.clipboard_flavours();
            let dated = mimes.contains(&PDF) && vectorcraft_doc::metadata::output_time() != second;
            if board.lock().unwrap().flavours == session || !dated {
                assert_eq!(board.lock().unwrap().flavours, session, "{what}");
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
    assert_eq!(board.lock().unwrap().flavours[0].data, b"Words");
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
    let (pdf, _) = other.clipboard_pdf().unwrap().unwrap();
    copy_elsewhere(&board, vec![(PNG, blue_png()), (PDF, pdf)]);
    run(&mut app, "edit.paste", json!({}));
    let p = pasted(&app);
    assert!(!p.is_empty() && p.iter().all(|n| !matches!(n.kind, NodeKind::Image(_))), "{p:?}");
}

#[test]
fn keyboard_paste_of_a_bitmap_without_a_paste_event() {
    let (mut app, board) = app();
    copy_elsewhere(&board, vec![(PNG, blue_png())]);
    frame(&mut app, vec![]);
    // Cmd+V: egui sends no Paste event with text, only the paste of a picture.
    frame(&mut app, vec![picture_paste()]);
    assert_eq!(count(&app), 1);
    assert!(matches!(pasted(&app)[0].kind, NodeKind::Image(_)));
    // A V typed is no paste.
    frame(&mut app, vec![key(egui::Key::V, true, egui::Modifiers::NONE)]);
    assert_eq!(count(&app), 1);
    // Text on the clipboard: egui's Paste event pastes once.
    copy_elsewhere(&board, vec![(TEXT, b"words".to_vec())]);
    frame(&mut app, vec![egui::Event::Paste("words".into())]);
    assert_eq!(count(&app), 2);
}

/// An app on a host that reads its clipboard only inside a paste event (the web).
fn web_app() -> VectorcraftApp {
    let clipboard = EventClipboard::new(|| 0, |_| Ok(0));
    VectorcraftApp::new(Session::new(), Services { clipboard: Some(Box::new(clipboard)), ..Default::default() })
}

#[test]
fn on_the_web_plain_text_is_still_not_pasted() {
    // The web: SVG text through the paste event only (see shortcuts' copy_and_paste_events_use_the_system_clipboard).
    let mut app = web_app();
    run(&mut app, "file.new", json!({"width": 200, "height": 200}));
    frame(&mut app, vec![]);
    frame(&mut app, vec![key(egui::Key::V, false, egui::Modifiers::COMMAND)]);
    frame(&mut app, vec![egui::Event::Paste("hello".into())]);
    assert_eq!(count(&app), 0);
}

#[test]
fn on_the_web_a_picture_the_host_read_from_a_paste_pastes() {
    // The web: the page's paste event carries the picture, egui's Paste event no text.
    let mut app = web_app();
    run(&mut app, "file.new", json!({"width": 400, "height": 300}));
    let v = app.view_mut().unwrap();
    (v.center, v.fitted) = (Point::new(250.0, 180.0), true);
    let ctx = crate::testing::ui_context();
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
