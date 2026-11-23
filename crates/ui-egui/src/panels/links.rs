//! Links panel: every image in the document, linked files with their status (missing, modified)
//! and embedded images, as rows with a thumbnail, name, page and status. Clicking a row selects
//! its image, Ctrl/Cmd-click adds or removes it, and Shift-click selects the images of the rows
//! from the last row clicked without Shift to it (with Ctrl/Cmd, adds them); the footer's buttons
//! Relink, Go To, Update and Edit Original act on the selected rows (Relink through its dialog,
//! with the Replace choice: `ui.relinkDialog`); Link Info shows the details of the first. The
//! panel menu adds Relink to Folder, Find in Folder (Find Linked Files: `ui.findLinksInFolder`),
//! Embed / Unembed, Placement Options, Select Linked / Embedded / Missing / Modified
//! (`addon.select.byLinkStatus`, Shift adds), Show filters, Sort, Copy Info (the selected rows'
//! file names to the clipboard: `links.copyInfo`), thumbnail size, Show in Folder and Show DCS
//! Transparency Interactions (a warning on the rows of images placed from DCS files that are
//! transparent or overlap transparent objects).
//!
//! Images and placed documents in opacity-mask art and in the art of symbols, graph designs and
//! patterns are listed after the layers' own, one row each; the canvas can't select them, so a
//! click picks such a row in the panel, and the footer's buttons act on it as on any other.
//!
//! Everything runs through the `links.*` commands (`links.editOriginal` and `links.reveal` are UI
//! commands: they hand the file to the system) and the panel's own `ui.links.*` commands
//! ([`run`]).

use std::collections::HashMap;
use std::sync::Arc;

use craft::behavior::{IconId, Selection, keyed};
use craft::host::{PickKind, PickRequest};
use craft::keymap::CommandRef;
use craft::ui::contract::{Bind, CollectionEvent, Props as _, Show as _, ShowCollection as _, ShowContainer as _};
use craft::ui::display::{Shade, Text};
use craft::ui::form::Field;
use craft::ui::list::{List, ListRow, ThumbSize};
use craft::ui::menu::{Menu, MenuItem, PanelRow};
use craft::ui::runtime::commands::{CommandState, CommandStates};
use craft::ui::sections::{FooterAction, FooterBar, Panel, Section};
use egui::Ui;
use serde_json::{Value, json};
use vectorcraft_doc::NodeId;

use crate::VectorcraftApp;
use crate::addons::select::link_status;
use crate::file_asks::{self, Owner};

pub const ID: &str = "links";
/// Seconds between two looks at the linked files (they change outside the app).
const RECHECK: f64 = 2.0;
const SHOW: [(&str, &str); 4] = [("all", "Show All"), ("missing", "Show Missing"), ("modified", "Show Modified"), ("embedded", "Show Embedded")];
const SORT: [(&str, &str); 4] = [("name", "Sort by Name"), ("kind", "Sort by Kind"), ("status", "Sort by Status"), ("size", "Sort by Size")];
const THUMBS: [(&str, ThumbSize, &str); 4] = [
    ("none", ThumbSize::None, "No Thumbnails"),
    ("small", ThumbSize::Small, "Small Thumbnails"),
    ("medium", ThumbSize::Medium, "Medium Thumbnails"),
    ("large", ThumbSize::Large, "Large Thumbnails"),
];
/// The footer's actions after Show Link Info, in the order the footer lays them out from its end edge.
const FOOTER: [(Act, &str, &str); 5] = [
    (Act::Edit, "pencil", "Edit Original"),
    (Act::Update, "rotate-cw", "Update Link"),
    (Act::GoTo, "zoom-in", "Go To Link"),
    (Act::Relink, "link", "Relink…"),
    (Act::BreakLink, "link-2-off", "Break Link"),
];

/// The panel's options (panel menu).
#[derive(Clone, Debug)]
pub(super) struct Options {
    pub(super) show: &'static str,
    pub(super) sort: Option<&'static str>,
    pub(super) thumb: ThumbSize,
    /// The Link Info section is open.
    pub(super) info: bool,
    /// Show DCS Transparency Interactions: rows of DCS images that are transparent or overlap
    /// transparent objects get a warning.
    pub(super) dcs: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self { show: "all", sort: None, thumb: ThumbSize::Medium, info: true, dcs: false }
    }
}

/// The rows outside the layer tree picked in the panel, with the canvas selection
/// ([`canvas_selection`]) they were picked with.
type NestedPicks = Option<((u64, Vec<u64>), Vec<u64>)>;

/// What the panel keeps for the session: its options, the rows it last fetched, the request to
/// show an image, the rows picked outside the layer tree, the list's selection (its lead and
/// anchor) and the row whose menu is open.
#[derive(Clone, Debug, Default)]
pub(crate) struct LinksModel {
    pub(super) options: Options,
    pub(super) cache: Arc<Cache>,
    /// The (document uid, image) [`show_selected_image`] asks the next draw to reveal.
    pub(super) show: Option<(u64, u64)>,
    nested: NestedPicks,
    selection: Selection<u64>,
    menu: Option<u64>,
}

/// `links.list` and the first selected image's `links.info`, with what they were fetched for.
#[derive(Clone, Debug, Default)]
pub(super) struct Cache {
    /// (document uid, revision, show, sort, Show DCS Transparency Interactions).
    key: Option<(u64, u64, &'static str, Option<&'static str>, bool)>,
    at: f64,
    list: Value,
    info: Option<(u64, Value)>,
    /// The objects outside the layer tree, by id: those of the rows, for their thumbnails, and
    /// those whose opacity masks hold a row's image, for the tooltip.
    nested: Vec<(u64, Arc<vectorcraft_doc::Node>)>,
}

impl Cache {
    /// Object `id` outside the layer tree, as [`cached`] collected it.
    pub(super) fn nested_node(&self, id: u64) -> Option<&Arc<vectorcraft_doc::Node>> {
        self.nested.binary_search_by_key(&id, |(n, _)| *n).ok().and_then(|i| self.nested.get(i)).map(|(_, n)| n)
    }
}

/// What a footer button, a row menu item or a panel menu row does to the selected rows.
#[derive(Clone, Copy, PartialEq)]
enum Act {
    Relink,
    RelinkToFolder,
    FindInFolder,
    GoTo,
    Update,
    Edit,
    Reveal,
    Embed,
    Unembed,
    BreakLink,
    Placement,
    CopyInfo,
}

/// Show the Links panel on the selected image (the Control bar's Linked File or Embedded label),
/// wherever the panel is docked or floats; a panel that is already shown stays where it is. The
/// Link Info section opens, and when the panel next draws the list, a Show filter that leaves out
/// the image's row goes back to Show All and the list scrolls to the row ([`show`]). The sort,
/// thumbnail size and DCS option keep their settings.
pub(crate) fn show_selected_image(app: &mut VectorcraftApp, _ctx: &egui::Context) {
    let first = selected_images(app).first().copied();
    let request = app.session.active().map(|st| st.uid).zip(first);
    let model = &mut app.ui.links;
    model.show = request;
    // The list drawn next is read after this request, so a row a file change has just brought
    // into the Show filter is there to scroll to.
    Arc::make_mut(&mut model.cache).at = f64::NEG_INFINITY;
    model.options.info = true;
    if let Err(e) = crate::panel_docking::reveal_panel(app, ID) {
        app.status(e);
    }
}

/// The rows to show (cached per document revision, the files looked at again every [`RECHECK`]
/// seconds) and the Link Info of the first selected image when the section is open. A frame that
/// fetches nothing returns the stored cache without copying it and stores nothing.
fn cached(app: &mut VectorcraftApp, ctx: &egui::Context) -> Option<Arc<Cache>> {
    let o = app.ui.links.options.clone();
    let st = app.session.active()?;
    let key = Some((st.uid, st.revision, o.show, o.sort, o.dcs));
    let first = selected_links(app).first().copied();
    let now = ctx.input(|i| i.time);
    let mut c = Arc::clone(&app.ui.links.cache);
    let stale = c.key != key || now - c.at > RECHECK;
    if stale {
        let same_doc = c.key == key;
        let list = app.session.execute("links.list", &json!({ "show": o.show, "sort": o.sort, "dcsTransparency": o.dcs })).ok()?;
        // One pass over the stored art for the rows outside the layer tree and the objects whose
        // opacity masks hold them, not one per frame. An object in the layer tree is looked up by
        // its index when drawn.
        let doc = app.session.active().map(|st| &st.doc);
        let want: std::collections::HashSet<u64> = list["links"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|r| nested_row(r))
            .flat_map(|r| [r["id"].as_u64(), r["owner"]["maskOf"].as_u64()])
            .flatten()
            .filter(|id| doc.is_some_and(|d| d.node(NodeId(*id)).is_none()))
            .collect();
        // The same revision keeps the objects collected for it, so their thumbnails are not drawn
        // again each time the files are looked at.
        let nested = if same_doc && want.iter().all(|id| c.nested_node(*id).is_some()) {
            c.nested.clone()
        } else {
            match doc {
                Some(d) if !want.is_empty() => stored_objects(d, want),
                _ => vec![],
            }
        };
        c = Arc::new(Cache { key, at: now, list, info: None, nested });
    }
    let want = first.filter(|_| o.info);
    if stale || c.info.as_ref().map(|(id, _)| *id) != want {
        let info = want.and_then(|id| Some((id, app.session.execute("links.info", &json!({ "id": id })).ok()?)));
        Arc::make_mut(&mut c).info = info;
        app.ui.links.cache = Arc::clone(&c);
    }
    // Files change on disk without a frame of ours: look again later.
    ctx.request_repaint_after(std::time::Duration::from_secs_f64(RECHECK));
    Some(c)
}

/// The objects of `doc` outside the layer tree whose ids are in `want`, sorted by id. The art of
/// the stored brushes is set on `doc` only while a command uses it ([`Document::mounted_art`]), so
/// when an id is not found otherwise that art is set on a copy of `doc`, which gives its objects
/// the ids `links.list` reports for them.
///
/// [`Document::mounted_art`]: vectorcraft_doc::Document::mounted_art
pub(super) fn stored_objects(doc: &vectorcraft_doc::Document, mut want: std::collections::HashSet<u64>) -> Vec<(u64, Arc<vectorcraft_doc::Node>)> {
    let mut out = vec![];
    collect_stored(doc, &mut want, &mut out);
    if !want.is_empty() && doc.mounted_art.is_empty() {
        let art = vectorcraft_brush::stored_art::StoredArt::read(doc);
        if !art.is_empty() {
            let mut d = doc.clone();
            art.mount(&mut d);
            collect_stored(&d, &mut want, &mut out);
        }
    }
    out.sort_by_key(|(id, _)| *id);
    out
}

/// Adds the objects of `d` outside the layer tree whose ids are in `want` to `out`, taking their
/// ids out of `want`.
fn collect_stored(d: &vectorcraft_doc::Document, want: &mut std::collections::HashSet<u64>, out: &mut Vec<(u64, Arc<vectorcraft_doc::Node>)>) {
    d.visit_stored_arcs(|n, place| {
        if !place.in_layers() && want.remove(&n.id.0) {
            out.push((n.id.0, Arc::clone(n)));
        }
    });
}

/// The ids of the selected image objects and placed documents.
fn selected_images(app: &VectorcraftApp) -> Vec<u64> {
    let Some(st) = app.session.active() else { return vec![] };
    let image = |id: &NodeId| {
        st.doc.node(*id).is_some_and(|n| matches!(n.kind, vectorcraft_doc::NodeKind::Image(_) | vectorcraft_doc::NodeKind::PlacedDocument(_)))
    };
    st.selection.objects.iter().filter(|id| image(id)).map(|id| id.0).collect()
}

/// Is row `r` of an object outside the layer tree (in opacity-mask art or the art of a symbol,
/// graph design or pattern), which the canvas can't select?
fn nested_row(r: &Value) -> bool {
    r["owner"].is_object()
}

/// The active document's uid and the ids of its selected objects.
fn canvas_selection(app: &VectorcraftApp) -> Option<(u64, Vec<u64>)> {
    app.session.active().map(|st| (st.uid, st.selection.objects.iter().map(|id| id.0).collect()))
}

/// The rows outside the layer tree picked in the panel, in the active document. A change of the
/// canvas selection, or another document, drops them.
fn nested_picks(app: &VectorcraftApp) -> Vec<u64> {
    match &app.ui.links.nested {
        Some((at, ids)) if Some(at) == canvas_selection(app).as_ref() => ids.clone(),
        _ => vec![],
    }
}

/// Forgets the rows picked in the panel once the canvas selection or the document has changed (the panel looks every
/// frame), so a later return to the same selection does not bring them back.
fn drop_stale_picks(app: &mut VectorcraftApp) {
    if app.ui.links.nested.as_ref().is_some_and(|(at, _)| Some(at) != canvas_selection(app).as_ref()) {
        app.ui.links.nested = None;
    }
}

/// Keep `ids` as the rows picked in the panel, with the canvas selection as it is now.
fn set_nested_picks(app: &mut VectorcraftApp, ids: Vec<u64>) {
    app.ui.links.nested = canvas_selection(app).map(|at| (at, ids));
}

/// The ids of the selected rows: the selected images and placed documents, then the rows outside
/// the layer tree picked in the panel.
fn selected_links(app: &VectorcraftApp) -> Vec<u64> {
    let mut ids = selected_images(app);
    for id in nested_picks(app) {
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    ids
}

/// [`selected_links`], for a command run outside the panel.
pub(crate) fn selected_rows_of_app(app: &VectorcraftApp) -> Vec<u64> {
    selected_links(app)
}

/// The rows of `list` whose image is selected.
fn selected_rows<'a>(list: &'a Value, selected: &[u64]) -> Vec<&'a Value> {
    list["links"].as_array().into_iter().flatten().filter(|r| r["id"].as_u64().is_some_and(|id| selected.contains(&id))).collect()
}

/// The ids of `rows` that `keep` accepts.
fn ids(rows: &[&Value], keep: impl Fn(&Value) -> bool) -> Vec<u64> {
    rows.iter().filter(|r| keep(r)).filter_map(|r| r["id"].as_u64()).collect()
}

fn status_is(r: &Value, status: &str) -> bool {
    r["status"].as_str() == Some(status)
}

/// Does row `r` hold a file that changed, or a preview that Update Link can read again?
fn is_modified(r: &Value) -> bool {
    status_is(r, "modified") || (r["preview"] == true && !status_is(r, "missing"))
}

fn report(app: &mut VectorcraftApp, r: Result<Value, String>, done: impl FnOnce(&Value) -> String) {
    let msg = match r {
        Ok(v) => done(&v),
        Err(e) => e,
    };
    app.status(msg);
}

/// The status bar's line for `n` relinked images.
pub(crate) fn relinked_status(n: usize) -> String {
    crate::i18n::msg_n(n as u64, "Relinked {n} image", "Relinked {n} images", &[])
}

/// The status bar's line for `n` images read again from their updated files.
pub(crate) fn updated_status(n: usize) -> String {
    crate::i18n::msg_n(n as u64, "Updated {n} image", "Updated {n} images", &[])
}

/// The status bar's line for `n` relinked images, naming the files `missed` that weren't found.
pub(crate) fn relinked_status_missed(n: usize, missed: &[&str]) -> String {
    if missed.is_empty() {
        relinked_status(n)
    } else {
        let (one, other) = ("Relinked {n} image; not in that folder: {names}", "Relinked {n} images; not in that folder: {names}");
        crate::i18n::msg_n(n as u64, one, other, &[("names", &missed.join(", "))])
    }
}

/// The ids of the selected image objects and placed documents (Relink's dialog).
pub(crate) fn selected_ids(app: &VectorcraftApp) -> Vec<u64> {
    selected_images(app)
}

/// Asks for `request` for the active document and runs `then` with the path picked; a cancelled
/// pick does nothing.
fn ask(app: &mut VectorcraftApp, request: PickRequest, then: impl FnOnce(&mut VectorcraftApp, String) + 'static) {
    file_asks::ask(app, request, Owner::Document, move |app, picked| {
        if let Some(path) = picked.first_path() {
            then(app, path.to_string_lossy().into_owned());
        }
        Ok(Value::Null)
    });
}

/// Replace images `ids` with a picked file (Control bar, Properties): it keeps its own aspect
/// ratio, fitted inside the old bounds (Placement Options: Fit), whatever their Placement Options
/// say.
pub(crate) fn replace(app: &mut VectorcraftApp, ids: Vec<u64>) {
    let request = vectorcraft_engine::cmd::fileio::place_filters().fold(PickRequest::open("links"), |r, (name, exts)| r.filter(name, exts));
    ask(app, request, move |app, path| {
        let r = app.run("links.relink", json!({ "ids": ids, "path": path, "preserve": "fit" }));
        report(app, r, |v| crate::i18n::msg_n(v["relinked"].as_array().map_or(0, Vec::len) as u64, "Replaced {n} image", "Replaced {n} images", &[]));
    });
}

/// Relink images `ids` (none: every missing one) to the files of their names in a picked folder.
fn relink_to_folder(app: &mut VectorcraftApp, ids: Vec<u64>) {
    ask(app, PickRequest::folder("links"), move |app, folder| {
        let ids = Some(ids).filter(|i| !i.is_empty());
        let r = app.run("links.relink", json!({ "ids": ids, "folder": folder }));
        report(app, r, |v| {
            let missed: Vec<&str> = v["notFound"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
            relinked_status_missed(v["relinked"].as_array().map_or(0, Vec::len), &missed)
        });
    });
}

/// Select image `id` and scroll it into view; one outside the layer tree only says that it has no
/// place on the canvas.
pub(crate) fn go_to(app: &mut VectorcraftApp, id: u64) {
    let Ok(r) = app.run("links.goTo", json!({ "id": id })) else { return };
    if r["onCanvas"] == false {
        app.status("The link is inside a symbol, graph design, pattern or opacity mask and has no place on the canvas");
        return;
    }
    let b: Vec<f64> = r["bounds"].as_array().into_iter().flatten().filter_map(Value::as_f64).collect();
    if let (&[x0, y0, x1, y1], Some(v)) = (&b[..], app.view_mut()) {
        v.center = vectorcraft_geom::Point::new((x0 + x1) / 2.0, (y0 + y1) / 2.0);
        v.fitted = true;
    }
}

/// Write embedded image `id` to a picked file (the web downloads it) and link it there.
pub(crate) fn unembed(app: &mut VectorcraftApp, id: u64, name: &str) {
    if app.services.download.is_some() {
        if let Ok(v) = app.session.execute("links.unembed", &json!({ "id": id }))
            && let (Some(name), Some(bytes)) = (v["name"].as_str(), v["dataBase64"].as_str().and_then(vectorcraft_format::base64_decode))
        {
            // A download that can't start is said like any file that can't be written.
            let _ = crate::io::write_reporting(app, name, &bytes);
        }
        return;
    }
    ask(app, PickRequest::save("links").file_name(name), move |app, path| {
        let r = app.run("links.unembed", json!({ "id": id, "path": path }));
        report(app, r, |v| format!("Unembedded to {}", v["path"].as_str().unwrap_or_default()));
    });
}

/// Whether Unembed… can act on `embedded` selected embedded images: one goes to a picked file,
/// several to a picked folder (the web downloads each one).
pub(crate) fn can_unembed(app: &VectorcraftApp, embedded: usize) -> bool {
    match embedded {
        0 => false,
        1 => true,
        _ => app.services.download.is_some() || file_asks::can(app, PickKind::Folder),
    }
}

/// Write embedded images `ids` to new files in a picked folder and link each one to its file, in
/// one undo step (the web downloads each one and links none).
pub(crate) fn unembed_several(app: &mut VectorcraftApp, ids: Vec<u64>) {
    if app.services.download.is_some() {
        for id in ids {
            unembed(app, id, "");
        }
        return;
    }
    ask(app, PickRequest::folder("links"), move |app, folder| {
        let r = app.run("links.unembed", json!({ "ids": ids, "folder": folder }));
        report(app, r, |v| {
            let n = v["unembedded"].as_array().map_or(0, Vec::len);
            crate::i18n::msg_n(n as u64, "Unembedded {n} image to {folder}", "Unembedded {n} images to {folder}", &[("folder", &folder)])
        });
    });
}

/// The UI commands `links.editOriginal` (open the linked file in its app) and `links.reveal`
/// (show it in its folder) for image `id` (default: the selected linked image) → `{path}`.
pub(crate) fn open_file(app: &mut VectorcraftApp, p: &Value, reveal: bool) -> Result<Value, String> {
    let id = p.get("id").and_then(Value::as_u64).or_else(|| {
        let list = app.session.execute("links.list", &json!({})).ok()?;
        let sel = selected_images(app);
        ids(&selected_rows(&list, &sel), |r| r["linked"] == true).first().copied()
    });
    let info = app.session.execute("links.info", &json!({ "id": id })).map_err(|e| e.to_string())?;
    let path = info["path"].as_str().ok_or("select a linked image: an embedded one has no file")?.to_string();
    if info["status"] == "missing" {
        return Err(format!("{path} can't be found: relink it first"));
    }
    // A placed document's file opens here, in a new tab: saving it updates this document.
    if !reveal && info["document"] == true {
        let opened = app.run("document.open", json!({ "path": path }))?;
        // Its missing fonts are asked about as for a document opened with File › Open.
        crate::dialogs::missing_fonts::after_open(app, &opened);
        return Ok(json!({ "path": path }));
    }
    if reveal {
        crate::io::reveal_path(app, &path)?;
    } else {
        app.refuse_automation("links.editOriginal")?;
        openable_original(&path)?;
        crate::io::open_original(app, &path)?;
    }
    Ok(json!({ "path": path }))
}

/// The UI command `links.copyInfo {ids?}` (the panel menu's Copy Info): copy the file names of
/// images and placed documents `ids` (default: the selected rows, [`selected_rows_of_app`], in the
/// order `links.list` gives them; `null` counts as left out) to the clipboard, one per line in that
/// order → `{text, names}`. An embedded image gives the name its row shows.
pub(crate) fn copy_info(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let key = "ids";
    let given = match p.get(key) {
        None | Some(Value::Null) => None,
        Some(Value::Array(a)) => {
            let mut out = Vec::with_capacity(a.len());
            for (index, value) in a.iter().enumerate() {
                out.push(value.as_u64().ok_or_else(|| format!("`{key}` entry {index} is {value}, not a non-negative integer object id"))?);
            }
            Some(out)
        }
        Some(_) => return Err(format!("{key} must be an array of non-negative integer object ids")),
    };
    let list = app.session.execute("links.list", &json!({})).map_err(|e| e.to_string())?;
    let rows: Vec<&Value> = list["links"].as_array().into_iter().flatten().collect();
    let wanted = match given {
        Some(wanted) => wanted,
        None => {
            let sel = selected_rows_of_app(app);
            ids(&rows, |r| r["id"].as_u64().is_some_and(|id| sel.contains(&id)))
        }
    };
    let by_id: HashMap<u64, &str> = rows.iter().filter_map(|r| Some((r["id"].as_u64()?, r["name"].as_str().unwrap_or_default()))).collect();
    let mut names = Vec::with_capacity(wanted.len());
    for id in &wanted {
        let name = by_id.get(id).ok_or_else(|| format!("object {} is not an image or a linked file", *id))?;
        names.push((*name).to_string());
    }
    if names.is_empty() {
        return Err("select the images, or pass ids".into());
    }
    let text = names.join("\n");
    app.clipboard_out = Some(text.clone());
    app.status("Copied the link info");
    Ok(json!({ "text": text, "names": names }))
}

/// Whether Edit Original may hand `path` to the system's default app: a regular file (after
/// following links) whose name, and the name it links to, have the extension of a format
/// VectorCraft reads. Anything else (a folder or app bundle, a script, a shortcut) is refused.
pub(crate) fn openable_original(path: &str) -> Result<(), String> {
    let readable = |p: &std::path::Path| {
        let ext = p.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
        use vectorcraft_engine::cmd::fileio::{FORMATS, read_extensions};
        !ext.is_empty() && FORMATS.iter().any(|f| f.read && read_extensions(f).contains(&ext.as_str()))
    };
    let name = std::path::Path::new(path).file_name().map_or_else(|| path.to_string(), |n| n.to_string_lossy().into_owned());
    let real = std::fs::canonicalize(path).map_err(|e| format!("can't open “{name}”: {e}"))?;
    if !real.is_file() {
        return Err(format!("“{name}” is not a file"));
    }
    if !readable(std::path::Path::new(path)) || !readable(&real) {
        return Err(format!("Edit Original opens image and document files only; “{name}” is not one"));
    }
    Ok(())
}

/// The tooltip of row `r`. A row outside the layer tree also names where it is stored
/// (`links.list`'s `owner`): the symbol, graph design, pattern or brush whose art holds it, and the
/// object whose opacity mask holds it, by `mask` (that object's name) or else by its id.
pub(super) fn status_tip(r: &Value, mask: Option<String>) -> String {
    let tip = file_tip(r);
    let owner = &r["owner"];
    let name = owner["name"].as_str().unwrap_or_default();
    let art = match owner["kind"].as_str() {
        Some("symbol") => Some(tl!("Symbols")),
        Some("graphDesign") => Some(tl!("Graph Design")),
        Some("pattern") => Some(tl!("Pattern")),
        Some("brush") => Some(tl!("Brushes")),
        _ => None,
    };
    let tip = match art {
        Some(kind) => format!("{tip}\n{} {name}", crate::i18n::colon(kind)),
        None => tip,
    };
    match owner["maskOf"].as_u64() {
        Some(id) => format!("{tip}\n{} {}", crate::i18n::colon(tl!("Opacity Mask")), mask.unwrap_or_else(|| id.to_string())),
        None => tip,
    }
}

/// The file, status and notes part of [`status_tip`].
fn file_tip(r: &Value) -> String {
    let path = r["found"].as_str().or(r["path"].as_str()).unwrap_or_default();
    let tip = match r["status"].as_str().unwrap_or_default() {
        "missing" => crate::i18n::fmt(tl!("Missing: {path}"), &[("path", path)]),
        "modified" => crate::i18n::fmt(tl!("Modified since it was read: {path}"), &[("path", path)]),
        "embedded" => tl!("Embedded").into(),
        _ => path.to_string(),
    };
    let tip = if r["dcsTransparency"] == true {
        format!("{tip}\n{}", tl!("Transparent, or overlaps transparent objects: check its separations"))
    } else {
        tip
    };
    // An image shown at reduced size or too large to draw (`links.list`'s note).
    match r["note"].as_str() {
        Some(note) => format!("{tip}\n{}", crate::i18n::msg(note)),
        None => tip,
    }
}

/// The name of the object whose opacity mask holds row `r`'s image, by the layer tree's index or
/// from the cache: no walk of the stored art each frame.
fn mask_name(app: &VectorcraftApp, doc: &vectorcraft_doc::Document, cache: &Cache, r: &Value) -> Option<String> {
    let m = r["owner"]["maskOf"].as_u64()?;
    let n = doc.node(NodeId(m)).or_else(|| cache.nested_node(m).map(|n| &**n))?;
    Some(super::layers::shown_name(app, n, crate::i18n::current()))
}

/// The icon of a row's status mark.
fn status_icon(status: &str) -> &'static str {
    match status {
        "missing" => "circle-help",
        "modified" => "triangle-alert",
        "embedded" => "image",
        _ => "link",
    }
}

/// The rows the list selected, as the canvas selection and the rows picked in the panel: the
/// canvas can't select a row outside the layer tree, and a pick without Ctrl/Cmd replaces both
/// kinds of selection.
fn apply_selection(app: &mut VectorcraftApp, rows: &[Value], items: Vec<u64>) {
    let outside = |id: &u64| rows.iter().any(|r| r["id"].as_u64() == Some(*id) && nested_row(r));
    let (nested, canvas): (Vec<u64>, Vec<u64>) = items.into_iter().partition(outside);
    app.run("select.set", json!({ "ids": canvas })).ok();
    set_nested_picks(app, nested);
}

/// What the selected rows allow, for the footer, the row menu and the panel menu alike.
struct Facts<'a> {
    rows: Vec<&'a Value>,
    /// Some image in the whole document is missing (not only in the selection).
    missing_any: bool,
}

impl<'a> Facts<'a> {
    fn of(list: &'a Value, selected: &[u64]) -> Self {
        Facts { rows: selected_rows(list, selected), missing_any: list["missing"].as_u64().unwrap_or(0) > 0 }
    }

    fn all(&self) -> Vec<u64> {
        ids(&self.rows, |_| true)
    }

    fn linked(&self) -> Vec<u64> {
        ids(&self.rows, |r| r["linked"] == true)
    }

    fn modified(&self) -> Vec<u64> {
        ids(&self.rows, is_modified)
    }

    fn embedded(&self) -> Vec<&'a Value> {
        self.rows.iter().copied().filter(|r| status_is(r, "embedded")).collect()
    }

    /// The first linked row whose file is there, for Edit Original and Show in Folder.
    fn file_row(&self) -> Value {
        match ids(&self.rows, |r| r["linked"] == true && !status_is(r, "missing")).first() {
            Some(id) => json!({ "id": id }),
            None => json!({}),
        }
    }
}

/// Is row `r` an object of the layer tree, which Go To can show on the canvas?
fn in_canvas(app: &VectorcraftApp, r: &Value) -> bool {
    !nested_row(r) && r["id"].as_u64().is_some_and(|id| app.session.active().is_some_and(|st| st.doc.node(NodeId(id)).is_some()))
}

/// Whether `act` can run on the selected rows; `row` is the row a menu opened on, which Go To
/// needs on the canvas.
fn can(app: &VectorcraftApp, act: Act, f: &Facts, row: Option<&Value>) -> bool {
    let any = !f.rows.is_empty();
    let editable = f.rows.iter().any(|r| r["linked"] == true && !status_is(r, "missing"));
    match act {
        Act::Relink => any && file_asks::can(app, PickKind::Open),
        Act::RelinkToFolder => (any || f.missing_any) && file_asks::can(app, PickKind::Folder),
        Act::FindInFolder => (!f.linked().is_empty() || f.missing_any) && file_asks::can(app, PickKind::Folder),
        Act::GoTo => row.map_or(f.rows.len() == 1, |r| in_canvas(app, r)),
        Act::Update => !f.modified().is_empty(),
        Act::Edit | Act::Reveal => editable,
        Act::Embed | Act::BreakLink => !f.linked().is_empty(),
        Act::Unembed => can_unembed(app, f.embedded().len()),
        Act::Placement | Act::CopyInfo => any,
    }
}

/// Runs `act` on the selected rows; `target` is the row a menu opened on, which Go To, Edit
/// Original and Show in Folder use when it is the one they can act on.
fn act(app: &mut VectorcraftApp, act: Act, target: Option<u64>) {
    let cache = Arc::clone(&app.ui.links.cache);
    let selected = selected_links(app);
    let f = Facts::of(&cache.list, &selected);
    let file_arg =
        || match target.filter(|id| f.rows.iter().any(|r| r["id"].as_u64() == Some(*id) && r["linked"] == true && !status_is(r, "missing"))) {
            Some(id) => json!({ "id": id }),
            None => f.file_row(),
        };
    match act {
        Act::Relink => crate::menus::invoke(app, "ui.relinkDialog", json!({ "ids": f.all() })),
        Act::RelinkToFolder => relink_to_folder(app, f.all()),
        Act::FindInFolder => crate::menus::invoke(app, "ui.findLinksInFolder", json!({ "ids": f.linked() })),
        Act::GoTo => {
            if let Some(id) = target.or_else(|| f.all().first().copied()) {
                go_to(app, id);
            }
        }
        Act::Update => {
            let r = app.run("links.update", json!({ "ids": f.modified() }));
            report(app, r, |v| updated_status(v["updated"].as_array().map_or(0, Vec::len)));
        }
        Act::Edit => crate::menus::invoke(app, "links.editOriginal", file_arg()),
        Act::Reveal => crate::menus::invoke(app, "links.reveal", file_arg()),
        Act::Embed => embed_images(app, f.linked()),
        Act::BreakLink => break_link(app, f.linked()),
        Act::Unembed => match &f.embedded()[..] {
            [] => {}
            [r] => {
                if let Some(id) = r["id"].as_u64() {
                    unembed(app, id, r["name"].as_str().unwrap_or("Image"));
                }
            }
            rows => unembed_several(app, rows.iter().filter_map(|r| r["id"].as_u64()).collect()),
        },
        Act::Placement => crate::menus::invoke(app, "ui.placementOptionsDialog", json!({ "ids": f.all() })),
        Act::CopyInfo => crate::menus::invoke(app, "links.copyInfo", json!({ "ids": f.all() })),
    }
}

/// The Links panel: the images as a list with a footer of link actions and the Link Info section.
pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    drop_stale_picks(app);
    let Some(mut cache) = cached(app, ui.ctx()) else {
        Panel::new(tl!("Links")).show(ui, |ui, _| {
            Text::new(tl!("No document")).shade(Shade::Muted).show(ui);
        });
        return;
    };
    let uid = app.session.active().map(|st| st.uid);
    let reveal = app.ui.links.show.take().filter(|(doc, _)| Some(*doc) == uid).map(|(_, id)| id);
    let listed = |cache: &Cache, id: u64| cache.list["links"].as_array().is_some_and(|rows| rows.iter().any(|r| r["id"].as_u64() == Some(id)));
    // A Show filter that leaves the requested row out goes back to Show All.
    if reveal.is_some_and(|id| app.ui.links.options.show != "all" && !listed(&cache, id)) {
        app.ui.links.options.show = "all";
        let Some(all) = cached(app, ui.ctx()) else { return };
        cache = all;
    }
    let Some(doc) = app.session.active().map(|st| st.doc.clone()) else { return };
    let o = app.ui.links.options.clone();
    let selected = selected_links(app);
    let mut selection = std::mem::take(&mut app.ui.links.selection);
    selection.items = selected.clone();
    let open_menu = app.ui.links.menu;
    let units = app.session.general_unit();

    let view: &VectorcraftApp = app;
    let facts = Facts::of(&cache.list, &selected);
    let rows: &[Value] = cache.list["links"].as_array().map_or(&[], Vec::as_slice);
    let by_id: HashMap<u64, &Value> = rows.iter().filter_map(|r| Some((r["id"].as_u64()?, r))).collect();
    let row_ids: Vec<u64> = rows.iter().filter_map(|r| r["id"].as_u64()).collect();
    // `lookup` gives the objects outside the layer tree.
    let thumbs = super::layers::link_thumbnails(ui.ctx());
    let nodes = super::layers::NodeThumbs::new(&doc, |id| cache.nested_node(id.0).cloned());
    let (thumbs, nodes) = (&thumbs, &nodes);
    let ppp = ui.ctx().pixels_per_point();

    let mut footer_pick = None;
    let (mut activated, mut menu_for, mut row_act) = (None, None, None);
    Panel::new(tl!("Links"))
        .collection_body()
        .footer(|ui, _| {
            let info = FooterAction::new(IconId::new(if o.info { "chevron-down" } else { "chevron-right" }), tl!("Show Link Info"));
            let actions = std::iter::once(info)
                .chain(FOOTER.iter().map(|(a, icon, label)| FooterAction::new(IconId::new(icon), tl!(label)).enabled(can(view, *a, &facts, None))));
            footer_pick = FooterBar::actions(actions).at_start(1).show_footer(ui).picked;
        })
        .show(ui, |ui, _| {
            let empty = if o.show == "all" { tl!("No images") } else { tl!("Nothing matches the Show filter.") };
            let mut list = List::<u64>::new(tl!("Links")).selection(Bind::bound(&mut selection)).empty(empty).row_menu();
            if let Some(id) = reveal {
                list = list.reveal(id);
            }
            let out = list.show_items(ui, &keyed(&row_ids, |id| *id), &mut |id, _| {
                let Some(r) = by_id.get(id) else { return ListRow::spacer() };
                let name = r["name"].as_str().unwrap_or_default();
                let tip = status_tip(r, mask_name(view, &doc, &cache, r));
                let node = NodeId(*id);
                let mut row = ListRow::new(name.to_string()).thumb(o.thumb, move |cx, rect| {
                    let request = craft::ui::primitives::ThumbRequest::for_cell(rect.width(), ppp);
                    let shown = thumbs
                        .state(
                            cx.painter.ctx(),
                            nodes,
                            &super::layers::NodeThumb { uid: uid.unwrap_or_default(), node },
                            &request,
                            craft::host::queue::Priority::VISIBLE,
                        )
                        .shown();
                    if let Some(texture) = shown {
                        cx.picture(texture.id, rect, texture.uv(), 0.0);
                    }
                });
                if let Some(page) = r["page"].as_u64() {
                    row = row.trailing(page.to_string());
                }
                if r["dcsTransparency"] == true {
                    row = row.trailing_icon(IconId::new("triangle-alert"), tip.clone());
                }
                row.trailing_icon(IconId::new(status_icon(r["status"].as_str().unwrap_or_default())), tip)
            });
            for event in out.events {
                match event {
                    CollectionEvent::Activate(id) => activated = Some(id),
                    CollectionEvent::ContextMenu(request) => menu_for = Some(request.key),
                    _ => {}
                }
            }
            if let Some(r) = menu_for.or(open_menu).and_then(|id| by_id.get(&id).copied()) {
                row_act = show_row_menu(ui, &out.response, view, &facts, r);
            }
            if o.info {
                link_info(ui, cache.info.as_ref().map(|(_, i)| i), units);
            }
        });

    app.ui.links.selection = selection;
    if app.ui.links.selection.items != selected {
        let items = app.ui.links.selection.items.clone();
        apply_selection(app, rows_of(&cache), items);
    }
    if menu_for.is_some() {
        app.ui.links.menu = menu_for;
    }
    if let Some(id) = activated {
        go_to(app, id);
    }
    match footer_pick {
        Some(0) => app.ui.links.options.info = !o.info,
        Some(i) => {
            if let Some((a, ..)) = FOOTER.get(i - 1) {
                act(app, *a, None);
            }
        }
        None => {}
    }
    if let Some((a, id)) = row_act {
        app.ui.links.menu = None;
        act(app, a, Some(id));
    }
}

/// The rows of the cached list.
fn rows_of(cache: &Cache) -> &[Value] {
    cache.list["links"].as_array().map_or(&[], Vec::as_slice)
}

/// The context menu of row `r`, hanging from `opener`: the link actions for the selected rows,
/// and Embed Image or Unembed… by what the row is. Returns the action chosen with the row.
fn show_row_menu(ui: &mut Ui, opener: &egui::Response, app: &VectorcraftApp, f: &Facts, r: &Value) -> Option<(Act, u64)> {
    let id = r["id"].as_u64()?;
    let last = if status_is(r, "embedded") { (Act::Unembed, "Unembed…") } else { (Act::Embed, "Embed Image") };
    let items = [
        (Act::Relink, "Relink…"),
        (Act::GoTo, "Go To Link"),
        (Act::Update, "Update Link"),
        (Act::Edit, "Edit Original"),
        (Act::Reveal, "Show in Folder"),
        last,
    ];
    let shown = Menu::context(tl!("Link"), opener).show(ui, |ui, _| {
        let mut ran = None;
        for (a, label) in items {
            if MenuItem::new(tl!(label)).enabled(can(app, a, f, Some(r))).show(ui).ran {
                ran = Some(a);
            }
        }
        ran
    });
    shown.inner.flatten().map(|a| (a, id))
}

/// Break Link (`links.embed`) of `ids`: images keep their files' pixels, placed documents become
/// editable copies of their art.
fn break_link(app: &mut VectorcraftApp, ids: Vec<u64>) {
    let r = app.run("links.embed", json!({ "ids": ids }));
    report(app, r, |v| {
        let missing = v["missing"].as_array().map_or(0, Vec::len);
        let n = v["embedded"].as_array().map_or(0, Vec::len);
        if missing == 0 {
            crate::i18n::msg_n(n as u64, "Broke {n} link", "Broke {n} links", &[])
        } else {
            crate::i18n::msg_n(
                n as u64,
                "Broke {n} link; {missing} can't be read: relink or update them first",
                "Broke {n} links; {missing} can't be read: relink or update them first",
                &[("missing", &missing.to_string())],
            )
        }
    });
}

/// Embed images `ids` (`links.embed`).
fn embed_images(app: &mut VectorcraftApp, ids: Vec<u64>) {
    let r = app.run("links.embed", json!({ "ids": ids }));
    report(app, r, |v| {
        let missing = v["missing"].as_array().map_or(0, Vec::len);
        let n = v["embedded"].as_array().map_or(0, Vec::len);
        if missing == 0 {
            crate::i18n::msg_n(n as u64, "Embedded {n} image", "Embedded {n} images", &[])
        } else {
            crate::i18n::msg_n(
                n as u64,
                "Embedded {n} image; {missing} missing: relink them first",
                "Embedded {n} images; {missing} missing: relink them first",
                &[("missing", &missing.to_string())],
            )
        }
    });
}

/// Bytes as B, KB or MB.
fn size_label(bytes: u64) -> String {
    match bytes {
        b if b < 1024 => format!("{b} B"),
        b if b < 1024 * 1024 => format!("{:.1} KB", b as f64 / 1024.0),
        b => format!("{:.1} MB", b as f64 / (1024.0 * 1024.0)),
    }
}

/// Milliseconds since the Unix epoch as a UTC date and time.
pub(crate) fn date_label(ms: u64) -> String {
    let [y, mo, d, h, mi, _] = vectorcraft_doc::metadata::civil(i64::try_from(ms / 1000).unwrap_or(i64::MAX));
    format!("{y:04}-{mo:02}-{d:02} {h:02}:{mi:02} UTC")
}

/// One line of Link Info: a labelled field whose text is its value.
fn info_row(ui: &mut Ui, label: &'static str, value: String) {
    Field::new(label).value(value.clone()).show(ui, |ui, _| {
        Text::new(value.clone()).show(ui);
    });
}

/// The Link Info section: the details of `info` (`links.info`), or a hint.
fn link_info(ui: &mut Ui, info: Option<&Value>, units: vectorcraft_doc::Unit) {
    Section::fixed(tl!("Link Info")).show(ui, |ui, _| {
        let Some(i) = info else {
            Text::new(tl!("Select an image to see its Link Info.")).shade(Shade::Muted).show(ui);
            return;
        };
        let pair = |k: &str| {
            let n: Vec<f64> = i[k].as_array().into_iter().flatten().filter_map(Value::as_f64).collect();
            match n[..] {
                [x, y] if (x - y).abs() < 0.05 => format!("{x:.0}"),
                [x, y] => format!("{x:.0} × {y:.0}"),
                _ => String::new(),
            }
        };
        info_row(ui, tl!("Name"), i["fileName"].as_str().or(i["name"].as_str()).unwrap_or_default().into());
        info_row(ui, tl!("Format"), i["format"].as_str().unwrap_or_default().into());
        info_row(ui, tl!("Color Space"), i["colorMode"].as_str().unwrap_or_default().into());
        if let Some(l) = i["location"].as_str() {
            info_row(ui, tl!("Location"), l.into());
        }
        // A placed document has no resolution: its rows are left out.
        for (label, k) in [(tl!("PPI"), "ppi"), (tl!("Effective PPI"), "effectivePpi")] {
            if i.get(k).is_some() {
                info_row(ui, label, pair(k));
            }
        }
        // Pixels, or a placed document's artboard size.
        let dims = match (i["pixelWidth"].as_u64(), i["pageWidth"].as_f64(), i["pageHeight"].as_f64()) {
            (Some(w), _, _) => format!("{w} × {} px", i["pixelHeight"].as_u64().unwrap_or(0)),
            (None, Some(w), Some(h)) => format!("{} × {}", units.format(w), units.format(h)),
            _ => String::new(),
        };
        info_row(ui, tl!("Dimensions"), dims);
        let scale: Vec<f64> = i["scale"].as_array().into_iter().flatten().filter_map(Value::as_f64).collect();
        if let [x, y] = scale[..] {
            info_row(ui, tl!("Scale"), format!("H: {x:.1}%  V: {y:.1}%"));
        }
        info_row(ui, tl!("Rotation"), format!("{:.1}°", i["rotation"].as_f64().unwrap_or(0.0)));
        if let Some(b) = i["fileSize"].as_u64() {
            info_row(ui, tl!("Size"), size_label(b));
        }
        for (k, label) in [("created", tl!("Created")), ("modified", tl!("Modified"))] {
            if let Some(ms) = i[k].as_u64() {
                info_row(ui, label, date_label(ms));
            }
        }
        if let Some(p) = i["page"].as_u64() {
            info_row(ui, tl!("Page"), p.to_string());
        }
        let status = match i["status"].as_str().unwrap_or_default() {
            "ok" => tl!("Linked"),
            "missing" => tl!("Missing"),
            "modified" => tl!("Modified"),
            _ => tl!("Embedded"),
        };
        info_row(ui, tl!("Status"), status.into());
    });
}

/// A command row with the state its panel-menu row shows.
fn entry(app: &VectorcraftApp, f: &Facts, id: &str, params: Value, a: Act) -> (CommandRef, CommandState) {
    (CommandRef::with_params(id, params), CommandState::new().enabled(can(app, a, f, None)))
}

/// A command row that is always enabled and checked or not.
fn option_entry(id: &str, params: Value, checked: bool) -> (CommandRef, CommandState) {
    (CommandRef::with_params(id, params), CommandState::new().enabled(true).checked(checked))
}

/// The panel menu's command rows with their states, in the groups the menu separates: the link
/// actions, Embed and Placement Options, Show, Sort, Copy Info, and the thumbnail and Link Info
/// options. The Select rows between the second and third group are the add-on's.
fn groups(app: &VectorcraftApp) -> [Vec<(CommandRef, CommandState)>; 6] {
    let cache = Arc::clone(&app.ui.links.cache);
    let selected = selected_links(app);
    let f = Facts::of(&cache.list, &selected);
    let o = &app.ui.links.options;
    let e = |id: &str, params: Value, a: Act| entry(app, &f, id, params, a);
    [
        vec![
            e("ui.relinkDialog", json!({ "ids": f.all() }), Act::Relink),
            e("ui.links.relinkToFolder", Value::Null, Act::RelinkToFolder),
            e("ui.findLinksInFolder", json!({ "ids": f.linked() }), Act::FindInFolder),
            e("ui.links.goTo", Value::Null, Act::GoTo),
            e("ui.links.update", Value::Null, Act::Update),
            e("links.editOriginal", f.file_row(), Act::Edit),
        ],
        vec![
            e("ui.links.embed", Value::Null, Act::Embed),
            e("ui.links.unembed", Value::Null, Act::Unembed),
            e("ui.placementOptionsDialog", json!({ "ids": f.all() }), Act::Placement),
            e("links.reveal", f.file_row(), Act::Reveal),
        ],
        SHOW.iter().map(|(k, _)| option_entry("ui.links.show", json!({ "show": k }), o.show == *k)).collect(),
        SORT.iter().map(|(k, _)| option_entry("ui.links.sort", json!({ "sort": k }), o.sort == Some(*k))).collect(),
        vec![e("links.copyInfo", json!({ "ids": f.all() }), Act::CopyInfo)],
        THUMBS
            .iter()
            .map(|(k, size, _)| option_entry("ui.links.thumbnails", json!({ "size": k }), o.thumb == *size))
            .chain([option_entry("ui.links.linkInfo", Value::Null, o.info), option_entry("ui.links.dcs", Value::Null, o.dcs)])
            .collect(),
    ]
}

/// The rows of the panel menu: the link commands for the selected rows, Select Linked / Embedded /
/// Missing / Modified for the whole document, the Show and Sort options, Copy Info and the
/// thumbnail options.
pub(crate) fn menu_rows(app: &VectorcraftApp) -> Vec<PanelRow> {
    let mut rows = vec![];
    for (i, group) in groups(app).into_iter().enumerate() {
        if i > 0 {
            rows.push(PanelRow::Separator);
        }
        if i == 2 {
            rows.extend(link_status::rows());
            rows.push(PanelRow::Separator);
        }
        rows.extend(group.into_iter().map(|(command, _)| PanelRow::Command(command)));
    }
    rows
}

/// The enabled and checked states of the panel menu's rows, into the frame's command states.
pub(crate) fn menu_states(app: &VectorcraftApp, states: &mut CommandStates) {
    for (command, state) in groups(app).into_iter().flatten() {
        states.set(command, state);
    }
    link_status::states(app, states);
}

/// The option of `names` that `p[key]` names.
fn choose(p: &Value, key: &str, mut names: impl Iterator<Item = &'static str>) -> Result<&'static str, String> {
    let given = p.get(key).and_then(Value::as_str).unwrap_or_default();
    names.find(|n| *n == given).ok_or_else(|| format!("`{key}` is not one of the options: {given:?}"))
}

/// The panel's own commands (`ui.links.*`), for the panel menu's rows and automation; `None` for
/// any other id.
pub(crate) fn run(app: &mut VectorcraftApp, id: &str, p: &Value) -> Option<Result<Value, String>> {
    let acted = |app: &mut VectorcraftApp, a: Act, target: Option<u64>| {
        act(app, a, target);
        Ok(())
    };
    let done = match id {
        "ui.links.relinkToFolder" => acted(app, Act::RelinkToFolder, None),
        "ui.links.goTo" => acted(app, Act::GoTo, p.get("id").and_then(Value::as_u64)),
        "ui.links.update" => acted(app, Act::Update, None),
        "ui.links.embed" => acted(app, Act::Embed, None),
        "ui.links.unembed" => acted(app, Act::Unembed, None),
        "ui.links.show" => choose(p, "show", SHOW.iter().map(|(k, _)| *k)).map(|k| app.ui.links.options.show = k),
        // Choosing the chosen sort again clears it.
        "ui.links.sort" => choose(p, "sort", SORT.iter().map(|(k, _)| *k)).map(|k| {
            let o = &mut app.ui.links.options;
            o.sort = if o.sort == Some(k) { None } else { Some(k) };
        }),
        "ui.links.thumbnails" => choose(p, "size", THUMBS.iter().map(|(k, ..)| *k)).map(|k| {
            if let Some((_, size, _)) = THUMBS.iter().find(|(n, ..)| *n == k) {
                app.ui.links.options.thumb = *size;
            }
        }),
        "ui.links.linkInfo" => {
            let _: () = app.ui.links.options.info = !app.ui.links.options.info;
            Ok(())
        }
        "ui.links.dcs" => {
            let _: () = app.ui.links.options.dcs = !app.ui.links.options.dcs;
            Ok(())
        }
        _ => return None,
    };
    Some(done.map(|()| Value::Null))
}
