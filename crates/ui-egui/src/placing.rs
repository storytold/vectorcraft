//! Placing files off the UI thread (desktop): a file the user places (the Place dialog's OK, a file
//! dropped on the canvas, the OK of the PDF, DXF and text import options Place asks for) is read and
//! decoded on a thread that reads one file at a time while the status bar shows it, and is placed in
//! the document it was placed in once it is ready, as it would be at once: the same object, undo
//! step, selection and errors. The files the Place dialog loads the place cursor with are read, and
//! their thumbnails made, the same way, and so is the file each click or drag of the place cursor
//! places. A Paste the user gives decodes another app's bitmap on the same thread, and runs once it
//! is decoded ([`adopt_for_paste`]). Files are read and placed in the order they were started.
//! `file.place`, `file.place.queue` and `edit.paste*` from MCP and scripts run at once, and so does
//! every place and paste on wasm (no threads), with `VECTORCRAFT_SYNC_OPEN` set, and in unit tests
//! that do not set `off_the_ui_thread` ([`available`]).

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use vectorcraft_engine::cmd::clipboard::{Clipboard, Flavour, ImageRead, read_image};
use vectorcraft_engine::cmd::place::{PlaceRead, QueueRead};
use vectorcraft_engine::file_access::ScopeHandle;

use crate::VectorcraftApp;

/// What was read: one file to place, the files to load the place cursor with, or a bitmap to paste.
enum Read {
    Place(Box<PlaceRead>),
    Queue(QueueRead),
    Paste(Box<Paste>),
}

/// A paste whose bitmap was decoded off the UI thread ([`adopt_for_paste`]).
struct Paste {
    /// The paste command and its params.
    id: String,
    params: Value,
    /// What it pastes (`None`: a bitmap the host converts couldn't be converted, or the paste
    /// waited behind another one without a bitmap of its own, and it takes the internal clipboard
    /// as it was when the paste started, as at once).
    pasted: Option<Pasted>,
    /// The view's center when the paste started, which the pasted objects are centered on.
    center: Option<[f64; 2]>,
    /// The objects selected when the paste started, which Paste in Front and Paste in Back paste
    /// next to (`None`: the paste follows an earlier one of its chain, and takes the selection
    /// that one left, [`Chain::left`]).
    selection: Option<Vec<vectorcraft_doc::NodeId>>,
    /// The number of its chain in [`Placing::chains`].
    chain: u64,
    /// Where new art went when the paste started, which is where the paste goes.
    insertion: Insertion,
    /// The latest Copy or Cut when the paste started ([`crate::sysclip::Publishing::latest`]).
    copied: u64,
    /// The internal clipboard when the paste started, which it pastes when [`Self::pasted`] is
    /// `None`.
    clipboard: Clipboard,
}

/// Where new art goes in the active document: the current layer, the isolated group or object, and
/// the drawing mode with the object Draw Inside draws into.
#[derive(Clone, PartialEq)]
struct Insertion {
    layer: Option<vectorcraft_doc::NodeId>,
    isolation: Option<vectorcraft_doc::NodeId>,
    draw_mode: vectorcraft_engine::DrawMode,
    draw_inside: Option<vectorcraft_doc::NodeId>,
}

impl Insertion {
    /// Where new art goes in `app`'s active document now.
    fn of(app: &VectorcraftApp) -> Option<Self> {
        let st = app.session.doc().ok()?;
        Some(Self { layer: st.active_layer, isolation: st.isolation, draw_mode: app.session.draw_mode, draw_inside: app.session.draw_inside })
    }

    /// Make it where new art goes in `app`'s active document → where new art went before.
    fn set(self, app: &mut VectorcraftApp) -> Option<Self> {
        let st = app.session.doc_mut().ok()?;
        let (layer, isolation) = (std::mem::replace(&mut st.active_layer, self.layer), std::mem::replace(&mut st.isolation, self.isolation));
        let draw_mode = std::mem::replace(&mut app.session.draw_mode, self.draw_mode);
        let draw_inside = std::mem::replace(&mut app.session.draw_inside, self.draw_inside);
        Some(Self { layer, isolation, draw_mode, draw_inside })
    }

    /// Put back each part of `self` (where new art went before [`Self::set`] set `started`) that
    /// differs from `started`, what the user changed meanwhile; a part the paste itself changed
    /// stays as the paste left it.
    fn restore_changed(self, started: &Self, app: &mut VectorcraftApp) {
        if let Ok(st) = app.session.doc_mut() {
            if self.layer != started.layer {
                st.active_layer = self.layer;
            }
            if self.isolation != started.isolation {
                st.isolation = self.isolation;
            }
        }
        if self.draw_mode != started.draw_mode {
            app.session.draw_mode = self.draw_mode;
        }
        if self.draw_inside != started.draw_inside {
            app.session.draw_inside = self.draw_inside;
        }
    }
}

/// Pastes decoded off the UI thread that follow each other in one document: each was given while the
/// one before waited, with the selection and the insertion context that one started with or, once
/// it ran, left.
struct Chain {
    /// The selection and the insertion context a paste given now must have to join the chain.
    context: (Vec<vectorcraft_doc::NodeId>, Insertion),
    /// The selection the chain's latest paste left, which the next one takes.
    left: Option<Vec<vectorcraft_doc::NodeId>>,
    /// How many of its pastes wait.
    waiting: usize,
}

/// What a paste whose bitmap was decoded off the UI thread pastes.
enum Pasted {
    /// The bitmap's type and the image read from it.
    Image(&'static str, Result<ImageRead, String>),
    /// Another flavour: the text that names the picture, when the picture couldn't be made.
    Flavour(Flavour),
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
    /// Whether it is a paste's bitmap ([`adopt_for_paste`]) rather than a file.
    paste: bool,
    /// The number of the paste's chain ([`Placing::chains`]).
    chain: Option<u64>,
    done: Receiver<Result<Read, String>>,
}

/// A read waiting for the thread that reads the files.
type Job = Box<dyn FnOnce() + Send>;

/// The files being read for Place, in the order they were started.
#[derive(Default)]
pub struct Placing {
    pub pending: Vec<Pending>,
    /// A deferred paste awaiting its own Swatch Conflict dialog.
    swatch_paste: Option<PendingSwatch>,
    /// Set by unit tests whose app reads the files the user places off the UI thread.
    pub(crate) off_the_ui_thread: bool,
    /// Set while [`crate::menus::invoke`] runs a Paste the user gave.
    pub(crate) for_user: bool,
    /// Set while a paste whose bitmap was converted off the UI thread runs: it takes the internal
    /// clipboard as it is.
    adopted: bool,
    /// The chains of pastes decoded off the UI thread that have pastes waiting, by number.
    chains: HashMap<u64, Chain>,
    /// For each document, the number of its latest chain, which a paste given there may join.
    latest_chain: HashMap<u64, u64>,
    /// The number the next chain takes.
    next_chain: u64,
    /// The thread that reads the files, one at a time in the order they were started: started with
    /// the first read, it ends when the app does.
    reader: Option<Sender<Job>>,
    /// Unit tests: how many reads run now, and the most that ran at once.
    #[cfg(test)]
    pub(crate) reading: std::sync::Arc<[std::sync::atomic::AtomicUsize; 2]>,
}

struct PendingSwatch {
    clipboard: Clipboard,
    insertion: Insertion,
    selection: Vec<vectorcraft_doc::NodeId>,
    doc: u64,
    chain: u64,
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

/// Before paste command `id` with `p`: what another app put on the system clipboard replaces the
/// internal clipboard ([`VectorcraftApp::adopt_system_clipboard`]) → `None`, and the paste runs at
/// once. For a Paste the user gives ([`crate::menus::invoke`]) where placed files are read off the
/// UI thread ([`available`]), the bitmap's bytes are copied off the system clipboard at once and
/// decoded on the thread that reads placed files, after the files started before it →
/// `{background: true, status}`: a bitmap the host converts to PNG is converted there
/// ([`crate::SystemClipboard::read_deferred`]), and the bitmap is read as `clipboard.importImage`
/// reads it ([`read_image`]). The paste runs once it is decoded, in the document it was given in,
/// as it would at once ([`poll`]). A Paste the user gives while such a paste waits waits behind it,
/// also when it has no bitmap to read. When no thread could start, the bitmap is read at once.
pub fn adopt_for_paste(app: &mut VectorcraftApp, id: &str, p: &Value) -> Result<Option<Value>, String> {
    if std::mem::take(&mut app.placing.adopted) {
        app.clipboard_image_request = None;
        return Ok(None);
    }
    if !(app.placing.for_user && available(app)) {
        return app.adopt_system_clipboard().map(|()| None);
    }
    let center = app.view().map(|v| [v.center.x, v.center.y]);
    let make = app.adopt_system_clipboard_or_defer(true)?;
    // A paste that takes the internal clipboard as it is (what another app put there was loaded
    // at once, or nothing was) runs at once, unless a paste whose bitmap is read waits: then it
    // waits behind that one, with the internal clipboard as it is now.
    if make.is_none() && !app.placing.pending.iter().any(|p| p.paste) {
        return Ok(None);
    }
    let copied = app.clipboard_publishing.latest();
    let (doc, objects) = app.session.doc().map(|st| (st.uid, st.selection.objects.clone())).map_err(|e| e.to_string())?;
    let insertion = Insertion::of(app).ok_or("no document")?;
    // A paste given while one of the document's latest chain waits, with the selection and the
    // insertion context that chain expects, joins it: it goes next to what the paste before it
    // leaves selected, as at once. Otherwise it starts a chain.
    let context = (objects, insertion.clone());
    let joins =
        app.placing.latest_chain.get(&doc).copied().filter(|c| app.placing.chains.get(c).is_some_and(|ch| ch.waiting > 0 && ch.context == context));
    let (chain, selection) = match joins {
        Some(chain) => (chain, None),
        None => (app.placing.next_chain, Some(context.0.clone())),
    };
    let clipboard = app.session.clipboard.clone();
    let (id, params) = (id.to_string(), p.clone());
    // The work stays here when no thread could start.
    let slot = Arc::new(Mutex::new(make));
    let work = slot.clone();
    let started = spawn_labeled(app, "clipboard".into(), "Pasting".into(), false, Some(chain), move || {
        let make = work.lock().ok().and_then(|mut m| m.take());
        let pasted = make.and_then(|m| m()).map(|f| {
            if crate::sysclip::imports_as_image(f.mime) {
                Pasted::Image(f.mime, read_image(&f.data).map_err(|e| e.to_string()))
            } else {
                Pasted::Flavour(f)
            }
        });
        Ok(Read::Paste(Box::new(Paste { id, params, pasted, center, selection, chain, insertion, copied, clipboard })))
    })?;
    if started.is_some() {
        if joins.is_none() {
            app.placing.next_chain += 1;
            app.placing.chains.insert(chain, Chain { context, left: None, waiting: 0 });
            app.placing.latest_chain.insert(doc, chain);
        }
        if let Some(ch) = app.placing.chains.get_mut(&chain) {
            ch.waiting += 1;
        }
        return Ok(started);
    }
    match slot.lock().ok().and_then(|mut m| m.take()).and_then(|m| m()) {
        Some(f) => app.adopt_flavour(&f, center).map(|()| None),
        None => Ok(None),
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
    spawn_labeled(app, name, format!("Placing {shown}"), dropped, None, read)
}

/// [`spawn`] with the status bar's `label`; `chain` for a paste's bitmap, its chain's number.
fn spawn_labeled(
    app: &mut VectorcraftApp,
    name: String,
    label: String,
    dropped: bool,
    chain: Option<u64>,
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
    app.placing.pending.push(Pending { name, label: label.clone(), doc, dropped, paste: chain.is_some(), chain, done });
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
    settle_swatch_paste(app);
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
        if let Some(chain) = p.chain
            && !app.placing.swatch_paste.as_ref().is_some_and(|waiting| waiting.chain == chain)
        {
            finish_chain(app, chain);
        }
    }
}

/// Place files read off the UI thread in their document, which becomes the active one again, as
/// [`crate::place::run`] and [`crate::place::queue`] place files read at once; an error shows in
/// the status bar as it does then.
fn finish(app: &mut VectorcraftApp, p: &Pending, r: Result<Read, String>) {
    let Some(i) = app.session.documents().iter().position(|st| st.uid == p.doc) else {
        let name = &p.name;
        if p.paste {
            app.status("Couldn't paste: the document it was pasted in was closed");
        } else if p.dropped {
            app.status(format!("Couldn't place {name}: the document it was dropped on was closed"));
        } else {
            app.status(format!("Couldn't place {name}: the document it was placed in was closed"));
        }
        return;
    };
    if p.paste {
        let error = match &r {
            Ok(Read::Paste(paste)) => match &paste.pasted {
                Some(Pasted::Image(_, Err(e))) => Some(e.as_str()),
                _ => None,
            },
            Err(e) => Some(e.as_str()),
            _ => None,
        };
        if let Some(e) = error {
            if let Some(ch) = p.chain.and_then(|chain| app.placing.chains.get_mut(&chain))
                && ch.left.is_none()
            {
                let selection = match &r {
                    Ok(Read::Paste(paste)) => paste.selection.clone(),
                    _ => None,
                };
                ch.left = Some(selection.unwrap_or_else(|| ch.context.0.clone()));
            }
            app.status(format!("Couldn't paste from the clipboard: {e}"));
            return;
        }
    }
    app.session.set_active(i);
    let r = match r {
        Ok(Read::Place(read)) => app.session.place_read(*read).map(|_| ()).map_err(|e| e.to_string()),
        Ok(Read::Queue(read)) => app.session.place_queue_read(read).map(|r| crate::place::show_thumbnails(app, &r)).map_err(|e| e.to_string()),
        Ok(Read::Paste(paste)) => return finish_paste(app, *paste),
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

/// Run a paste whose bitmap was decoded off the UI thread, in its document, which is the active one:
/// the image replaces the internal clipboard, the objects selected when the paste started are
/// selected again, and the paste command runs as it runs at once ([`VectorcraftApp::run`]), into
/// the layer that was current when it started, with its errors in the status bar. A Copy or Cut
/// made since the paste started keeps the internal clipboard it made.
fn finish_paste(app: &mut VectorcraftApp, paste: Paste) {
    let kept = (app.clipboard_publishing.latest() != paste.copied).then(|| app.session.clipboard.clone());
    let adopted = match paste.pasted {
        Some(Pasted::Image(mime, read)) => app.adopt_image_read(mime, paste.center, read),
        Some(Pasted::Flavour(f)) => app.adopt_flavour(&f, paste.center),
        None => {
            app.session.clipboard = paste.clipboard;
            Ok(())
        }
    };
    let chain = paste.chain;
    let selection = paste.selection.or_else(|| app.placing.chains.get_mut(&chain).and_then(|ch| ch.left.take()));
    let mut deferred = false;
    let left = match adopted {
        Err(e) => {
            app.status(e);
            selection
        }
        Ok(()) => {
            // Where new art goes now, before a selection changes the current layer.
            let current = Insertion::of(app);
            // The objects selected when the paste started (or that the paste it follows left) are
            // selected again (those that are left), as `Session::place_read` selects the object
            // Replace swaps.
            let now = app.session.doc().map(|st| st.selection.objects.clone()).unwrap_or_default();
            if let Some(selection) = selection.filter(|s| *s != now) {
                let left: Vec<u64> =
                    selection.iter().filter(|id| app.session.doc().is_ok_and(|st| st.doc.node(**id).is_some())).map(|id| id.0).collect();
                let _ = app.session.execute("select.set", &json!({ "ids": left }));
            }
            // The paste goes where new art went when it started (the current layer, the isolated
            // group, the object Draw Inside draws into), and what the user changed meanwhile is
            // kept.
            paste.insertion.clone().set(app);
            let requested_selection = app.session.doc().map(|st| st.selection.objects.clone()).unwrap_or_default();
            app.placing.adopted = true;
            // `run` shows an error in the status bar.
            let _ = app.run(&paste.id, paste.params);
            // Also when the paste did not reach the clipboard (the Type tool took it).
            app.placing.adopted = false;
            if let Some(d) = app.ui.dialog.as_mut().filter(|d| d.kind == crate::dialogs::swatch_conflict::KIND) {
                d.fields.insert("__deferredPaste".into(), json!(chain));
                if let Ok(st) = app.session.doc() {
                    app.placing.swatch_paste = Some(PendingSwatch {
                        clipboard: app.session.clipboard.clone(),
                        insertion: paste.insertion.clone(),
                        selection: requested_selection,
                        doc: st.uid,
                        chain,
                    });
                    deferred = true;
                }
            }
            if let Some(current) = current {
                current.restore_changed(&paste.insertion, app);
            }
            app.session.doc().ok().map(|st| st.selection.objects.clone())
        }
    };
    // The chain's next paste takes the selection this one left, and a paste given with it and the
    // insertion context joins the chain while a paste of it waits ([`poll`] drops the chain).
    let context = app.session.doc().ok().map(|st| st.selection.objects.clone()).zip(Insertion::of(app));
    if !deferred && let Some(ch) = app.placing.chains.get_mut(&chain) {
        ch.left = left;
        if let Some(context) = context {
            ch.context = context;
        }
    }
    if let Some(kept) = kept {
        app.session.clipboard = kept;
    }
}

fn finish_chain(app: &mut VectorcraftApp, chain: u64) {
    if let Some(ch) = app.placing.chains.get_mut(&chain) {
        ch.waiting = ch.waiting.saturating_sub(1);
        if ch.waiting == 0 {
            app.placing.chains.remove(&chain);
            app.placing.latest_chain.retain(|_, c| *c != chain);
        }
    }
}

/// Release a deferred paste when its Swatch Conflict dialog is canceled or replaced.
pub(crate) fn cancel_swatch_paste(app: &mut VectorcraftApp) {
    if let Some(waiting) = app.placing.swatch_paste.take() {
        if let Some(ch) = app.placing.chains.get_mut(&waiting.chain) {
            ch.left = Some(waiting.selection);
        }
        finish_chain(app, waiting.chain);
    }
}

/// Release a deferred paste once its dialog is no longer the active dialog.
pub(crate) fn settle_swatch_paste(app: &mut VectorcraftApp) {
    let active = app.placing.swatch_paste.as_ref().is_some_and(|waiting| {
        app.ui.dialog.as_ref().is_some_and(|d| {
            d.kind == crate::dialogs::swatch_conflict::KIND && d.fields.get("__deferredPaste").and_then(Value::as_u64) == Some(waiting.chain)
        })
    });
    if app.placing.swatch_paste.is_some() && !active {
        cancel_swatch_paste(app);
    }
}

/// Run the command confirmed by the deferred paste's Swatch Conflict dialog.
pub(crate) fn confirm_swatch_paste(app: &mut VectorcraftApp, id: &str, params: Value, token: Option<u64>) -> Result<Value, String> {
    if !app.placing.swatch_paste.as_ref().is_some_and(|waiting| Some(waiting.chain) == token) {
        if token.is_some() || app.placing.swatch_paste.is_some() {
            cancel_swatch_paste(app);
            return Err("the deferred paste is no longer available".into());
        }
        return app.run(id, params);
    }
    let Some(waiting) = app.placing.swatch_paste.take() else { return app.run(id, params) };
    let Some(index) = app.session.documents().iter().position(|st| st.uid == waiting.doc) else {
        finish_chain(app, waiting.chain);
        return Err("Couldn't paste: the document it was pasted in was closed".into());
    };
    app.session.set_active(index);
    app.sync_views();
    let current = std::mem::replace(&mut app.session.clipboard, waiting.clipboard);
    let changed = Insertion::of(app);
    let now = app.session.doc().map(|st| st.selection.objects.clone()).unwrap_or_default();
    let selected: Vec<u64> =
        waiting.selection.iter().filter(|id| app.session.doc().is_ok_and(|st| st.doc.node(**id).is_some())).map(|id| id.0).collect();
    if selected != now.iter().map(|id| id.0).collect::<Vec<_>>() {
        let _ = app.session.execute("select.set", &json!({"ids": selected}));
    }
    let insertion = waiting.insertion.clone();
    insertion.clone().set(app);
    app.placing.adopted = true;
    let result = app.run(id, params);
    app.placing.adopted = false;
    if let Some(changed) = changed {
        changed.restore_changed(&insertion, app);
    }
    app.session.clipboard = current;
    let left = if result.is_ok() { app.session.doc().ok().map(|st| st.selection.objects.clone()) } else { Some(waiting.selection) };
    let context = app.session.doc().ok().map(|st| st.selection.objects.clone()).zip(Insertion::of(app));
    if let Some(ch) = app.placing.chains.get_mut(&waiting.chain) {
        ch.left = left;
        if let Some(context) = context {
            ch.context = context;
        }
    }
    finish_chain(app, waiting.chain);
    result
}

#[cfg(test)]
mod tests;
