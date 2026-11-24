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
use std::sync::mpsc::{Receiver, TryRecvError, channel};

use craft::ui::dialog::{DialogId, dialog_id};
use serde_json::{Value, json};
use vectorcraft_engine::cancel::{self, CancelToken};
use vectorcraft_engine::cmd::clipboard::{Clipboard, Flavour, ImageRead, read_image};
use vectorcraft_engine::cmd::place::{PlaceRead, QueueRead};
use vectorcraft_engine::file_access::ScopeHandle;

use crate::VectorcraftApp;
use crate::background::Lane;
use crate::export_progress::Shown;

/// What was read: one file to place (and where new art went when it was placed), the files to load
/// the place cursor with (and whether they ask for their import options when placed), or a bitmap
/// to paste.
enum Read {
    Place(Box<PlaceRead>, Insertion),
    Queue(QueueRead, bool),
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
/// the drawing mode with the object Draw Inside draws into. A paste and a placed file read off the
/// UI thread keep the one from when they were given.
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

    /// Whether new art can still go where `self` says in `app`'s active document: the isolated group
    /// or object when it names one, else the object Draw Inside draws into when it names one and
    /// `draws_inside` (a paste draws inside it, a placed file does not), else its layer, which must
    /// still be a layer, is still there, shown and unlocked. A paste or a placed file whose layer or
    /// group was deleted, hidden or locked meanwhile goes where new art goes now.
    fn usable(&self, app: &VectorcraftApp, draws_inside: bool) -> bool {
        app.session.doc().is_ok_and(|st| {
            let inside = self.draw_inside.filter(|_| draws_inside && self.draw_mode == vectorcraft_engine::DrawMode::Inside);
            match (self.isolation, inside, self.layer) {
                (Some(target), ..) | (None, Some(target), _) => st.doc.is_editable(target),
                (None, None, Some(l)) => st.doc.node(l).is_some_and(|n| n.is_layer()) && st.doc.is_editable(l),
                (None, None, None) => true,
            }
        })
    }

    /// `self` when new art can still go there ([`Self::usable`]), else `now`: where new art went
    /// before the paste or the place selected anything again.
    fn or_now(self, now: Option<&Self>, app: &VectorcraftApp, draws_inside: bool) -> Self {
        match now {
            Some(now) if !self.usable(app, draws_inside) => now.clone(),
            _ => self,
        }
    }

    /// Put back each part of `self` (where new art went before [`Self::set`] set `started`) that
    /// differs from `started`, what the user changed meanwhile; a part the paste or the place
    /// itself changed stays as it left it.
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
    /// The document's [`vectorcraft_engine::DocState::reverts`] when the chain started: a paste
    /// given after a File → Revert does not join it.
    reverts: u64,
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
    /// Its [`vectorcraft_engine::DocState::reverts`] when the read started: a read for content that
    /// File → Revert has since thrown away is dropped.
    reverts: u64,
    /// Whether the file was dropped on the canvas (its messages say so, as
    /// [`crate::place::drop_files`]'s do).
    dropped: bool,
    /// Whether it is a paste's bitmap ([`adopt_for_paste`]) rather than a file.
    pub(crate) paste: bool,
    /// The number of the paste's chain ([`Placing::chains`]).
    chain: Option<u64>,
    done: Receiver<Result<Read, String>>,
    /// The automation roots in force when the read started, kept in force when it is placed: a
    /// DXF placed against an artboard that changed meanwhile is read again then.
    scope: ScopeHandle,
    token: CancelToken,
    pub hidden: bool,
    /// [`crate::export_progress::now`] when the read started.
    pub(crate) started: f64,
}

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
    /// Reads the files one at a time in the order they were started.
    lane: Lane,
    /// Unit tests: how many reads run now, and the most that ran at once.
    #[cfg(test)]
    pub(crate) reading: std::sync::Arc<[std::sync::atomic::AtomicUsize; 2]>,
}

struct PendingSwatch {
    clipboard: Clipboard,
    insertion: Insertion,
    selection: Vec<vectorcraft_doc::NodeId>,
    doc: u64,
    /// The document's [`vectorcraft_engine::DocState::reverts`] when the dialog opened.
    reverts: u64,
    chain: u64,
    /// The question the paste waits on, on the dialog host.
    asked: DialogId,
}

/// Does `app` read a file the user places off the UI thread? Where documents open off it
/// ([`crate::opening::available`]), and in the unit tests that set `off_the_ui_thread`.
pub fn available(app: &VectorcraftApp) -> bool {
    app.placing.off_the_ui_thread || crate::opening::available()
}

/// `file.place` with `p` for the active document, its file read and decoded off the UI thread
/// ([`vectorcraft_engine::Session::prepare_place`], [`vectorcraft_engine::cmd::place::PlaceJob::read`])
/// → `{background: true, status}`; `dropped` when it was dropped on the canvas. The params are
/// checked here first, so a bad one is an error at once.
pub fn start(app: &mut VectorcraftApp, p: &Value, dropped: bool) -> Result<Value, String> {
    // The edit the tool computes on another thread comes first: the object Replace swaps is the
    // one selected after it.
    let r = app.session.finish_tool_edit();
    crate::canvas::apply_requests(app, r);
    let job = app.session.prepare_place(p).map_err(|e| e.to_string())?;
    let name = job.name();
    let insertion = Insertion::of(app).ok_or("no document")?;
    spawn(app, name.clone(), name, dropped, move || job.read().map(|r| Read::Place(Box::new(r), insertion)).map_err(|e| e.to_string()))
}

/// `file.place.queue` with `p` for the active document, its files read and their thumbnails made off
/// the UI thread → `{background: true, status}`; the place cursor is loaded once they are ready.
/// `asks`: the cursor's files ask for their import options when placed ([`crate::place::show_thumbnails`]).
pub fn start_queue(app: &mut VectorcraftApp, p: &Value, asks: bool) -> Result<Value, String> {
    let job = app.session.prepare_place_queue(p).map_err(|e| e.to_string())?;
    let names = job.names().join(", ");
    spawn(app, names.clone(), names, false, move || job.read().map(|r| Read::Queue(r, asks)).map_err(|e| e.to_string()))
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
/// also when it has no bitmap to read.
pub fn adopt_for_paste(app: &mut VectorcraftApp, id: &str, p: &Value) -> Result<Option<Value>, String> {
    if std::mem::take(&mut app.placing.adopted) {
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
    let (doc, reverts, objects) = app.session.doc().map(|st| (st.uid, st.reverts, st.selection.objects.clone())).map_err(|e| e.to_string())?;
    let insertion = Insertion::of(app).ok_or("no document")?;
    // A paste given while one of the document's latest chain waits, with the selection and the
    // insertion context that chain expects, joins it: it goes next to what the paste before it
    // leaves selected, as at once. Otherwise (also when the document was reverted since the chain
    // started) it starts a chain.
    let context = (objects, insertion.clone());
    let joins = app
        .placing
        .latest_chain
        .get(&doc)
        .copied()
        .filter(|c| app.placing.chains.get(c).is_some_and(|ch| ch.waiting > 0 && ch.reverts == reverts && ch.context == context));
    let (chain, selection) = match joins {
        Some(chain) => (chain, None),
        None => (app.placing.next_chain, Some(context.0.clone())),
    };
    let clipboard = app.session.clipboard.clone();
    let (id, params) = (id.to_string(), p.clone());
    let started = spawn_labeled(app, "clipboard".into(), "Pasting".into(), false, Some(chain), move || {
        let pasted = make.and_then(|m| m()).map(|f| {
            if crate::sysclip::imports_as_image(f.mime) {
                Pasted::Image(f.mime, read_image(&f.data).map_err(|e| e.to_string()))
            } else {
                Pasted::Flavour(f)
            }
        });
        Ok(Read::Paste(Box::new(Paste { id, params, pasted, center, selection, chain, insertion, copied, clipboard })))
    })?;
    if joins.is_none() {
        app.placing.next_chain += 1;
        app.placing.chains.insert(chain, Chain { context, left: None, waiting: 0, reverts });
        app.placing.latest_chain.insert(doc, chain);
    }
    if let Some(ch) = app.placing.chains.get_mut(&chain) {
        ch.waiting += 1;
    }
    Ok(Some(started))
}

/// Run `read` for the active document on the place lane, after the reads started before it (so a
/// batch of large pictures is decoded one at a time, as at once) → `{background: true, status}`.
fn spawn(
    app: &mut VectorcraftApp,
    name: String,
    shown: String,
    dropped: bool,
    read: impl FnOnce() -> Result<Read, String> + Send + 'static,
) -> Result<Value, String> {
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
) -> Result<Value, String> {
    let (doc, reverts) = app.session.doc().map(|st| (st.uid, st.reverts)).map_err(|e| e.to_string())?;
    let (tx, done) = channel();
    // The automation roots in force here are in force there.
    let scope = ScopeHandle::capture();
    let completion = scope.clone();
    #[cfg(test)]
    let reading = app.placing.reading.clone();
    let token = CancelToken::new();
    let raised = token.clone();
    let job: Box<dyn FnOnce() + Send> = Box::new(move || {
        #[cfg(test)]
        {
            let now = reading[0].fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
            reading[1].fetch_max(now, std::sync::atomic::Ordering::SeqCst);
        }
        let r = scope
            .run(|| vectorcraft_engine::guard::catch_panic(|| cancel::scope(&raised, None, read)))
            .unwrap_or_else(|msg| Err(format!("internal error: {msg} (please report this bug)")));
        #[cfg(test)]
        reading[0].fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
        // The app may have quit meanwhile: nothing to hand it to.
        let _ = tx.send(r);
    });
    let jobs = app.jobs();
    app.placing.lane.push(&*jobs, "vectorcraft-place", job);
    app.placing.pending.push(Pending {
        name,
        label: label.clone(),
        doc,
        reverts,
        dropped,
        paste: chain.is_some(),
        chain,
        done,
        scope: completion,
        token,
        hidden: false,
        started: crate::export_progress::now(),
    });
    let status = format!("{label}…");
    app.status(status.clone());
    Ok(json!({ "background": true, "status": status }))
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
    while !app.any_dialog_open()
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
        p.scope.clone().run(|| finish(app, &p, r));
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
    // File → Revert threw away the content the read was started for: what was read is dropped,
    // with the document, its selection, the clipboard and the tools as they are.
    if app.session.documents().get(i).is_some_and(|st| st.reverts != p.reverts) {
        let name = &p.name;
        if p.paste {
            app.status("Couldn't paste: the document it was pasted in was reverted");
        } else if p.dropped {
            app.status(format!("Couldn't place {name}: the document it was dropped on was reverted"));
        } else {
            app.status(format!("Couldn't place {name}: the document it was placed in was reverted"));
        }
        return;
    }
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
        Ok(Read::Place(read, insertion)) => {
            // The file goes where new art went when it was placed (the current layer, the isolated
            // group), as a paste does, and what the user changed meanwhile is kept. Where new art
            // goes now is taken before Replace selects the object it swaps again.
            let current = Insertion::of(app);
            let insertion = insertion.or_now(current.as_ref(), app, false);
            insertion.clone().set(app);
            let r = app.session.place_read(*read).map(|_| ()).map_err(|e| e.to_string());
            if let Some(current) = current {
                current.restore_changed(&insertion, app);
            }
            r
        }
        Ok(Read::Queue(read, asks)) => {
            app.session.place_queue_read(read).map(|r| crate::place::show_thumbnails(app, &r, asks)).map_err(|e| e.to_string())
        }
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
            let insertion = paste.insertion.or_now(current.as_ref(), app, pastes_inside(&paste.id));
            insertion.clone().set(app);
            let requested_selection = app.session.doc().map(|st| st.selection.objects.clone()).unwrap_or_default();
            app.placing.adopted = true;
            // `run` shows an error in the status bar.
            let _ = app.run(&paste.id, paste.params);
            // Also when the paste did not reach the clipboard (the Type tool took it).
            app.placing.adopted = false;
            deferred = defer_question(app, chain, &insertion, requested_selection);
            if let Some(current) = current {
                current.restore_changed(&insertion, app);
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

/// What a question a deferred paste waits on answered this pass.
pub(crate) enum Taken {
    /// The user answered; the value has the keys `command`, `params`, `deferred` and `asked`.
    Answered(Value),
    /// The question closed without an answer.
    Cancelled,
    /// The question is still open.
    Open,
}

/// The questions a paste asks before it runs, which differ in the param that holds the answer and
/// in how the changed question is told.
#[derive(Clone, Copy)]
enum Question {
    SwatchConflict,
    ProfileMismatch,
}

/// The answer of question `id`, whichever of the two it is.
fn take_question(app: &mut VectorcraftApp, id: DialogId) -> (Question, Taken) {
    match crate::dialogs::swatch_conflict::take(app, id) {
        Taken::Open => (Question::ProfileMismatch, crate::dialogs::profile_mismatch::take_paste(app, id)),
        taken => (Question::SwatchConflict, taken),
    }
}

/// Marks the question the paste just asked as the one chain `chain` waits on, and keeps the
/// paste's context for its answer → whether a question is open.
fn defer_question(app: &mut VectorcraftApp, chain: u64, insertion: &Insertion, selection: Vec<vectorcraft_doc::NodeId>) -> bool {
    let Ok((doc, reverts)) = app.session.doc().map(|st| (st.uid, st.reverts)) else { return false };
    let asked = if let Some(id) = crate::dialogs::swatch_conflict::id(app) {
        crate::dialogs::swatch_conflict::defer(app, chain);
        id
    } else if let Some(id) = crate::dialogs::profile_mismatch::paste_id(app) {
        crate::dialogs::profile_mismatch::defer_paste(app, chain);
        id
    } else {
        return false;
    };
    app.placing.swatch_paste =
        Some(PendingSwatch { clipboard: app.session.clipboard.clone(), insertion: insertion.clone(), selection, doc, reverts, chain, asked });
    true
}

/// Runs a deferred paste whose question was answered, and releases it when the question closed
/// without an answer or another dialog took the window over it.
pub(crate) fn settle_swatch_paste(app: &mut VectorcraftApp) {
    let asked = app.placing.swatch_paste.as_ref().map(|waiting| waiting.asked);
    let (question, taken) = match asked {
        Some(asked) => take_question(app, asked),
        // A paste that ran at once (automation's) waits on nothing deferred: its Swatch Conflict answer runs it.
        None => (Question::SwatchConflict, crate::dialogs::swatch_conflict::take(app, dialog_id(crate::dialogs::swatch_conflict::KIND))),
    };
    let answer = match taken {
        Taken::Open => {
            // The question is the window's one dialog: another dialog opened over it replaces it.
            if let Some(asked) = asked
                && crate::dialogs::host::top_dialog(app).is_some_and(|top| top != asked)
            {
                if let (Some(host), Some(ctx)) = (app.dialogs(), app.ui_ctx.clone()) {
                    host.with(|h| h.cancel_by_id(&ctx, asked));
                }
                cancel_swatch_paste(app);
            }
            return;
        }
        Taken::Cancelled => return cancel_swatch_paste(app),
        Taken::Answered(answer) => answer,
    };
    let command = answer["command"].as_str().unwrap_or_default().to_string();
    let seen = answer["asked"].clone();
    let changed = |app: &mut VectorcraftApp, command: &str, params: &Value| match question {
        Question::SwatchConflict => crate::dialogs::swatch_conflict::changed(app, params, &seen),
        Question::ProfileMismatch => {
            crate::dialogs::profile_mismatch::paste_changed(app, command, params, seen.as_array().map_or(&[], Vec::as_slice))
        }
    };
    let key = match question {
        Question::SwatchConflict => "swatchConflict",
        Question::ProfileMismatch => "profileMismatch",
    };
    let recheck = Recheck { key, changed: &changed };
    if let Err(e) = confirm_swatch_paste(app, &command, answer["params"].clone(), answer["deferred"].as_u64(), Some(&recheck)) {
        app.status(e);
    }
}

/// Does the paste command `id` paste inside the object Draw Inside draws into? Paste in Front and
/// Paste in Back keep their own places.
fn pastes_inside(id: &str) -> bool {
    !matches!(id, "edit.pasteInFront" | "edit.pasteInBack")
}

/// The answer a paste question gave, checked again where the paste runs (in the document and with
/// the clipboard it was given with): `key` is the param that holds the answer, and `changed` tells
/// whether the question the paste command `id` with those params would ask now differs from the one
/// answered. The paste then runs without the answer, and so asks again.
pub(crate) struct Recheck<'a> {
    pub key: &'static str,
    pub changed: &'a dyn Fn(&mut VectorcraftApp, &str, &Value) -> bool,
}

/// `params` without the answer `recheck` names when the question it answered has changed.
fn rechecked(app: &mut VectorcraftApp, id: &str, mut params: Value, recheck: Option<&Recheck<'_>>) -> Value {
    if let Some(r) = recheck
        && (r.changed)(app, id, &params)
        && let Some(o) = params.as_object_mut()
    {
        o.remove(r.key);
    }
    params
}

/// Run the command confirmed by the deferred paste's Swatch Conflict or Paste Profile Mismatch
/// dialog, asking again first when `recheck` finds that the question has changed.
pub(crate) fn confirm_swatch_paste(
    app: &mut VectorcraftApp,
    id: &str,
    params: Value,
    token: Option<u64>,
    recheck: Option<&Recheck<'_>>,
) -> Result<Value, String> {
    if !app.placing.swatch_paste.as_ref().is_some_and(|waiting| Some(waiting.chain) == token) {
        if token.is_some() || app.placing.swatch_paste.is_some() {
            cancel_swatch_paste(app);
            return Err("the deferred paste is no longer available".into());
        }
        let params = rechecked(app, id, params, recheck);
        return app.run(id, params);
    }
    let Some(waiting) = app.placing.swatch_paste.take() else { return app.run(id, params) };
    let Some(index) = app.session.documents().iter().position(|st| st.uid == waiting.doc) else {
        finish_chain(app, waiting.chain);
        return Err("Couldn't paste: the document it was pasted in was closed".into());
    };
    // File → Revert threw away the content the paste was given in: nothing is pasted, and the
    // clipboard, selection and insertion context stay as they are.
    if app.session.documents().get(index).is_some_and(|st| st.reverts != waiting.reverts) {
        finish_chain(app, waiting.chain);
        return Err("Couldn't paste: the document it was pasted in was reverted".into());
    }
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
    let insertion = waiting.insertion.clone().or_now(changed.as_ref(), app, pastes_inside(id));
    insertion.clone().set(app);
    // The question checked again with the clipboard and document the paste runs with.
    let params = rechecked(app, id, params, recheck);
    app.placing.adopted = true;
    let result = app.run(id, params);
    app.placing.adopted = false;
    // The paste asked again (Swatch Conflict after Paste Profile Mismatch, or the same question
    // about what changed): it stays deferred, in the context it was given in.
    if defer_question(app, waiting.chain, &insertion, waiting.selection.clone()) {
        if let Some(changed) = changed {
            changed.restore_changed(&insertion, app);
        }
        app.session.clipboard = current;
        return result;
    }
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

/// The progress window's Cancel: the files being read for Place aren't placed.
pub fn cancel_all(app: &mut VectorcraftApp) -> Value {
    let mut cancelled = 0;
    let mut i = 0;
    while i < app.placing.pending.len() {
        if !app.placing.pending[i].paste {
            let p = app.placing.pending.remove(i);
            p.token.cancel();
            if app.ui.status == format!("{}…", p.label) {
                app.ui.status.clear();
            }
            if let Some(chain) = p.chain
                && !app.placing.swatch_paste.as_ref().is_some_and(|waiting| waiting.chain == chain)
            {
                finish_chain(app, chain);
            }
            cancelled += 1;
        } else {
            i += 1;
        }
    }
    json!({ "cancelled": cancelled })
}

/// Can any placed file being read be cancelled?
pub fn can_cancel(app: &VectorcraftApp) -> bool {
    app.placing.pending.iter().any(|p| !p.paste)
}

/// Is a placed file being read that the progress window shows?
pub fn any_shown(app: &VectorcraftApp) -> bool {
    app.placing.pending.iter().any(|p| !p.paste && !p.hidden)
}

/// The progress window's entry for the first file being read for Place; it can only be cancelled.
pub(crate) fn shown(app: &VectorcraftApp) -> Option<Shown> {
    let waiting: Vec<&Pending> = app.placing.pending.iter().filter(|p| !p.paste && !p.hidden).collect();
    let first = *waiting.first()?;
    let more = waiting.len() - 1;
    Some(Shown {
        title: format!("{}…", crate::i18n::msg(&first.label)),
        step: tl!("Reading the file…").to_string(),
        fraction: None,
        counts: None,
        queued: (more > 0).then(|| crate::i18n::tn(more as u64, "{n} more file waiting", "{n} more files waiting").to_string()),
        started: first.started,
        cancel: "file.cancelPlace",
        hide: None,
    })
}

#[cfg(test)]
mod tests;
