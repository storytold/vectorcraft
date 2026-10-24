//! Placing files off the UI thread (desktop): a file the user places (the Place dialog's OK, a file
//! dropped on the canvas, the OK of the PDF, DXF and text import options Place asks for) is read and
//! decoded on a thread that reads one file at a time while the status bar shows it, and is placed in
//! the document it was placed in once it is ready, as it would be at once: the same object, undo
//! step, selection and errors. The files the Place dialog loads the place cursor with are read, and
//! their thumbnails made, the same way, and so is the file each click or drag of the place cursor
//! places. Files are read and placed in the order they were started. `file.place` and
//! `file.place.queue` from MCP and scripts run at once, and so does every place on wasm (no
//! threads), with `VECTORCRAFT_SYNC_OPEN` set, and in unit tests that do not set
//! `off_the_ui_thread` ([`available`]).

use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};

use serde_json::{Value, json};
use vectorcraft_engine::cmd::place::{PlaceRead, QueueRead};
use vectorcraft_engine::file_access::ScopeHandle;

use crate::VectorcraftApp;

/// What was read: one file to place, or the files to load the place cursor with.
enum Read {
    Place(Box<PlaceRead>),
    Queue(QueueRead),
}

/// Files being read for Place.
pub struct Pending {
    /// The file's name ("photo.tif"; the names joined with ", " for the place cursor's files).
    pub(crate) name: String,
    /// What the status bar shows ("Placing photo.tif").
    pub label: String,
    /// The document they are placed in ([`vectorcraft_engine::DocState::uid`]).
    doc: u64,
    /// Whether the file was dropped on the canvas (its messages say so, as
    /// [`crate::place::drop_files`]'s do).
    dropped: bool,
    done: Receiver<Result<Read, String>>,
}

/// A read waiting for the thread that reads the files.
type Job = Box<dyn FnOnce() + Send>;

/// The files being read for Place, in the order they were started.
#[derive(Default)]
pub struct Placing {
    pub pending: Vec<Pending>,
    /// Set by unit tests whose app reads the files the user places off the UI thread.
    pub(crate) off_the_ui_thread: bool,
    /// The thread that reads the files, one at a time in the order they were started: started with
    /// the first read, it ends when the app does.
    reader: Option<Sender<Job>>,
    /// Unit tests: how many reads run now, and the most that ran at once.
    #[cfg(test)]
    pub(crate) reading: std::sync::Arc<[std::sync::atomic::AtomicUsize; 2]>,
}

/// Does `app` read a file the user places off the UI thread? Where documents open off it
/// ([`crate::opening::available`]), and in the unit tests that set `off_the_ui_thread`.
pub fn available(app: &VectorcraftApp) -> bool {
    app.placing.off_the_ui_thread || crate::opening::available()
}

/// `file.place` with `p` for the active document, its file read and decoded off the UI thread
/// ([`vectorcraft_engine::Session::prepare_place`], [`vectorcraft_engine::cmd::place::PlaceJob::read`])
/// → `{background: true, status}`; `dropped` when it was dropped on the canvas. The params are
/// checked here first, so a bad one is an error at once. When no thread could start, the file is
/// placed at once → `file.place`'s reply.
pub fn start(app: &mut VectorcraftApp, p: &Value, dropped: bool) -> Result<Value, String> {
    let job = app.session.prepare_place(p).map_err(|e| e.to_string())?;
    let name = job.name();
    let started = spawn(app, name.clone(), name, dropped, move || job.read().map(|r| Read::Place(Box::new(r))).map_err(|e| e.to_string()))?;
    started.map_or_else(
        || {
            let r = app.session.execute("file.place", p).map_err(|e| e.to_string());
            app.sync_views();
            r
        },
        Ok,
    )
}

/// `file.place.queue` with `p` for the active document, its files read and their thumbnails made off
/// the UI thread → `{background: true, status}`; the place cursor is loaded once they are ready. When no thread could start, the cursor is loaded at once → `file.place.queue`'s reply.
pub fn start_queue(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let job = app.session.prepare_place_queue(p).map_err(|e| e.to_string())?;
    let names = job.names().join(", ");
    let started = spawn(app, names.clone(), names, false, move || job.read().map(Read::Queue).map_err(|e| e.to_string()))?;
    match started {
        Some(r) => Ok(r),
        None => {
            let r = app.session.execute("file.place.queue", p).map_err(|e| e.to_string());
            app.sync_views();
            let r = r?;
            crate::place::show_thumbnails(app, &r);
            Ok(r)
        }
    }
}

/// Run `read` for the active document on the thread that reads the files, after the reads started
/// before it (so a batch of large pictures is decoded one at a time, as at once) →
/// `{background: true, status}`, or `None` when that thread could not start.
fn spawn(
    app: &mut VectorcraftApp,
    name: String,
    shown: String,
    dropped: bool,
    read: impl FnOnce() -> Result<Read, String> + Send + 'static,
) -> Result<Option<Value>, String> {
    let doc = app.session.doc().map_err(|e| e.to_string())?.uid;
    let (tx, done) = channel();
    // The automation roots in force here are in force there.
    let scope = ScopeHandle::capture();
    #[cfg(test)]
    let reading = app.placing.reading.clone();
    let job: Job = Box::new(move || {
        #[cfg(test)]
        {
            let now = reading[0].fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
            reading[1].fetch_max(now, std::sync::atomic::Ordering::SeqCst);
        }
        let r = scope
            .run(|| vectorcraft_engine::guard::catch_panic(read))
            .unwrap_or_else(|msg| Err(format!("internal error: {msg} (please report this bug)")));
        #[cfg(test)]
        reading[0].fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
        // The app may have quit meanwhile: nothing to hand it to.
        let _ = tx.send(r);
    });
    // A reader that has ended gives the job back.
    let job = match &app.placing.reader {
        Some(reader) => reader.send(job).err().map(|e| e.0),
        None => Some(job),
    };
    if let Some(job) = job {
        let (reader, jobs) = channel::<Job>();
        let started = std::thread::Builder::new().name("vectorcraft-place".into()).spawn(move || {
            for job in jobs {
                job();
            }
        });
        if started.is_err() {
            return Ok(None);
        }
        let _ = reader.send(job);
        app.placing.reader = Some(reader);
    }
    let label = format!("Placing {shown}");
    app.placing.pending.push(Pending { name, label: label.clone(), doc, dropped, done });
    let status = format!("{label}…");
    app.status(status.clone());
    Ok(Some(json!({ "background": true, "status": status })))
}

/// Place the files that are ready (each frame), in the order they were started: a file read
/// sooner than one started before it waits for that one. Not while a dialog is open, an
/// interaction is open, or a tool is busy with a drag of its own (the place cursor's) while the
/// pointer button is held, as [`crate::opening::poll`]: a placed file's document becomes the
/// active one and the dialog's OK acts on the active document; a drag in another document is
/// canceled when the active document changes, and a drag or preview in the same one previews from
/// the document as it was before it began, without the placed art. A tool that is busy while the
/// button is not held is waiting between its clicks (the Dimension tool after its first point):
/// the file is placed unless the tool has a preview open (the Flare tool after its center drag).
pub fn poll(app: &mut VectorcraftApp) {
    while app.ui.dialog.is_none()
        && !app.session.in_interaction()
        && !(app.session.tool_busy() && app.session.pointer_held())
        && let Some(p) = app.placing.pending.first()
    {
        let r = match p.done.try_recv() {
            Ok(r) => r,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err("the file stopped being read (please report this bug)".to_string()),
        };
        let p = app.placing.pending.remove(0);
        // The status bar no longer says it is placing.
        if app.ui.status == format!("{}…", p.label) {
            app.ui.status.clear();
        }
        finish(app, &p, r);
    }
}

/// Place files read off the UI thread in their document, which becomes the active one again, as
/// [`crate::place::run`] and [`crate::place::queue`] place files read at once; an error shows in
/// the status bar as it does then.
fn finish(app: &mut VectorcraftApp, p: &Pending, r: Result<Read, String>) {
    let Some(i) = app.session.documents().iter().position(|st| st.uid == p.doc) else {
        let name = &p.name;
        if p.dropped {
            app.status(format!("Couldn't place {name}: the document it was dropped on was closed"));
        } else {
            app.status(format!("Couldn't place {name}: the document it was placed in was closed"));
        }
        return;
    };
    app.session.set_active(i);
    let r = match r {
        Ok(Read::Place(read)) => app.session.place_read(*read).map(|_| ()).map_err(|e| e.to_string()),
        Ok(Read::Queue(read)) => app.session.place_queue_read(read).map(|r| crate::place::show_thumbnails(app, &r)).map_err(|e| e.to_string()),
        Err(e) => Err(e),
    };
    app.sync_views();
    let name = &p.name;
    match r {
        Err(e) if p.dropped => app.status(format!("Couldn't place {name}: {e}")),
        Err(e) => app.status(e),
        Ok(()) => {}
    }
}

#[cfg(test)]
mod tests {
    use image::ImageFormat::{Bmp, Png, Tiff};
    use vectorcraft_doc::{Document, NodeId};
    use vectorcraft_engine::Session;

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

    /// A file that can't be read shows the message it shows when read at once, dropped or placed
    /// from the dialog, and nothing is placed.
    #[test]
    fn a_file_that_cant_be_read_off_the_ui_thread_reports_as_at_once() {
        let gone = saved("unread", "gone.tif", b"");
        std::fs::remove_file(&gone).unwrap();
        let unknown = saved("unread", "notes.xyz", b"hello");
        for path in [gone, unknown] {
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
}
