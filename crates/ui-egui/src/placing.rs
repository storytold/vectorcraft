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
mod tests;
