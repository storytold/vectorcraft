//! File → Place in the app: picking the files, the Place dialog's options, files dropped on the
//! window, the loaded place cursor and the Control bar's image details. The engine places
//! (`file.place`, `file.place.queue`); this gathers the files and options.

use std::cell::RefCell;
use std::sync::{Arc, Mutex};

use craft::behavior::CanvasView;
use craft::behavior::{Input, Metrics, Sense};
use craft::host::{PickRequest, Picked};
use craft::ui::buttons::{Button, Link};
use craft::ui::contract::DropOperation;
use craft::ui::contract::{Common, Component, Cx, MeasureCx, Pointer as _, Props, Role, Show as _, ShowContainer as _, Trigger, clicked};
use craft::ui::display::{Badge, Picture, PictureSize, Shade, Text};
use craft::ui::layout_components::Row;
use craft::ui::list::{ThumbAspect, ThumbSize};
use craft::ui::menu::{Menu, MenuItem};
use craft::ui::popover::WholeWindow;
use craft::ui::primitives::textures::Revision;
use craft::ui::runtime::drag::{DropMark, DropTarget as FilesTarget, OsFile, OsFiles, WindowDropTarget};
use craft::ui::style::{ComponentStyle, StyleCx};
use egui::{Response, Vec2};
use serde_json::{Value, json};
use vectorcraft_doc::NodeKind;
use vectorcraft_engine::cmd::fileio;
use vectorcraft_geom::Point;

use crate::file_asks::Owner;
use crate::{VectorcraftApp, io};

/// The loaded place cursor's thumbnail size (px on the longer side).
const THUMB: u32 = 64;

/// A file dropped on a canvas that arrived asynchronously (web, where the bytes are read after the
/// drop).
pub struct PlaceArrival {
    pub name: String,
    pub bytes: Vec<u8>,
    pub drop: DropAt,
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

/// Where a dropped file landed, as far as the platform says.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DropSpot {
    /// On the canvas, at this document point.
    Canvas(Point),
    /// Off the canvas: a panel, the tab bar, a dialog.
    Elsewhere,
    /// The platform gives no place (floating panel windows, desktop drags without a position).
    Unknown,
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

/// Normal Place import options wait for their opening drawing. Canvas drops already name their destination.
pub(crate) fn import_options_drawing(app: &VectorcraftApp, params: &Value) -> Option<u64> {
    app.session.active().filter(|_| params.get(DROPPED).is_none()).map(|st| st.uid)
}

pub(crate) fn check_import_options_drawing(app: &VectorcraftApp, drawing: Option<u64>) -> Result<(), String> {
    if drawing.is_some_and(|drawing| app.session.active().is_none_or(|st| st.uid != drawing)) {
        return Err(tl!("Return to the original drawing to place the file").into());
    }
    Ok(())
}

/// The `file.place` param that holds the import options a Shift-click of the place cursor set
/// aside ([`set_aside_import_options`]): the file is placed with them when no import options dialog
/// opens for it. Removed before any dialog opens.
const SET_ASIDE: &str = "__placeCursorOptions";

/// A place cursor thumbnail, decoded when the cursor is loaded.
struct Thumb {
    image: std::sync::Arc<egui::ColorImage>,
}

/// The Place dialog's options, as OK answers them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlaceOptions {
    /// Keep a link to the file.
    pub link: bool,
    /// Place onto a template layer.
    pub template: bool,
    /// Replace the selected object.
    pub replace: bool,
    /// Ask for each file's import options.
    pub show_import_options: bool,
}

impl PlaceOptions {
    /// The option the dialog field `key` names, for the dialog's rows and its `ui.dialog.set`.
    pub(crate) fn flag(&mut self, key: &str) -> Option<&mut bool> {
        match key {
            "link" => Some(&mut self.link),
            "template" => Some(&mut self.template),
            "replace" => Some(&mut self.replace),
            "showImportOptions" => Some(&mut self.show_import_options),
            _ => None,
        }
    }
}

impl PlaceState {
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
    let mut set_aside = None;
    if let Some(o) = p.as_object_mut() {
        o.remove("showImportOptions");
        set_aside = o.remove(SET_ASIDE);
    }
    // A file placed with an importer plug-in `importer` names goes to that importer, without the
    // import options of a built-in format that reads its extension.
    let builtin = p.get("importer").is_none_or(Value::is_null);
    if !ask && builtin {
        // Placed with the options confirmed last for the format.
        if p.get("text").is_none() && crate::dialogs::text_import::is_text(&p) {
            p["text"] = crate::dialogs::text_import::last_options(app);
        }
        if let Some(dxf) = crate::dialogs::dxf_import::last_options(app, &p) {
            p["dxf"] = dxf;
        }
        crate::dialogs::psd_import::last_options(app, &mut p);
    }
    if builtin && p.get("text").is_none() && crate::dialogs::text_import::is_text(&p) {
        return Ok(crate::dialogs::text_import::open(app, &p));
    }
    if p.get("path").is_none() && p.get("dataBase64").is_none() {
        let files: Vec<Value> = match p.get("paths").and_then(Value::as_array) {
            Some(a) => a.iter().filter_map(Value::as_str).map(|path| with_importer(json!({ "path": path }), &p)).collect(),
            None => return Ok(pick(app, &p)),
        };
        if files.is_empty() {
            return Err("paths: give at least one file".into());
        }
        open_dialog(app, files, show);
        return Ok(Value::Null);
    }
    // A PDF with several pages or a password asks for the page, box and password first; with
    // Show Import Options off, or with a `page` given, only an encrypted one asks, for its password.
    let offered = builtin && ((ask && crate::dialogs::import_pdf::offer_place(app, &p)) || crate::dialogs::import_pdf::offer_place_password(app, &p));
    if offered {
        return Ok(Value::Null);
    }
    // A DXF drawing asks for its import options.
    if builtin && ask && crate::dialogs::dxf_import::offer_place(app, &p) {
        return Ok(json!({ "dialog": crate::dialogs::dxf_import::KIND }));
    }
    // A PSD file with several layers or with slices that the user embeds asks for its import
    // options. A place run as a command (MCP, scripts) places at once and takes `layers` and
    // `importSlices` itself.
    if builtin && ask && matches!(read, Read::ForUser { .. }) && crate::dialogs::psd_import::offer_place(app, &p) {
        return Ok(json!({ "dialog": crate::dialogs::psd_import::KIND }));
    }
    if let (Some(Value::Object(kept)), Some(o)) = (set_aside, p.as_object_mut()) {
        for (k, v) in kept {
            o.entry(k).or_insert(v);
        }
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
    // An embedded PDF without `textAs` takes the `pdfTextAs` preference, placed by the user or run
    // as a command.
    crate::dialogs::import_pdf::place_with_preferred_text_as(app, &mut p);
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

/// Pick files to place: the Place dialog opens with them (on the web once they arrive), placed with
/// the importer plug-in and parameters `p` names ([`with_importer`]). Returns what
/// [`crate::file_asks::ask`] returns.
fn pick(app: &mut VectorcraftApp, p: &Value) -> Value {
    let request =
        fileio::place_dialog_filters().fold(PickRequest::open_many("place"), |request, (name, extensions)| request.filter(name, extensions));
    let with = with_importer(json!({}), p);
    // The web answers with the files' contents instead of paths.
    crate::file_asks::ask(app, request, Owner::Document, move |app, picked| {
        let files = match picked {
            Picked::Paths(paths) => paths.into_iter().map(|path| with_importer(json!({ "path": path.to_string_lossy() }), &with)).collect(),
            Picked::Contents(files) => files
                .into_iter()
                .map(|(name, bytes)| {
                    app.place.picked.retain(|(n, _)| *n != name);
                    let entry = with_importer(json!({ "name": name }), &with);
                    app.place.picked.push((name, bytes));
                    entry
                })
                .collect(),
            // Only an open's answers carry files.
            _ => return Ok(Value::Null),
        };
        open_dialog(app, files, None);
        Ok(Value::Null)
    })
}

/// The params of `file.place` that name an importer plug-in and its parameters: a Place dialog file
/// entry carries them, and its file is read and placed with them.
const IMPORTER_KEYS: [&str; 2] = ["importer", "params"];

/// Place dialog file entry `file` with the importer plug-in and parameters `p` names
/// ([`IMPORTER_KEYS`]).
fn with_importer(mut file: Value, p: &Value) -> Value {
    for k in IMPORTER_KEYS {
        if let Some(v) = p.get(k).filter(|v| !v.is_null()) {
            file[k] = v.clone();
        }
    }
    file
}

/// The Place dialog for `files` (`{path}`, or `{name}` with the bytes in [`PlaceState`]), with Show
/// Import Options set to `show` when given, else as last chosen.
fn open_dialog(app: &mut VectorcraftApp, files: Vec<Value>, show: Option<bool>) {
    let Some(dialogs) = app.dialogs() else { return };
    let units = app.session.active().map(|d| d.doc.units).unwrap_or_default();
    let info = lines(app, &files, units);
    let one_object = app.session.active().is_some_and(|d| d.selection.objects.len() == 1);
    let linked: Vec<bool> = files.iter().map(|f| is_vectorcraft_file(app, f)).collect();
    let options =
        PlaceOptions { link: app.ui.place_link, template: false, replace: false, show_import_options: show.unwrap_or(app.ui.place_import_options) };
    let replace_enabled = files.len() == 1 && one_object;
    let dialog = crate::dialogs::place::PlaceDialog::new(files, info, options, replace_enabled, linked.contains(&true), linked.contains(&false));
    dialogs.with(|host| host.open(dialog));
}

/// The Place dialog's per-frame work: its lines once they are read off the UI thread, and its OK.
/// An error from placing shows in the status bar, as the dialog is closed by then.
// PLAN: the sheet puts `fill_lines` in the dialog's own show; the dialog has no `app`, so the pump fills it.
pub(crate) fn pump(app: &mut VectorcraftApp, ctx: &egui::Context) {
    use crate::dialogs::place::PlaceDialog;
    use craft::ui::dialog::DialogResult;
    let Some(dialogs) = app.dialogs() else { return };
    let (reading, answer) = dialogs.with(|host| {
        let reading = host.get_mut::<PlaceDialog>().is_some_and(|d| fill_lines(app, &d.files, &mut d.info));
        (reading, host.take::<PlaceDialog>())
    });
    if reading {
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
    }
    match answer {
        Some(DialogResult::Ok((files, options))) => {
            if let Err(e) = confirm(app, files, options) {
                app.status(e);
            }
        }
        Some(_) => app.place.reading_lines = None,
        None => {}
    }
}

/// Is dialog file entry `file` a VectorCraft document on disk (what Link places as one locked
/// object), or a PDF, `.ai`, EPS or PostScript file on disk? A DCS file (an EPS file naming its
/// plates) is placed as a linked image instead. Where the platform reads part of a file, only the
/// file's start is read; a file longer than that is taken to be at least as long as its binary EPS
/// header says, and one whose header can't be told from it counts as a document. A file that names
/// an importer plug-in is none of these, and none of it is read here.
fn is_vectorcraft_file(app: &VectorcraftApp, file: &Value) -> bool {
    use vectorcraft_engine::cmd::place::{DCS_HEAD_BYTES, head_is_dcs};
    if file.get("importer").is_some() {
        return false;
    }
    let Some(path) = file.get("path").and_then(Value::as_str) else { return false };
    let native = ["vectorcraft", vectorcraft_format::LEGACY_EXTENSION, "vctemplate"];
    // The file's start, and the file's length when that is all of it.
    let bytes = || match (&app.services.read_head, &app.services.read) {
        (Some(read_head), _) => read_head(path, DCS_HEAD_BYTES + 1).ok().map(|mut b| {
            let whole = b.len() <= DCS_HEAD_BYTES;
            b.truncate(DCS_HEAD_BYTES);
            let size = if whole { b.len() as u64 } else { u64::MAX };
            (b, size)
        }),
        (None, Some(read)) => read(path).ok().map(|b| {
            let size = b.len() as u64;
            (b, size)
        }),
        (None, None) => None,
    };
    let dcs = |(b, size): &(Vec<u8>, u64)| {
        let head = b.get(..DCS_HEAD_BYTES).unwrap_or(b);
        vectorcraft_pdf::is_postscript(head) && head_is_dcs(head, *size).unwrap_or(false)
    };
    match fileio::extension(path).as_str() {
        e if native.contains(&e) => true,
        "pdf" | "ai" | "ait" => bytes().is_none_or(|b| !dcs(&b)),
        "eps" | "epsf" | "ps" => bytes().is_some_and(|b| vectorcraft_pdf::is_postscript(&b.0) && !dcs(&b)),
        _ => false,
    }
}

/// The engine params naming `file` (a dialog file entry), with the importer plug-in and parameters
/// it carries ([`IMPORTER_KEYS`]).
fn source(app: &VectorcraftApp, file: &Value) -> Option<Value> {
    if let Some(path) = file.get("path").and_then(Value::as_str) {
        return Some(with_importer(json!({ "path": path }), file));
    }
    let name = file.get("name").and_then(Value::as_str)?;
    let bytes = app.place.picked.iter().find(|(n, _)| n == name).map(|(_, b)| b)?;
    Some(with_importer(json!({ "name": name, "dataBase64": vectorcraft_format::base64_encode(bytes) }), file))
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

/// The open Place dialog's lines `info` about its `files` once they have been read off the UI
/// thread ([`lines`]) → whether they are still being read.
pub(crate) fn fill_lines(app: &mut VectorcraftApp, files: &[Value], info: &mut Vec<String>) -> bool {
    let Some((read_files, read)) = &app.place.reading_lines else { return false };
    if *read_files != json!(files) {
        return false;
    }
    match read.try_recv() {
        Ok(lines) => {
            *info = lines;
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
            let ppi = crate::i18n::fmt(tl!("{n} ppi"), &[("n", &fmt_ppi(ppi))]);
            format!("{w} × {h} px, {ppi}, {} ({size})", i["colorMode"].as_str().unwrap_or("RGB"))
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
pub fn confirm(app: &mut VectorcraftApp, files: Vec<Value>, options: PlaceOptions) -> Result<Value, String> {
    let sources: Vec<Value> = files.iter().filter_map(|f| source(app, f)).collect();
    if sources.is_empty() {
        return Err("no files to place".into());
    }
    let PlaceOptions { link, template, replace, show_import_options: ask } = options;
    app.ui.place_link = link;
    app.ui.place_import_options = ask;
    app.place.reading_lines = None;
    let r = match &sources[..] {
        [one] => {
            let mut p = one.clone();
            p["link"] = json!(link);
            p["template"] = json!(template);
            p["showImportOptions"] = json!(ask);
            if replace {
                p["replace"] = json!(true);
            }
            run_for_user(app, &p)
        }
        many => {
            let paths: Vec<&Value> = many.iter().filter_map(|s| s.get("path")).collect();
            let files: Vec<&Value> = many.iter().filter(|s| s.get("path").is_none()).collect();
            // The files of one dialog come from one `file.place`, with one importer plug-in and its
            // parameters, if any.
            let mut q = with_importer(json!({ "paths": paths, "files": files, "link": link, "template": template }), &many[0]);
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
/// asks, for its password. A Shift-click (`showImportOptions: true`) asks for them however the
/// cursor was loaded. An error shows in the status bar.
pub(crate) fn place_from_cursor(app: &mut VectorcraftApp, p: &Value) {
    // Read within the automation roots of whoever loaded the cursor, even on a click of the person
    // at the keyboard: an agent's queue reaches no file it wasn't given.
    let roots = app.session.place_queue_roots();
    forget_older_queue(app);
    vectorcraft_engine::file_access::confine(roots.as_ref(), || {
        let locked = is_locked(app, p);
        // A Shift-click asks for the file's import options whatever the cursor was loaded with.
        let shift_click = p.get("showImportOptions") == Some(&Value::Bool(true));
        if app.place.queue_asks || locked || shift_click {
            let mut p = clicked_at_full_size(app, p);
            if shift_click {
                set_aside_import_options(&mut p);
            } else if !app.place.queue_asks {
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
        } else {
            // An embedded PDF takes the `pdfTextAs` preference, as one placed from the Place dialog.
            let mut p = p.clone();
            crate::dialogs::import_pdf::place_with_preferred_text_as(app, &mut p);
            if crate::placing::available(app) {
                if let Err(e) = crate::placing::start(app, &p, false) {
                    app.status(e);
                }
            } else {
                let _ = engine(app, "file.place", &p);
            }
        }
    });
}

/// Move the import options the place cursor's file `p` was loaded with for its format (`text`,
/// `dxf`, a PDF's `page` and `crop`, a PSD file's `layers`, `hiddenLayers` and `importSlices`) to
/// [`SET_ASIDE`], so that a Shift-click opens its import options dialog; a file whose dialog does
/// not open is placed with them.
fn set_aside_import_options(p: &mut Value) {
    let ext = p.get("path").or_else(|| p.get("name")).and_then(Value::as_str).map(fileio::extension).unwrap_or_default();
    let keys: &[&str] = match ext.as_str() {
        "pdf" | "ai" | "ait" => &["page", "crop"],
        "dxf" => &["dxf"],
        "psd" | "pdd" | "psb" => &["layers", "hiddenLayers", "importSlices"],
        _ if crate::dialogs::text_import::is_text(p) => &["text"],
        _ => &[],
    };
    let Some(o) = p.as_object_mut() else { return };
    let kept: serde_json::Map<String, Value> = keys.iter().filter_map(|k| Some((k.to_string(), o.remove(*k)?))).collect();
    if !kept.is_empty() {
        o.insert(SET_ASIDE.into(), Value::Object(kept));
    }
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
            Some((f["name"].as_str()?.to_string(), Thumb { image: std::sync::Arc::new(color) }))
        })
        .collect();
}

/// The loaded place cursor at `at` in the overlay layer `ui`: the current file's thumbnail with the
/// number of files loaded. A thumbnail is named by its place in the cursor's list, which
/// [`show_thumbnails`] replaces only together with a new load number.
pub fn cursor_overlay(app: &mut VectorcraftApp, ui: &mut egui::Ui, at: egui::Pos2) {
    let opts = app.session.tool_options();
    let count = opts["count"].as_u64().unwrap_or(0);
    let name = opts["name"].as_str().unwrap_or_default();
    let found = app.place.thumbs.iter().enumerate().find(|(_, (n, _))| n == name);
    let revision = Revision::new(app.place.queue_load);
    let room = egui::Rect::from_min_max(at, ui.max_rect().max.max(at));
    ui.scope_builder(egui::UiBuilder::new().max_rect(room), |ui| {
        match found {
            Some((key, (_, thumb))) => {
                let side = |n: usize| u16::try_from(n.max(1)).unwrap_or(u16::MAX);
                let [w, h] = thumb.image.size;
                Picture::new(name.to_string(), key as u64, &|| (*thumb.image).clone())
                    .revision(revision)
                    .aspect(ThumbAspect::new(side(w), side(h)))
                    .extent(PictureSize::Thumb(ThumbSize::Medium))
                    .show(ui);
            }
            None => {
                Text::new(name.to_string()).shade(Shade::Muted).show(ui);
            }
        }
        if count > 1 {
            Badge::count(u32::try_from(count).unwrap_or(u32::MAX)).show(ui);
        }
    });
}

// ---------- drops ----------

impl VectorcraftApp {
    /// Where file `name` dropped at `spot` goes: onto the canvas of the open document (Shift
    /// embeds it), else opened. Where the platform doesn't say ([`DropSpot::Unknown`]), a document
    /// opens as a tab of its own and a picture or text is placed in the middle of the view. A
    /// library, a presets file, a plug-in, a shortcut set, a script or a file only an importer
    /// plug-in reads opens wherever it is dropped, as File → Open opens it ([`io::opens_as`],
    /// [`fileio::opens_with_importer`]).
    pub fn drop_target(&self, name: &str, spot: DropSpot, shift: bool) -> DropTarget {
        let (Some(st), Some(v)) = (self.session.active(), self.view()) else { return DropTarget::Open };
        let at = match spot {
            _ if io::opens_as(name).is_some() || fileio::opens_with_importer(name) => return DropTarget::Open,
            DropSpot::Elsewhere => return DropTarget::Open,
            DropSpot::Canvas(at) => at,
            DropSpot::Unknown if is_document(name) => return DropTarget::Open,
            DropSpot::Unknown => v.center,
        };
        DropTarget::Place(DropAt { doc: st.uid, reverts: st.reverts, at, embed: shift })
    }

    /// Where a drop at `pos` on the window lands, for a host that follows its file drags itself (the
    /// web's): on the active canvas as it last showed, else elsewhere; without a position, unknown.
    pub fn drop_spot(&self, pos: Option<egui::Pos2>) -> DropSpot {
        let Some(pos) = pos else { return DropSpot::Unknown };
        match self.canvas_view() {
            Some(view) if view.rect.contains(pos) => {
                let at = view.to_doc(pos);
                DropSpot::Canvas(Point::new(at.x, at.y))
            }
            _ => DropSpot::Elsewhere,
        }
    }
}

/// Files held over, or dropped on, a canvas that would be placed there.
pub struct CanvasDrop {
    files: Vec<OsFile>,
    at: Point,
}

/// The canvas as a target of files dragged from the OS: it takes those [`VectorcraftApp::drop_target`]
/// places, and outlines the canvas while they hover. On the web the browser gives a hovered file's
/// type but not its name, so any file is taken then, including libraries, presets, plug-ins and
/// shortcut sets, which open when dropped.
pub struct CanvasFiles<'a> {
    app: &'a VectorcraftApp,
    rect: egui::Rect,
    view: &'a CanvasView,
}

impl<'a> CanvasFiles<'a> {
    /// The target over `rect`, showing the document through `view`.
    pub fn new(app: &'a VectorcraftApp, rect: egui::Rect, view: &'a CanvasView) -> Self {
        CanvasFiles { app, rect, view }
    }
}

impl FilesTarget<OsFiles> for CanvasFiles<'_> {
    type Drop = CanvasDrop;

    fn accept(&self, payload: &OsFiles, at: egui::Pos2, _op: DropOperation) -> Option<CanvasDrop> {
        let doc = self.view.to_doc(at);
        let at = Point::new(doc.x, doc.y);
        let files: Vec<OsFile> = payload
            .files
            .iter()
            .filter(|f| f.name.is_empty() || matches!(self.app.drop_target(&f.name, DropSpot::Canvas(at), false), DropTarget::Place(_)))
            .cloned()
            .collect();
        (!files.is_empty()).then_some(CanvasDrop { files, at })
    }

    fn mark(&self, _: &CanvasDrop) -> DropMark {
        DropMark::region(self.rect)
    }

    // Files dropped under an alert are placed too, as production placed them.
    fn under_modal(&self) -> bool {
        true
    }
}

/// Files dropped on the canvas ([`CanvasFiles`]), placed where they landed; Shift embeds them.
// PLAN: `DropOperation` has no Shift, so the held modifiers of the drop's frame are read here.
pub fn canvas_drop(app: &mut VectorcraftApp, ctx: &egui::Context, dropped: CanvasDrop) {
    let shift = ctx.input(|i| i.modifiers.shift);
    os_drop(app, dropped.files, DropSpot::Canvas(dropped.at), shift);
}

/// Files the OS dropped at `spot`, each placed or opened as [`VectorcraftApp::drop_target`] says for it, Shift held
/// when `shift`.
pub(crate) fn os_drop(app: &mut VectorcraftApp, files: Vec<OsFile>, spot: DropSpot, shift: bool) {
    let files = files
        .into_iter()
        .filter_map(|f| {
            let path = f.path.map(|p| p.to_string_lossy().into_owned());
            // A file the platform gave neither a path nor bytes for (the web reads them after the drop) has nothing to
            // open or place.
            let bytes = match (&path, f.bytes) {
                (None, Some(bytes)) => bytes.to_vec(),
                (None, None) => return None,
                (Some(_), _) => vec![],
            };
            // Bytes with no name, as a backend may give them: their format decides, so an SVG opens as a document and
            // a raster image is placed.
            if f.name.is_empty() {
                let name = "dropped".to_owned();
                let target = if fileio::detect(&name, &bytes).is_none_or(|format| !format.raster) {
                    DropTarget::Open
                } else {
                    app.drop_target(&name, spot, shift)
                };
                return Some((target, (name, path, bytes)));
            }
            Some((app.drop_target(&f.name, spot, shift), (f.name, path, bytes)))
        })
        .collect();
    drop_files(app, files);
}

/// The style of the window's file catcher: it paints nothing of its own.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct WindowFilesStyle {
    #[style(metrics)]
    metrics: Metrics,
}

/// The catcher of the files the OS drops on the app's window where no canvas takes them.
#[derive(Component, Props)]
pub(crate) struct WindowFiles<'a> {
    #[common]
    common: Common,
    out: &'a RefCell<Option<OsFiles>>,
}

impl Component for WindowFiles<'_> {
    const ROLE: Role = Role::None;
    const PARTS: bool = false;
    const STOPS: bool = false;
    const INPUT: Input = Input::Display;
    const SHOWS_LABEL: bool = false;
    const OVERLAYS: bool = false;
    type Style = WindowFilesStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: WindowFilesStyleKey) -> WindowFilesStyle {
        WindowFilesStyle { metrics: Metrics::new() }
    }

    fn measure(&mut self, _: &MeasureCx<'_, Self>) -> Vec2 {
        Vec2::ZERO
    }

    fn sense(&self) -> Sense {
        Sense::empty()
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        *self.out.borrow_mut() = cx.window_drop_target::<OsFiles, _>(&UnderAlerts(WholeWindow::new(cx.rect)));
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

/// The whole window as the target of files dropped off every canvas, under an alert too, as production opened them.
struct UnderAlerts(WholeWindow);

impl WindowDropTarget<OsFiles> for UnderAlerts {
    type Drop = OsFiles;

    fn accept(&self, payload: &OsFiles, op: DropOperation) -> Option<OsFiles> {
        self.0.accept(payload, op)
    }

    fn mark(&self, drop: &OsFiles) -> DropMark {
        self.0.mark(drop)
    }

    fn under_modal(&self) -> bool {
        true
    }
}

/// Opens the files dropped on the window off every canvas, as a drop on a panel or the tab bar does
/// ([`DropSpot::Elsewhere`]); a canvas's own target takes the files it places.
pub(crate) fn window_files(app: &mut VectorcraftApp, ui: &mut egui::Ui) {
    let out = RefCell::new(None);
    WindowFiles { common: Common::new(), out: &out }.id_salt("window-files").show_at(ui, ui.max_rect());
    if let Some(dropped) = out.take() {
        let shift = ui.ctx().input(|i| i.modifiers.shift);
        os_drop(app, dropped.files, DropSpot::Elsewhere, shift);
    }
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
                _ => io::open_bytes(app, &name, &bytes, path.clone()).map(|r| {
                    // A document that asks a question first goes on the recent files once it opens.
                    if let Some(path) = path.as_ref().filter(|_| !io::asks_first(&r)) {
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

/// Files that arrived to be placed on a canvas (web, where the bytes are read after the drop).
pub fn drain(app: &mut VectorcraftApp) {
    let arrived: Vec<PlaceArrival> =
        app.services.place_inbox.as_ref().map(|q| std::mem::take(&mut *q.lock().unwrap_or_else(|e| e.into_inner()))).unwrap_or_default();
    for a in arrived {
        drop_files(app, vec![(DropTarget::Place(a.drop), (a.name, None, a.bytes))]);
    }
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

/// The link actions of an image: what each surface that shows them (the Control bar, Properties)
/// lists, so the labels and effects are in one place.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LinkAction {
    /// Embed a linked image.
    Embed,
    /// Open a linked image's file in its editor.
    EditOriginal,
    /// Unembed an embedded image into a file.
    Unembed,
}

impl LinkAction {
    /// The actions that apply to an image, `linked` or embedded.
    pub(crate) fn shown(linked: bool) -> &'static [LinkAction] {
        if linked { &[LinkAction::Embed, LinkAction::EditOriginal] } else { &[LinkAction::Unembed] }
    }

    /// The action's label in the interface language.
    pub(crate) fn label(self) -> &'static str {
        match self {
            LinkAction::Embed => tl!("Embed"),
            LinkAction::EditOriginal => tl!("Edit Original"),
            LinkAction::Unembed => tl!("Unembed…"),
        }
    }

    /// Runs the action for image `id`, named `name`.
    pub(crate) fn run(self, app: &mut VectorcraftApp, id: u64, name: &str) {
        match self {
            LinkAction::Embed => {
                app.run("links.embed", json!({ "ids": [id] })).ok();
            }
            LinkAction::EditOriginal => crate::menus::invoke(app, "links.editOriginal", json!({ "id": id })),
            LinkAction::Unembed => crate::panels::links::unembed(app, id, name),
        }
    }
}

/// The Control bar for the one selected image: its file name, colour mode and effective
/// resolution, Embed or Unembed, Edit Original (a linked file), Image Trace, Mask, Crop Image and
/// Remove Background, each a ranked row that leaves a shrinking bar whole. While the Crop Image
/// tool is in use, the bar shows the file name, colour mode and resolution only, and the tool's
/// cropping options follow them. Whether one image is selected.
pub fn control_bar_details(app: &mut VectorcraftApp, ui: &mut egui::Ui) -> bool {
    use crate::chrome::rank;
    let Some(i) = selected_image_info(app) else { return false };
    let (name, details) = image_summary(i);
    let (Some(id), linked) = (i["id"].as_u64(), i["linked"] == true) else { return false };
    // The file name is a link: its menu relinks (replaces) the image and reaches the Links
    // panel's other actions for it.
    Row::new().fit_rank(rank::GROUP).show(ui, |ui, _| {
        let link = Link::new(name.clone()).show(ui);
        Menu::new(name.clone()).opened_by(&link, Trigger::Press).show(ui, |ui, _| {
            if MenuItem::new(tl!("Replace…").to_string()).enabled(crate::file_asks::can(app, craft::host::PickKind::Open)).show(ui).ran {
                crate::panels::links::replace(app, vec![id]);
            }
            if MenuItem::new(tl!("Go To Link").to_string()).show(ui).ran {
                crate::panels::links::go_to(app, id);
            }
            if MenuItem::new(LinkAction::EditOriginal.label().to_string()).enabled(linked).show(ui).ran {
                LinkAction::EditOriginal.run(app, id, &name);
            }
            if MenuItem::new(tl!("Update Link").to_string()).enabled(linked).show(ui).ran {
                let msg = match app.run("links.update", json!({ "ids": [id] })) {
                    Ok(v) => crate::panels::links::updated_status(v["updated"].as_array().map_or(0, Vec::len)),
                    Err(e) => e,
                };
                app.status(msg);
            }
            if MenuItem::new(tl!("Placement Options…").to_string()).show(ui).ran {
                crate::menus::invoke(app, "ui.placementOptionsDialog", json!({ "ids": [id] }));
            }
        });
    });
    Row::new().fit_rank(rank::GROUP).show(ui, |ui, _| {
        Text::new(details.clone()).shade(Shade::Muted).show(ui);
    });
    if app.session.tool_id() == vectorcraft_tools::cropimage::ID {
        return true;
    }
    for action in LinkAction::shown(linked) {
        let at = if *action == LinkAction::EditOriginal { rank::DETAIL } else { rank::GROUP };
        Row::new().fit_rank(at).show(ui, |ui, _| {
            if clicked(&Button::new(action.label().to_string()).show(ui)) {
                action.run(app, id, &name);
            }
        });
    }
    Row::new().fit_rank(rank::GROUP).show(ui, |ui, _| crate::panels::image_trace::trace_button(app, ui));
    for (label, cmd) in
        [(tl!("Mask"), "object.maskImage"), (tl!("Crop Image"), "ui.cropImage"), (tl!("Remove Background"), crate::addons::image::REMOVE_BACKGROUND)]
    {
        Row::new().fit_rank(rank::GROUP).show(ui, |ui, _| {
            if clicked(&Button::new(label.to_string()).show(ui)) {
                crate::menus::invoke(app, cmd, json!({}));
            }
        });
    }
    true
}
