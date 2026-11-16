//! File → Place in the app: picking the files, the Place dialog's options, files dropped on the
//! window, the loaded place cursor and the Control bar's image details. The engine places
//! (`file.place`, `file.place.queue`); this gathers the files and options.

use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use vectorcraft_doc::NodeKind;
use vectorcraft_engine::cmd::fileio;
use vectorcraft_geom::Point;

use crate::theme::{self, Tokens};
use crate::{VectorcraftApp, io, widgets};

/// The loaded place cursor's thumbnail size (px on the longer side).
const THUMB: u32 = 64;
/// Where the thumbnail sits from the pointer (points).
const THUMB_OFFSET: egui::Vec2 = egui::vec2(14.0, 14.0);

/// A file to place that arrived asynchronously (web): picked for File → Place (`drop: None`), or
/// dropped on a canvas.
pub struct PlaceArrival {
    pub name: String,
    pub bytes: Vec<u8>,
    pub drop: Option<DropAt>,
}

pub type PlaceInbox = Arc<Mutex<Vec<PlaceArrival>>>;

/// Where files dropped on a canvas land: document `doc` ([`vectorcraft_engine::DocState::uid`]) at
/// `at` (document coordinates), embedded when Shift was held. The document is kept because the web
/// reads dropped files asynchronously: another document may have become active when they arrive.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DropAt {
    pub doc: u64,
    /// The document's [`vectorcraft_engine::DocState::reverts`] at the drop: files that arrive
    /// after File → Revert threw that content away are not placed.
    pub reverts: u64,
    pub at: Point,
    pub embed: bool,
}

/// Where a file dropped on the window goes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DropTarget {
    /// The canvas of the document it was dropped on.
    Place(DropAt),
    /// Opened as File → Open opens it: no document is open, the drop missed the canvas (the tab
    /// bar), it is a document dropped where the platform doesn't say, or it is a library, a presets
    /// file, a plug-in or a shortcut set.
    Open,
}

/// Does a file named `name` open as a document of its own (vector art: native, SVG, PDF, `.ai`,
/// EPS, DXF, metafiles) rather than being a picture or text to place?
pub fn is_document(name: &str) -> bool {
    fileio::format_for_name(name).is_some_and(|f| f.read && !f.raster)
}

/// The app's Place state (not saved with the preferences).
#[derive(Default)]
pub struct PlaceState {
    /// Web: the bytes of the files picked for the Place dialog, by name.
    pub(crate) picked: Vec<(String, Vec<u8>)>,
    /// The loaded cursor's thumbnails by file name.
    thumbs: Vec<(String, Thumb)>,
    /// The Control bar's `image.info`, for (document uid, revision, image id).
    info: Option<((u64, u64, u64), Value)>,
    /// The Place dialog's lines about its files while they are read off the UI thread: the
    /// dialog's files, and the lines once they are read.
    reading_lines: Option<(Value, std::sync::mpsc::Receiver<Vec<String>>)>,
    /// The files of the loaded place cursor ask for their import options when clicked or dragged
    /// (the Place dialog loaded it with Show Import Options on).
    queue_asks: bool,
    /// The loaded cursor's files by name, with the size a click places them at.
    queue_sizes: Vec<(String, f64, f64)>,
    /// The names of the loaded cursor's encrypted PDF files, which ask for their password.
    queue_locked: Vec<String>,
    /// The engine's count of cursor loads ([`vectorcraft_engine::Session::place_queue_loads`]) when
    /// `queue_asks`, `queue_sizes` and `queue_locked` were set: a later load (`tool.setOption`, a
    /// script) clears them.
    queue_load: u64,
    /// The import options dialog a click of the cursor opened (the token in its params'
    /// [`ASKED`]), with the automation roots the cursor was loaded in: its OK, pressed outside any
    /// roots, places the file within them. Cleared when the cursor is loaded again.
    asked: Option<(u64, std::sync::Arc<vectorcraft_engine::file_access::AutomationRoots>)>,
}

/// The `file.place` param that ties an import options dialog's OK to the click of the place cursor
/// that opened it ([`PlaceState::asked`]); hidden from the dialog's fields and removed before the
/// file is placed.
const ASKED: &str = "__placeCursorAsk";

/// The `file.place` param that keeps where a file dropped on a canvas goes (`{doc, reverts}`, as
/// [`DropAt`]) through the import options dialog the drop opened, so its OK places the file in that
/// document; hidden from the dialog's fields and removed before the file is placed.
const DROPPED: &str = "__dropTarget";

/// A place cursor thumbnail: decoded when the cursor is loaded, uploaded when first drawn (and
/// again in a new graphics context).
struct Thumb {
    image: std::sync::Arc<egui::ColorImage>,
    tex: Option<egui::TextureHandle>,
}

impl PlaceState {
    /// The UI moved to a new egui context: the thumbnails are uploaded again when drawn.
    pub(crate) fn forget_textures(&mut self) {
        for (_, th) in &mut self.thumbs {
            th.tex = None;
        }
    }

    /// The names of the files the loaded cursor has thumbnails of.
    #[cfg(test)]
    pub(crate) fn thumbnails(&self) -> Vec<&str> {
        self.thumbs.iter().map(|(n, _)| n.as_str()).collect()
    }
}

/// Run engine command `id` itself (past the UI's handling of the same id); an error shows in the
/// status bar.
fn engine(app: &mut VectorcraftApp, id: &str, p: &Value) -> Result<Value, String> {
    let r = app.session.execute(id, p).map_err(|e| e.to_string());
    app.sync_views();
    if let Err(e) = &r {
        app.status(e.clone());
    }
    r
}

// ---------- File → Place… ----------

/// File → Place… (`file.place`): with a file (`path` or `name` + `dataBase64`), place it, centred
/// in the view unless `at`, `rect` or `replace` say otherwise (a text file without `text` options,
/// a DXF drawing without `dxf` options, a PSD file embedded without `layers` (when the user places
/// it) and a PDF with several pages or a password and no `page` ask for them first, unless
/// `showImportOptions: false` places them with the options confirmed last); with `paths`, the
/// Place dialog for them; with neither, pick the files first. The file is read and placed at once
/// (MCP, scripts).
pub fn run(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    place_file(app, p, Read::AtOnce)
}

/// [`run`] for a file the user places (the Place dialog's OK, [`io::place_bytes`], the OK of the
/// import options Place asks for): on the desktop the file is read off the UI thread
/// ([`crate::placing::start`]), and the result is then `{background: true, status}`.
pub fn run_for_user(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    place_file(app, p, Read::ForUser { dropped: false })
}

/// How [`place_file`] and [`queue_files`] read files: at once, or for the user, off the UI thread
/// where they can ([`crate::placing::available`]); `dropped` when the file was dropped on the canvas.
#[derive(Clone, Copy)]
enum Read {
    AtOnce,
    ForUser { dropped: bool },
}

fn place_file(app: &mut VectorcraftApp, p: &Value, mut read: Read) -> Result<Value, String> {
    // A file dropped on a canvas, and the OK of the import options its drop asked for, go to the
    // document it was dropped on, which becomes active again; not when that document has closed or
    // been reverted since.
    if let Some(target) = p.get(DROPPED) {
        let field = |k: &str| target.get(k).and_then(Value::as_u64);
        let Some(i) = app.session.documents().iter().position(|st| Some(st.uid) == field("doc")) else {
            return Err("the document it was dropped on was closed".into());
        };
        if app.session.documents().get(i).is_some_and(|st| Some(st.reverts) != field("reverts")) {
            return Err("the document it was dropped on was reverted".into());
        }
        app.session.set_active(i);
        if let Read::ForUser { .. } = read {
            read = Read::ForUser { dropped: true };
        }
    }
    // The OK of import options a click of the place cursor asked for places within the roots the
    // cursor was loaded in. A caller with roots of its own places within those.
    let Some(token) = p.get(ASKED) else { return place_file_in(app, p, read) };
    forget_older_queue(app);
    let Some(roots) = app.place.asked.as_ref().filter(|(t, _)| Some(*t) == token.as_u64()).map(|(_, r)| r.clone()) else {
        return Err("the place cursor that asked for these import options is no longer loaded".into());
    };
    if vectorcraft_engine::file_access::current().is_some() {
        return place_file_in(app, p, read);
    }
    vectorcraft_engine::file_access::confine(Some(&roots), || place_file_in(app, p, read))
}

fn place_file_in(app: &mut VectorcraftApp, p: &Value, read: Read) -> Result<Value, String> {
    let show = match p.get("showImportOptions") {
        None | Some(Value::Null) => None,
        Some(Value::Bool(b)) => Some(*b),
        Some(_) => return Err("`showImportOptions` must be true or false".into()),
    };
    let ask = show.unwrap_or(true);
    let mut p = p.clone();
    if let Some(o) = p.as_object_mut() {
        o.remove("showImportOptions");
    }
    if !ask {
        // Placed with the options confirmed last for the format.
        if p.get("text").is_none() && crate::dialogs::text_import::is_text(&p) {
            p["text"] = crate::dialogs::text_import::last_options(app);
        }
        if let Some(dxf) = crate::dialogs::dxf_import::last_options(app, &p) {
            p["dxf"] = dxf;
        }
        crate::dialogs::psd_import::last_options(app, &mut p);
    }
    if p.get("text").is_none() && crate::dialogs::text_import::is_text(&p) {
        return Ok(crate::dialogs::text_import::open(app, &p));
    }
    if p.get("path").is_none() && p.get("dataBase64").is_none() {
        let files: Vec<Value> = match p.get("paths").and_then(Value::as_array) {
            Some(a) => a.iter().filter_map(Value::as_str).map(|path| json!({ "path": path })).collect(),
            None => return pick(app).map(|_| Value::Null),
        };
        if files.is_empty() {
            return Err("paths: give at least one file".into());
        }
        open_dialog(app, files);
        if let (Some(show), Some(d)) = (show, app.ui.dialog.as_mut()) {
            d.fields.insert("showImportOptions".into(), json!(show));
        }
        return Ok(Value::Null);
    }
    // A PDF with several pages or a password asks for the page, box and password first; with
    // Show Import Options off, or with a `page` given, only an encrypted one asks, for its password.
    let offered = (ask && crate::dialogs::import_pdf::offer_place(app, &p)) || crate::dialogs::import_pdf::offer_place_password(app, &p);
    if offered {
        return Ok(Value::Null);
    }
    // A DXF drawing asks for its import options.
    if ask && crate::dialogs::dxf_import::offer_place(app, &p) {
        return Ok(json!({ "dialog": crate::dialogs::dxf_import::KIND }));
    }
    // A PSD file with several layers or with slices that the user embeds asks for its import
    // options. A place run as a command (MCP, scripts) places at once and takes `layers` and
    // `importSlices` itself.
    if ask && matches!(read, Read::ForUser { .. }) && crate::dialogs::psd_import::offer_place(app, &p) {
        return Ok(json!({ "dialog": crate::dialogs::psd_import::KIND }));
    }
    if ["at", "rect", "replace"].iter().all(|k| p.get(k).is_none())
        && let Some(v) = app.view()
    {
        p["at"] = json!([v.center.x, v.center.y]);
    }
    if let Some(o) = p.as_object_mut() {
        o.remove(ASKED);
        o.remove(DROPPED);
    }
    match read {
        Read::ForUser { dropped } if crate::placing::available(app) => {
            let r = crate::placing::start(app, &p, dropped);
            if let Err(e) = &r {
                app.status(e.clone());
            }
            r
        }
        _ => engine(app, "file.place", &p),
    }
}

/// Pick files to place: the Place dialog opens with them (on the web once they arrive).
fn pick(app: &mut VectorcraftApp) -> Result<(), String> {
    if let Some(f) = app.services.place_async.as_mut() {
        f();
        return Ok(());
    }
    let paths = if app.services.pick_open_multi.is_some() {
        crate::picks::open_many(app)
    } else {
        crate::picks::open(app, &crate::FilePick { filters: crate::FilePick::filters(fileio::place_filters()), ..Default::default() })
            .into_iter()
            .collect()
    };
    if paths.is_empty() {
        return Err("cancelled".into());
    }
    open_dialog(app, paths.into_iter().map(|path| json!({ "path": path })).collect());
    Ok(())
}

/// The Place dialog for `files` (`{path}`, or `{name}` with the bytes in [`PlaceState`]).
fn open_dialog(app: &mut VectorcraftApp, files: Vec<Value>) {
    let units = app.session.active().map(|d| d.doc.units).unwrap_or_default();
    let info = lines(app, &files, units);
    let one_object = app.session.active().is_some_and(|d| d.selection.objects.len() == 1);
    let linked: Vec<bool> = files.iter().map(|f| is_vectorcraft_file(app, f)).collect();
    app.ui.dialog = Some(crate::state::Dialog::new(
        crate::dialogs::place::KIND,
        json!({
            "files": files,
            "link": app.ui.place_link,
            "template": false,
            "replace": false,
            "showImportOptions": app.ui.place_import_options,
            "__replace": files.len() == 1 && one_object,
            "__documents": linked.contains(&true),
            "__others": linked.contains(&false),
            "__info": info,
        }),
    ));
}

/// Is dialog file entry `file` a VectorCraft document on disk (what Link places as one locked
/// object), or a PDF or `.ai` file on disk? A `.ai` saved without PDF compatibility is PostScript,
/// which `file.place` embeds whether Link is on or not.
fn is_vectorcraft_file(app: &VectorcraftApp, file: &Value) -> bool {
    let Some(path) = file.get("path").and_then(Value::as_str) else { return false };
    let native = ["vectorcraft", vectorcraft_format::LEGACY_EXTENSION, "vctemplate"];
    match fileio::extension(path).as_str() {
        e if native.contains(&e) => true,
        "pdf" | "ai" | "ait" => app.services.read.as_ref().and_then(|r| r(path).ok()).is_none_or(|b| !vectorcraft_pdf::is_postscript(&b)),
        _ => false,
    }
}

/// The engine params naming `file` (a dialog file entry).
fn source(app: &VectorcraftApp, file: &Value) -> Option<Value> {
    if let Some(path) = file.get("path").and_then(Value::as_str) {
        return Some(json!({ "path": path }));
    }
    let name = file.get("name").and_then(Value::as_str)?;
    let bytes = app.place.picked.iter().find(|(n, _)| n == name).map(|(_, b)| b)?;
    Some(json!({ "name": name, "dataBase64": vectorcraft_format::base64_encode(bytes) }))
}

/// The Place dialog's line about each of `files`, read at once, or where the files the user places
/// are read off the UI thread ([`crate::placing::available`]) on a thread of its own: the lines are
/// then empty until [`fill_lines`] shows them.
fn lines(app: &mut VectorcraftApp, files: &[Value], units: vectorcraft_doc::Unit) -> Vec<String> {
    if crate::placing::available(app) {
        let sources: Vec<Option<Value>> = files.iter().map(|f| source(app, f)).collect();
        let (tx, read) = std::sync::mpsc::channel();
        // The automation roots in force here are in force there.
        let scope = vectorcraft_engine::file_access::ScopeHandle::capture();
        let spawned = std::thread::Builder::new().name("vectorcraft-place-info".into()).spawn(move || {
            let lines: Vec<String> = scope.run(|| {
                sources
                    .iter()
                    .map(|p| match p {
                        None => "not found".to_string(),
                        Some(p) => line(
                            vectorcraft_engine::guard::catch_panic(|| vectorcraft_engine::cmd::place::read_info(p).map_err(|e| e.to_string()))
                                .unwrap_or_else(|msg| Err(format!("internal error: {msg} (please report this bug)"))),
                            units,
                        ),
                    })
                    .collect()
            });
            // The app may have quit meanwhile: nothing to hand them to.
            let _ = tx.send(lines);
        });
        if spawned.is_ok() {
            app.place.reading_lines = Some((json!(files), read));
            return vec![String::new(); files.len()];
        }
    }
    files.iter().map(|f| summary(app, f, units)).collect()
}

/// The open Place dialog `d`'s lines about its files once they have been read off the UI thread
/// ([`lines`]) → whether they are still being read.
pub(crate) fn fill_lines(app: &mut VectorcraftApp, d: &mut crate::state::Dialog) -> bool {
    let Some((files, read)) = &app.place.reading_lines else { return false };
    if d.fields.get("files") != Some(files) {
        return false;
    }
    match read.try_recv() {
        Ok(lines) => {
            d.fields.insert("__info".into(), json!(lines));
            app.place.reading_lines = None;
            false
        }
        Err(std::sync::mpsc::TryRecvError::Empty) => true,
        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
            app.place.reading_lines = None;
            false
        }
    }
}

/// One line about `file` for the Place dialog, read at once ([`line`]).
fn summary(app: &mut VectorcraftApp, file: &Value, units: vectorcraft_doc::Unit) -> String {
    let Some(p) = source(app, file) else { return "not found".into() };
    line(app.session.execute("file.place.info", &p).map_err(|e| e.to_string()), units)
}

/// One line for the Place dialog from `file.place.info`'s reply `i` ("300 × 150 px, 300 ppi, RGB
/// (1 in × 0.5 in)"), or why the file can't be placed.
fn line(i: Result<Value, String>, units: vectorcraft_doc::Unit) -> String {
    let i = match i {
        Ok(i) => i,
        Err(e) => return e,
    };
    let size = format!("{} × {}", units.format(i["width"].as_f64().unwrap_or(0.0)), units.format(i["height"].as_f64().unwrap_or(0.0)));
    match (i["pixelWidth"].as_u64(), i["pixelHeight"].as_u64()) {
        (Some(w), Some(h)) => {
            let ppi = i["ppi"][0].as_f64().unwrap_or(72.0);
            format!("{w} × {h} px, {} ppi, {} ({size})", fmt_ppi(ppi), i["colorMode"].as_str().unwrap_or("RGB"))
        }
        _ => format!("{} ({size})", i["format"].as_str().unwrap_or_default().to_uppercase()),
    }
}

/// A resolution as the Control bar and the Place dialog show it ("300", "299.5").
fn fmt_ppi(v: f64) -> String {
    let s = format!("{v:.1}");
    s.strip_suffix(".0").map(str::to_string).unwrap_or(s)
}

/// Place (the dialog's OK): one file is placed centred in the view (or replaces the selection);
/// several load the place cursor. With Show Import Options on, each file that has import options
/// asks for them before it is placed (the cursor's files when clicked or dragged); off, each is
/// placed with the options confirmed last for its format.
pub fn confirm(app: &mut VectorcraftApp, d: &crate::state::Dialog) -> Result<Value, String> {
    let files: Vec<Value> = d.fields.get("files").and_then(Value::as_array).cloned().unwrap_or_default();
    let sources: Vec<Value> = files.iter().filter_map(|f| source(app, f)).collect();
    if sources.is_empty() {
        return Err("no files to place".into());
    }
    let (link, template, ask) = (d.bool("link"), d.bool("template"), d.bool("showImportOptions"));
    app.ui.place_link = link;
    app.ui.place_import_options = ask;
    app.ui.dialog = None;
    app.place.reading_lines = None;
    let r = match &sources[..] {
        [one] => {
            let mut p = one.clone();
            p["link"] = json!(link);
            p["template"] = json!(template);
            p["showImportOptions"] = json!(ask);
            if d.bool("replace") && d.bool("__replace") {
                p["replace"] = json!(true);
            }
            run_for_user(app, &p)
        }
        many => {
            let paths: Vec<&Value> = many.iter().filter_map(|s| s.get("path")).collect();
            let files: Vec<&Value> = many.iter().filter(|s| s.get("path").is_none()).collect();
            let mut q = json!({ "paths": paths, "files": files, "link": link, "template": template });
            if !ask {
                // Each text file, DXF drawing and embedded PSD file in the cursor takes the options
                // confirmed last.
                q["text"] = crate::dialogs::text_import::last_options(app);
                q["dxf"] = crate::dialogs::dxf_import::remembered(app);
                if !link || paths.is_empty() {
                    // Files from bytes have no file to link to; they may carry the dialog's Link.
                    q["link"] = json!(false);
                    crate::dialogs::psd_import::queue_options(app, &mut q);
                }
            }
            queue_files(app, &q, Read::ForUser { dropped: false }, ask)
        }
    };
    app.place.picked.clear();
    r
}

// ---------- the place cursor ----------

/// `file.place.queue` with thumbnails for the cursor (unless `thumbnail` is given), the files read
/// at once (MCP, scripts).
pub fn queue(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    queue_files(app, p, Read::AtOnce, false)
}

/// `file.place.queue` with `p`; `asks`: the cursor's files ask for their import options when placed.
fn queue_files(app: &mut VectorcraftApp, p: &Value, read: Read, asks: bool) -> Result<Value, String> {
    let mut p = p.clone();
    if p.get("thumbnail").is_none() {
        p["thumbnail"] = json!(THUMB);
    }
    if matches!(read, Read::ForUser { .. }) && crate::placing::available(app) {
        let r = crate::placing::start_queue(app, &p, asks);
        if let Err(e) = &r {
            app.status(e.clone());
        }
        return r;
    }
    let r = engine(app, "file.place.queue", &p)?;
    show_thumbnails(app, &r, asks);
    Ok(r)
}

/// The file a click or drag of the place cursor places (`file.place` with `p`, its `rect` set):
/// read off the UI thread where the files the user places are ([`crate::placing::start`]), else at
/// once. When the Place dialog loaded the cursor with Show Import Options on, a file that has
/// import options asks for them first (Cancel leaves it unplaced); otherwise only an encrypted PDF
/// asks, for its password. An error shows in the status bar.
pub(crate) fn place_from_cursor(app: &mut VectorcraftApp, p: &Value) {
    // Read within the automation roots of whoever loaded the cursor, even on a click of the person
    // at the keyboard: an agent's queue reaches no file it wasn't given.
    let roots = app.session.place_queue_roots();
    forget_older_queue(app);
    vectorcraft_engine::file_access::confine(roots.as_ref(), || {
        let locked = is_locked(app, p);
        if app.place.queue_asks || locked {
            let mut p = clicked_at_full_size(app, p);
            if !app.place.queue_asks {
                // Only its password is asked for, as File › Place with Show Import Options off.
                p["showImportOptions"] = json!(false);
            }
            // The import options it may ask for keep the token, and their OK places within the
            // same roots ([`place_file`]).
            app.place.asked = roots.clone().map(|r| {
                static TOKEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
                let token = TOKEN.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                p[ASKED] = json!(token);
                (token, r)
            });
            // Placing shows its own errors in the status bar.
            let _ = place_file(app, &p, Read::ForUser { dropped: false });
        } else if crate::placing::available(app) {
            if let Err(e) = crate::placing::start(app, p, false) {
                app.status(e);
            }
        } else {
            let _ = engine(app, "file.place", p);
        }
    });
}

/// The cursor was loaded again since [`show_thumbnails`] (`tool.setOption` with a `queue`, a
/// script): its files no longer ask for their import options, and their queued sizes are unknown.
fn forget_older_queue(app: &mut VectorcraftApp) {
    if app.place.queue_load != app.session.place_queue_loads() {
        app.place.queue_asks = false;
        app.place.queue_sizes.clear();
        app.place.queue_locked.clear();
        app.place.asked = None;
    }
}

/// Does `p` (a cursor click's `file.place` params) name a file the cursor holds as an encrypted PDF?
fn is_locked(app: &VectorcraftApp, p: &Value) -> bool {
    let name = p.get("path").or_else(|| p.get("name")).and_then(Value::as_str).map(fileio::file_name);
    name.is_some_and(|n| app.place.queue_locked.contains(&n))
}

/// The params of a cursor click (`rect` at the file's queued size) on a PDF or DXF file, whose
/// import options may change its size (another page, another scale): placed as its options place
/// it at the click (`at`, the center of the box the click gave), as File › Place of that one file
/// places it, instead of fitted into the box of the first page or the default scale. A drag keeps
/// its `rect`; one to exactly the queued size gives the same `rect` as a click, and is taken as one.
fn clicked_at_full_size(app: &VectorcraftApp, p: &Value) -> Value {
    let mut p = p.clone();
    let Some(name) = p.get("path").or_else(|| p.get("name")).and_then(Value::as_str).map(fileio::file_name) else { return p };
    if !matches!(fileio::extension(&name).as_str(), "pdf" | "ai" | "ait" | "dxf") {
        return p;
    }
    let r: Vec<f64> = p.get("rect").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_f64).collect()).unwrap_or_default();
    let [x, y, w, h] = r[..] else { return p };
    // Files of the same name in the cursor must all have that size: the place tool's request names
    // the file, not its place in the cursor.
    let mut same_name = app.place.queue_sizes.iter().filter(|(n, ..)| *n == name).peekable();
    let queued = same_name.peek().is_some() && same_name.all(|(_, qw, qh)| (qw - w).abs() < 1e-6 && (qh - h).abs() < 1e-6);
    if queued && let Some(o) = p.as_object_mut() {
        o.remove("rect");
        o.insert("at".into(), json!([x + w / 2.0, y + h / 2.0]));
    }
    p
}

/// The cursor's thumbnails and its files' sizes from `file.place.queue`'s reply `r` once the cursor
/// is loaded; `asks`: its files ask for their import options when placed.
pub(crate) fn show_thumbnails(app: &mut VectorcraftApp, r: &Value, asks: bool) {
    app.place.queue_load = app.session.place_queue_loads();
    app.place.queue_asks = asks;
    app.place.asked = None;
    app.place.queue_sizes = r["files"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|f| Some((f["name"].as_str()?.to_string(), f["width"].as_f64()?, f["height"].as_f64()?)))
        .collect();
    app.place.queue_locked = r["files"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|f| f["needsPassword"] == true)
        .filter_map(|f| Some(f["name"].as_str()?.to_string()))
        .collect();
    app.place.thumbs = r["files"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|f| {
            let png = vectorcraft_format::base64_decode(f["thumbnailBase64"].as_str()?)?;
            let img = craft_image_codecs::decode_as(craft_image_codecs::Format::Png, &png).ok()?;
            let color = egui::ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], &img.to_rgba8());
            Some((f["name"].as_str()?.to_string(), Thumb { image: std::sync::Arc::new(color), tex: None }))
        })
        .collect();
}

/// The loaded place cursor at `pos`: the current file's thumbnail with the number of files loaded.
pub fn paint_cursor(app: &mut VectorcraftApp, ctx: &egui::Context, painter: &egui::Painter, pos: egui::Pos2) {
    let opts = app.session.tool_options();
    let count = opts["count"].as_u64().unwrap_or(0);
    let name = opts["name"].as_str().unwrap_or_default();
    let t = Tokens::get(ctx);
    let tex = app.place.thumbs.iter_mut().find(|(n, _)| n == name).map(|(n, th)| {
        let tex = th.tex.get_or_insert_with(|| ctx.load_texture(format!("place-thumb-{n}"), th.image.clone(), egui::TextureOptions::LINEAR));
        (tex.id(), tex.size_vec2())
    });
    let (id, size) = tex.unzip();
    let size = size.unwrap_or(egui::vec2(THUMB as f32, THUMB as f32 * 0.75));
    let k = THUMB as f32 / size.x.max(size.y).max(1.0);
    let r = egui::Rect::from_min_size(pos + THUMB_OFFSET, size * k);
    painter.rect_filled(r.expand(1.0), 0.0, t.panel);
    match id {
        Some(id) => {
            painter.image(id, r, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::from_white_alpha(200));
        }
        None => {
            painter.text(r.center(), egui::Align2::CENTER_CENTER, name, theme::semibold(10.0), t.text_dim);
        }
    }
    painter.rect_stroke(r.expand(1.0), 0.0, egui::Stroke::new(1.0, t.border), egui::StrokeKind::Outside);
    if count > 1 {
        // The badge: how many files are loaded.
        let c = r.right_top() + egui::vec2(-2.0, 2.0);
        painter.circle_filled(c, 9.0, t.accent);
        painter.text(c, egui::Align2::CENTER_CENTER, count.to_string(), theme::semibold(10.5), egui::Color32::WHITE);
    }
}

// ---------- drops ----------

impl VectorcraftApp {
    /// Where file `name` dropped with the pointer at `pos` (screen points) goes: as in Illustrator,
    /// onto the canvas of the open document (Shift embeds it), else opened. Where the platform
    /// doesn't say (`None`: desktop drags carry no position), a document opens as a tab of its
    /// own and a picture or text is placed in the middle of the view. A library, a presets file, a
    /// plug-in, a shortcut set, a script or a file only an importer plug-in reads opens wherever it
    /// is dropped, as File → Open opens it ([`io::opens_as`], [`fileio::opens_with_importer`]).
    pub fn drop_target(&self, name: &str, pos: Option<egui::Pos2>, shift: bool) -> DropTarget {
        let (Some(rect), Some(st), Some(v)) = (self.canvas_rect, self.session.active(), self.view()) else { return DropTarget::Open };
        let at = match pos {
            _ if io::opens_as(name).is_some() || fileio::opens_with_importer(name) => return DropTarget::Open,
            Some(p) if !rect.contains(p) => return DropTarget::Open,
            Some(p) => crate::canvas::Xf::at(rect, v, self.canvas_ui_zoom).to_doc(p),
            None if is_document(name) => return DropTarget::Open,
            None => v.center,
        };
        DropTarget::Place(DropAt { doc: st.uid, reverts: st.reverts, at, embed: shift })
    }
}

/// Where the pointer is during a file drag over the window, as far as egui knows: nowhere on the
/// desktop, whose drags carry no position (the one egui had before is stale; files dropped there
/// go by their kind, see [`VectorcraftApp::drop_target`]). On the web, egui's own position (for
/// the highlight: the web host follows its drags itself for drops).
pub fn drag_pos(ctx: &egui::Context) -> Option<egui::Pos2> {
    if cfg!(target_arch = "wasm32") { ctx.input(|i| i.pointer.latest_pos()) } else { None }
}

/// A file dropped on the window: its name, path (when it has one) and bytes.
pub type DropFile = (String, Option<String>, Vec<u8>);

/// Files dropped on the window, each placed on a canvas or opened (and added to Open Recent
/// Files) as its target says: the placed ones first, so the last document opened is the one
/// shown. A file for a canvas goes to the document it was dropped on, which becomes active again;
/// it isn't placed when that document has closed or been reverted since, and neither is it by the
/// OK of an import options dialog the drop opened ([`DROPPED`]).
pub fn drop_files(app: &mut VectorcraftApp, mut files: Vec<(DropTarget, DropFile)>) {
    files.sort_by_key(|(target, _)| *target == DropTarget::Open);
    for (target, (name, path, bytes)) in files {
        let r = match target {
            DropTarget::Place(DropAt { doc, reverts, at, embed }) => {
                let mut p = match &path {
                    Some(path) => json!({ "path": path }),
                    None => json!({ "name": name, "dataBase64": vectorcraft_format::base64_encode(&bytes) }),
                };
                p["at"] = json!([at.x, at.y]);
                p["link"] = json!(!embed);
                p[DROPPED] = json!({ "doc": doc, "reverts": reverts });
                place_file(app, &p, Read::ForUser { dropped: true }).map(|_| ())
            }
            DropTarget::Open => match path.as_deref() {
                Some(path) if app.services.read.is_some() => io::open_path(app, path).map(|_| ()),
                _ => io::open_bytes(app, &name, &bytes, path.clone()).map(|_| {
                    if let Some(path) = &path {
                        io::note_recent(app, path);
                    }
                }),
            },
        };
        match r {
            Err(e) if target == DropTarget::Open => io::report_open_error(app, &name, &e),
            Err(e) => app.status(format!("Couldn't place {name}: {e}")),
            Ok(()) => {}
        }
    }
}

/// Files that arrived to be placed (web): drops go where they were dropped, picked files to the
/// Place dialog.
pub fn drain(app: &mut VectorcraftApp) {
    let arrived: Vec<PlaceArrival> =
        app.services.place_inbox.as_ref().map(|q| std::mem::take(&mut *q.lock().unwrap_or_else(|e| e.into_inner()))).unwrap_or_default();
    let mut picked = vec![];
    for a in arrived {
        match a.drop {
            Some(d) => drop_files(app, vec![(DropTarget::Place(d), (a.name, None, a.bytes))]),
            None => {
                picked.push(json!({ "name": a.name }));
                app.place.picked.retain(|(n, _)| *n != a.name);
                app.place.picked.push((a.name, a.bytes));
            }
        }
    }
    if !picked.is_empty() {
        open_dialog(app, picked);
    }
}

/// Files dragged over the window: the canvas is outlined where they would be placed. On the web
/// the browser gives a dragged file's type but not its name, so the canvas is outlined for any
/// file dragged over it, including libraries, presets, plug-ins and shortcut sets, which open when
/// dropped.
pub fn paint_drop_highlight(app: &VectorcraftApp, ctx: &egui::Context, painter: &egui::Painter, rect: egui::Rect) {
    let pos = drag_pos(ctx);
    let placed = ctx.input(|i| {
        i.raw.hovered_files.iter().any(|f| {
            let name = f.path.as_deref().map(|p| p.to_string_lossy()).unwrap_or_default();
            app.drop_target(&name, pos, false) != DropTarget::Open
        })
    });
    if !placed {
        return;
    }
    let t = Tokens::get(ctx);
    painter.rect_filled(rect, 0.0, t.accent.gamma_multiply(0.08));
    painter.rect_stroke(rect.shrink(1.5), 0.0, egui::Stroke::new(3.0, t.accent), egui::StrokeKind::Inside);
}

// ---------- the Control bar ----------

/// `image.info` of the one selected image, fetched again only when it changes (the Control bar's
/// and the Properties panel's image details).
pub fn selected_image_info(app: &mut VectorcraftApp) -> Option<&Value> {
    let st = app.session.active()?;
    let key = match &st.selection.objects[..] {
        [id] if st.doc.node(*id).is_some_and(|n| matches!(n.kind, NodeKind::Image(_))) => (st.uid, st.revision, id.0),
        _ => return None,
    };
    if app.place.info.as_ref().is_none_or(|(k, _)| *k != key) {
        let info = app.session.execute("image.info", &json!({ "id": key.2 })).unwrap_or_default();
        app.place.info = Some((key, info));
    }
    app.place.info.as_ref().map(|(_, i)| i)
}

/// An image's file name, and its colour mode and effective resolution (`image.info` `i`).
pub fn image_summary(i: &Value) -> (String, String) {
    let name = i["link"].as_str().map(fileio::file_name).or_else(|| i["name"].as_str().map(str::to_string)).unwrap_or_default();
    let ppi = i["ppi"].as_array().and_then(|a| a.first()).and_then(Value::as_f64).unwrap_or(0.0);
    (name, format!("{}   PPI: {}", i["colorMode"].as_str().unwrap_or("RGB"), fmt_ppi(ppi)))
}

/// The Control bar for the one selected image: Linked File or Embedded, its file name, colour mode
/// and effective resolution, Embed or Unembed, Edit Original (a linked file), Image Trace, Mask,
/// Crop Image and Remove Background. While the Crop Image tool is in use, the bar shows the file
/// name, colour mode and resolution only, and the tool's cropping options follow them. Whether one
/// image is selected. A file name the row has no room for is left out alone; after it, each control
/// shows only while the row has room for it and `reserve` after it, and the controls after one left
/// out are left out too.
pub fn control_bar_details(app: &mut VectorcraftApp, ui: &mut egui::Ui, reserve: f32) -> bool {
    let Some(i) = selected_image_info(app) else { return false };
    let (name, details) = image_summary(i);
    let (Some(id), linked) = (i["id"].as_u64(), i["linked"] == true) else { return false };
    let t = Tokens::get(ui.ctx());
    // The file name is a link: its menu relinks (replaces) the image and reaches the Links
    // panel's other actions for it.
    let name_text = egui::RichText::new(&name).size(12.0).color(t.accent).underline();
    if crate::chrome::Fit::new(reserve).room(ui, &[widgets::text_width(ui, name_text.clone())]) {
        let resp = ui.add(egui::Label::new(name_text).sense(egui::Sense::click())).on_hover_cursor(egui::CursorIcon::PointingHand);
        egui::Popup::menu(&resp).show(|ui| link_menu(app, ui, id, linked));
        crate::chrome::group_separator(ui);
    }
    let mut fit = crate::chrome::Fit::new(reserve);
    let details = egui::RichText::new(details).size(12.0).color(t.text_dim);
    if fit.room(ui, &[widgets::text_width(ui, details.clone())]) {
        ui.label(details);
    }
    fit.separator(ui);
    if app.session.tool_id() == vectorcraft_tools::cropimage::ID {
        return true;
    }
    link_buttons(app, ui, id, linked, &name, LinkButtons::Fit(&mut fit));
    // The Image Trace button grows past its usual width for a longer translated label.
    let arrow = widgets::SPLIT_ARROW;
    let trace = widgets::flat_button_fit_width(ui, tl!("Image Trace"), crate::panels::image_trace::TRACE_BUTTON_W - arrow) + arrow;
    if fit.room(ui, &[trace]) {
        crate::panels::image_trace::trace_button(app, ui, trace);
    }
    for (label, cmd, w) in [
        (tl!("Mask"), "object.maskImage", 52.0),
        (tl!("Crop Image"), "ui.cropImage", 84.0),
        (tl!("Remove Background"), crate::addons::image::REMOVE_BACKGROUND, 128.0),
    ] {
        if fit.room(ui, &[widgets::flat_button_fit_width(ui, label, w)]) && widgets::flat_button_fit(ui, label, w).clicked() {
            crate::menus::invoke(app, cmd, json!({}));
        }
    }
    fit.separator(ui);
    true
}

/// How [`link_buttons`] sizes its buttons.
pub(crate) enum LinkButtons<'a> {
    /// Each button is this wide (Properties).
    Width(f32),
    /// Each button fits its label and shows only while the Control bar's row has room for it.
    Fit(&'a mut crate::chrome::Fit),
}

/// The least widths of the Embed, Edit Original and Unembed… buttons fitted to their labels.
const EMBED_WIDTH: f32 = 60.0;
const EDIT_ORIGINAL_WIDTH: f32 = 92.0;
const UNEMBED_WIDTH: f32 = 80.0;

/// The menu of the Control bar's file name for image `id`: Replace… (another file takes the
/// image's place with its own aspect ratio; an embedded image becomes linked), Go To Link, Edit
/// Original, Update Link and Placement Options….
fn link_menu(app: &mut VectorcraftApp, ui: &mut egui::Ui, id: u64, linked: bool) {
    ui.set_min_width(200.0);
    if widgets::menu_item(ui, tl!("Replace…"), app.services.pick_open.is_some(), false) {
        crate::panels::links::replace(app, vec![id]);
    }
    if widgets::menu_item(ui, tl!("Go To Link"), true, false) {
        crate::panels::links::go_to(app, id);
    }
    if widgets::menu_item(ui, tl!("Edit Original"), linked, false) {
        crate::menus::invoke(app, "links.editOriginal", json!({ "id": id }));
    }
    if widgets::menu_item(ui, tl!("Update Link"), linked, false) {
        let msg = match app.run("links.update", json!({ "ids": [id] })) {
            Ok(v) => crate::panels::links::updated_status(v["updated"].as_array().map_or(0, Vec::len)),
            Err(e) => e,
        };
        app.status(msg);
    }
    if widgets::menu_item(ui, tl!("Placement Options…"), true, false) {
        crate::menus::invoke(app, "ui.placementOptionsDialog", json!({ "ids": [id] }));
    }
}

/// Embed (and Edit Original) for linked image `id`, Unembed… for an embedded one named `name`
/// (Control bar, Properties), sized as `size` says.
pub(crate) fn link_buttons(app: &mut VectorcraftApp, ui: &mut egui::Ui, id: u64, linked: bool, name: &str, mut size: LinkButtons) {
    let mut button = |ui: &mut egui::Ui, label: &str, least: f32| match &mut size {
        LinkButtons::Width(w) => widgets::flat_button(ui, label, *w).clicked(),
        LinkButtons::Fit(fit) => {
            fit.room(ui, &[widgets::flat_button_fit_width(ui, label, least)]) && widgets::flat_button_fit(ui, label, least).clicked()
        }
    };
    if linked {
        if button(ui, tl!("Embed"), EMBED_WIDTH) {
            app.run("links.embed", json!({ "ids": [id] })).ok();
        }
        if button(ui, tl!("Edit Original"), EDIT_ORIGINAL_WIDTH) {
            crate::menus::invoke(app, "links.editOriginal", json!({ "id": id }));
        }
    } else if button(ui, tl!("Unembed…"), UNEMBED_WIDTH) {
        crate::panels::links::unembed(app, id, name);
    }
}
