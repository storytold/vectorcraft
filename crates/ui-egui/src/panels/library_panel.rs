//! Library panels: read-only views of one library each (swatch, graphic style, symbol and brush
//! libraries; any kind of library implementing [`LibraryKind`]) shown by the framework's preset
//! browser with thumbnail and list views, a find field, the libraries menu and previous / next
//! arrows. A plain click on an item adds it to the document and applies it; Shift and Cmd/Ctrl-clicks
//! select items (and folders) for Add to …. A kind whose items drag ([`LibraryKind::drags`]) lets
//! them be dragged out of the panel.
//!
//! As in Illustrator (<https://helpx.adobe.com/illustrator/using/using-creating-swatches.html>,
//! "Swatch libraries"), each open library is a panel of its own, `library:<kind>:<library id>`
//! ([`panel_id`]), that floats, tabs, docks and closes as any panel of the shared docking system:
//! `window.swatchLibrary`, `window.graphicStyleLibrary`, `window.symbolLibrary` and `window.brushLibrary` open one, or bring it forward when it is
//! open. `UiState::library_panels` keeps each one's view, Show Find Field and Persistent
//! (`ui.libraryPanel`); a persistent panel reopens when the app starts ([`restore`]). A persistent
//! panel whose library can't be found shows the library as missing, with its file and Locate
//! Library… (`ui.libraryPanel.locate`). The find text is the browser's; the selection is panel state
//! (`UiState::library_selection`).

use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::HashMap;

use craft::behavior::{IconId, Selection, SelectionModel};
use craft::host::{PickRequest, Picked};
use craft::theme::variant::Tone;
use craft::ui::contract::{Bind, MemoKey, MemoProps, SelectMode, Show as _, ShowContainer as _, Trigger};
use craft::ui::menu::{Menu, MenuItem, PanelRow};
use craft::ui::notices::{Notice, NoticeAction, NoticeLayout};
use craft::ui::patterns::presets::{
    PresetBrowser, PresetEvent, PresetFns, PresetKind, PresetRow, PresetRowMenu, PresetSource, PresetState, PresetView,
};
use craft::ui::primitives::{ContentPainterCx, IconSpec};
use craft::ui::runtime::access::PopupKind;
use craft::ui::runtime::commands::{CommandRef, CommandState, CommandStates};
use craft::ui::sections::{FooterAction, FooterBar, FooterOutput, Panel};
use egui::{Rect, Ui};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use vectorcraft_doc::style_libs::STYLES_EXT;
use vectorcraft_engine::cmd::fileio;
use vectorcraft_engine::cmd::swatchlib::{self, LibraryInfo};

use super::swatches::View;
use crate::VectorcraftApp;
use crate::file_asks::{self, Owner};
use crate::state::UiState;

/// What every library panel's id starts with.
pub const PREFIX: &str = "library:";

/// The most persistent panels [`restore`] reads from the preferences.
const MAX_RESTORED: usize = 256;

/// A library panel's settings: one per library opened this session, and the persistent ones
/// (also once closed), most recently opened or brought forward last.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LibraryPanel {
    /// The kind of library ([`LibraryKind::KIND`], e.g. "swatches").
    pub kind: String,
    /// The library's id.
    pub id: String,
    /// The file a loaded library was read from, which [`restore`] reads it from again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// The view picked from the panel menu (none: the kind's, [`LibraryKind::VIEW`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view: Option<View>,
    /// Show Find Field is off.
    #[serde(default)]
    pub hide_find: bool,
    /// Persistent: the panel reopens when the app starts.
    #[serde(default)]
    pub persistent: bool,
}

impl LibraryPanel {
    /// The panel's id.
    pub fn panel(&self) -> String {
        panel_id(&self.kind, &self.id)
    }
}

/// A library as the libraries menu lists it.
pub(crate) struct LibraryRef {
    pub id: String,
    pub name: String,
    /// The submenu it is listed under (`None`: the top level).
    pub submenu: Option<&'static str>,
}

/// A row of the panel: an item, or the folder of a group of items (`item` None).
pub(crate) struct Row<'a, I> {
    pub name: &'a str,
    pub item: Option<&'a I>,
}

/// The key of a browser row: the panel it is in and the stored name of its item or folder. A drag out
/// of the panel carries it, so a drop target knows the library an item came from.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, MemoProps)]
pub(crate) struct LibraryKey {
    /// The id of the panel the row is in ([`panel_id`]).
    pub panel: String,
    /// The item's or folder's name as the library stores it.
    pub name: String,
    /// The row is a folder.
    pub folder: bool,
}

/// One kind of library the panels show.
pub(crate) trait LibraryKind {
    /// `LibraryPanel::kind`, and the middle part of its panels' ids.
    const KIND: &'static str;
    /// The UI command that opens a library's panel (`{library: id}`).
    const OPEN: &'static str;
    /// The engine command listing the libraries, for error messages.
    const LIST: &'static str;
    /// The engine command loading a library from a file (`{path}` → `{library}`).
    const LOAD: &'static str;
    /// The libraries menu's tooltip ("Swatch Libraries Menu").
    const MENU: &'static str;
    /// What adding the selection is called ("Add to Swatches").
    const ADD: &'static str;
    /// The view a panel shows until another is picked.
    const VIEW: View = View::MediumThumb;
    /// A library's contents, cheap to clone.
    type Lib;
    type Item: Sync;
    /// What the thumbnails paint with besides each item (a panel setting, the documents a library's patterns come
    /// from); `()` for nothing.
    type Thumbs: MemoProps + Send;
    /// Every library, in menu order.
    fn list(app: &VectorcraftApp) -> Vec<LibraryRef>;
    /// Library `id`: its name and contents.
    fn get(app: &VectorcraftApp, id: &str) -> Option<(String, Self::Lib)>;
    /// Library `key` (an id or a name).
    fn find(app: &VectorcraftApp, key: &str) -> Found;
    /// The file library `id` was read from, if any.
    fn path(app: &VectorcraftApp, id: &str) -> Option<String>;
    /// The rows of library `id`; the browser's search filters what it shows (by the shown name and
    /// [`PresetRow::search_name`]).
    fn rows<'a>(lib: &'a Self::Lib, id: &str) -> Vec<Row<'a, Self::Item>>;
    /// Row `name` of library `id` as its list row and tooltips show it (by default as it is). The
    /// row keeps `name` for its tile, selection, clicks, drags and the commands they run.
    fn label<'a>(_id: &str, name: &'a str) -> Cow<'a, str> {
        Cow::Borrowed(name)
    }
    /// The browser row of `item`, named `label`, with the tooltip that describes it. The panel
    /// sets the row's key.
    fn row(id: &str, item: &Self::Item, label: &str) -> PresetRow<LibraryKey>;
    /// What the thumbnails of `lib` paint with.
    fn thumbs(app: &VectorcraftApp, lib: &Self::Lib) -> Self::Thumbs;
    /// An item's thumbnail in `rect`, painted with `thumbs`.
    fn thumb(thumbs: &Self::Thumbs, cx: &ContentPainterCx, rect: Rect, item: &Self::Item);
    /// An item described in words, for list tooltips.
    fn describe(item: &Self::Item) -> String;
    /// A plain click on item `name` of library `id`: add it to the document and apply it.
    fn click(app: &mut VectorcraftApp, ui: &Ui, id: &str, name: &str);
    /// Add the items and folders `names` of library `id` to the document.
    fn add(app: &mut VectorcraftApp, id: &str, names: Vec<String>);
    /// Whether items of this kind drag out of the panel.
    fn drags() -> bool {
        false
    }
    /// Items heading the libraries menu.
    fn menu_head(_app: &mut VectorcraftApp, _ui: &mut Ui) {}
    /// Items closing the libraries menu.
    fn menu_tail(_app: &mut VectorcraftApp, _ui: &mut Ui) {}
}

/// A library looked up by its id or name: its id, name and item count.
pub(crate) type Found = Option<(String, String, usize)>;

/// What code that knows a library panel by its id alone needs of its kind.
#[derive(Clone, Copy)]
pub(crate) struct Kind {
    pub kind: &'static str,
    list: &'static str,
    load: &'static str,
    view: View,
    name: fn(&VectorcraftApp, &str) -> Option<String>,
    find: fn(&VectorcraftApp, &str) -> Found,
    path: fn(&VectorcraftApp, &str) -> Option<String>,
    neighbour: fn(&VectorcraftApp, &str, isize) -> Option<String>,
    add: fn(&mut VectorcraftApp, &str, Vec<String>),
    add_label: &'static str,
    body: fn(&mut VectorcraftApp, &mut Ui, &str),
}

fn kind_of<K: LibraryKind>() -> Kind {
    Kind {
        kind: K::KIND,
        list: K::LIST,
        load: K::LOAD,
        view: K::VIEW,
        name: |app, id| K::get(app, id).map(|(name, _)| name),
        find: K::find,
        path: K::path,
        neighbour: neighbour::<K>,
        add: K::add,
        add_label: K::ADD,
        body: body::<K>,
    }
}

/// The kind of library `kind` names.
fn kind(kind: &str) -> Option<Kind> {
    [
        kind_of::<super::swatches::SwatchLibraries>(),
        kind_of::<super::graphic_styles::GraphicStyleLibraries>(),
        kind_of::<super::symbols::SymbolLibraries>(),
        kind_of::<super::brushes::BrushLibraries>(),
    ]
    .into_iter()
    .find(|k| k.kind == kind)
}

/// The id of the panel of library `id` of `kind` (`library:swatches:pastels`).
pub fn panel_id(kind: &str, id: &str) -> String {
    format!("{PREFIX}{kind}:{id}")
}

/// The kind and library id of library panel `panel`.
pub(crate) fn parse(panel: &str) -> Option<(Kind, &str)> {
    let (k, id) = panel.strip_prefix(PREFIX)?.split_once(':')?;
    if id.is_empty() || panel.len() > 1024 {
        return None;
    }
    Some((kind(k)?, id))
}

/// Is panel `panel` open (in the dock or floating)?
pub(crate) fn is_open(ui: &UiState, panel: &str) -> bool {
    ui.dock_workspace.layout().contains(&panel.to_string())
}

/// The open library panels.
fn open_panels(ui: &UiState) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    for p in ui.dock_workspace.layout().panels() {
        if p.starts_with(PREFIX) && !out.contains(p) {
            out.push(p.clone());
        }
    }
    out
}

/// Is a library panel of kind `K` open?
pub(crate) fn any_open<K: LibraryKind>(ui: &UiState) -> bool {
    open_panels(ui).iter().any(|p| parse(p).is_some_and(|(k, _)| k.kind == K::KIND))
}

/// The library panel opened or brought forward last that is open.
#[cfg(test)]
pub(crate) fn last_open(ui: &UiState) -> Option<&LibraryPanel> {
    ui.library_panels.iter().rev().find(|p| is_open(ui, &p.panel()))
}

fn settings<'a>(ui: &'a UiState, kind: &str, id: &str) -> Option<&'a LibraryPanel> {
    ui.library_panels.iter().find(|p| p.kind == kind && p.id == id)
}

/// The settings of library `id`'s panel, made when it has none.
fn settings_mut<'a>(app: &'a mut VectorcraftApp, k: Kind, id: &str) -> Option<&'a mut LibraryPanel> {
    if settings(&app.ui, k.kind, id).is_none() {
        let path = if id.starts_with("loaded/") { (k.path)(app, id) } else { None };
        app.ui.library_panels.push(LibraryPanel { kind: k.kind.into(), id: id.into(), path, view: None, hide_find: false, persistent: false });
    }
    app.ui.library_panels.iter_mut().find(|p| p.kind == k.kind && p.id == id)
}

/// The file of library `id` of `k` when the library is missing: its panel is persistent and the
/// library can't be found (its file was gone at startup, or a User Defined library's file was
/// deleted). The file is the path the library was read from, else the library's id.
fn missing_file(app: &VectorcraftApp, k: Kind, id: &str) -> Option<String> {
    let s = settings(&app.ui, k.kind, id).filter(|s| s.persistent)?;
    (k.name)(app, id).is_none().then(|| s.path.clone().unwrap_or_else(|| id.to_string()))
}

/// The file of library `id` of `k` when its panel is open and the library can't be found,
/// Persistent or not (a panel whose Persistent a command turned off stays open until [`tidy`]).
fn open_missing_file(app: &VectorcraftApp, k: Kind, id: &str) -> Option<String> {
    let s = settings(&app.ui, k.kind, id).filter(|_| is_open(&app.ui, &panel_id(k.kind, id)))?;
    (k.name)(app, id).is_none().then(|| s.path.clone().unwrap_or_else(|| id.to_string()))
}

/// The name of missing library `id` of `k` ([`missing_file`], or [`open_missing_file`]): its file's
/// name without the extension.
fn missing_name(app: &VectorcraftApp, k: Kind, id: &str) -> Option<String> {
    let file = missing_file(app, k, id).or_else(|| open_missing_file(app, k, id))?;
    Some(std::path::Path::new(&file).file_stem().map_or_else(|| file.clone(), |s| s.to_string_lossy().into_owned()))
}

/// Library panel `panel`'s tab label: its library's name (a built-in one's in the UI language), or
/// a missing library's [`missing_name`].
pub(crate) fn tab_label(app: &VectorcraftApp, panel: &str) -> Option<String> {
    let (k, id) = parse(panel)?;
    match (k.name)(app, id) {
        Some(name) => Some(library_name(id, &name).to_string()),
        None => missing_name(app, k, id),
    }
}

/// Library panel `panel`'s contents (the dock or its floating group draws its tab and menu button).
pub(crate) fn show(app: &mut VectorcraftApp, ui: &mut Ui, panel: &str) {
    if let Some((k, id)) = parse(panel) {
        (k.body)(app, ui, id);
    }
}

/// Per frame, before the panels are drawn: close the panels of libraries that are gone (a User
/// Defined library whose file was deleted), except persistent ones, which show the library as
/// missing ([`missing_file`]).
pub fn tidy(app: &mut VectorcraftApp) {
    for panel in open_panels(&app.ui) {
        if parse(&panel).is_some_and(|(k, id)| (k.name)(app, id).is_none() && missing_file(app, k, id).is_none()) {
            close(app, &panel);
        }
    }
}

fn close(app: &mut VectorcraftApp, panel: &str) {
    if let Err(e) = app.run("window.panel.close", json!({ "panel": panel })) {
        app.status(e);
    }
}

/// At startup, after the preferences are restored: the persistent library panels reopen, where the
/// saved layout has them or floating, their libraries read again (a loaded one from its file); the
/// others closed with the app. A panel whose library can't be found reopens showing the library
/// as missing ([`missing_file`]) and stays in the preferences, and the status bar names its file.
pub fn restore(app: &mut VectorcraftApp) {
    let saved = std::mem::take(&mut app.ui.library_panels);
    let mut missing = vec![];
    for mut p in saved.into_iter().filter(|p| p.persistent).take(MAX_RESTORED) {
        let Some(k) = kind(&p.kind) else { continue };
        if (k.name)(app, &p.id).is_none()
            && let Some(path) = p.path.clone()
            // A file now in the user library folder loads as that User Defined library.
            && let Ok(id) = app.run(k.load, json!({ "path": path })).map(|r| r["library"].as_str().unwrap_or_default().to_string())
            && id != p.id
            && std::sync::Arc::make_mut(&mut app.ui.dock_workspace).rename(&p.panel(), panel_id(k.kind, &id)).is_ok()
        {
            p.id = id;
        }
        if (k.name)(app, &p.id).is_none() {
            missing.push(p.path.clone().unwrap_or_else(|| p.id.clone()));
        }
        app.ui.library_panels.push(p);
    }
    let kept: Vec<String> = app.ui.library_panels.iter().map(LibraryPanel::panel).collect();
    for panel in open_panels(&app.ui).into_iter().filter(|p| !kept.contains(p)) {
        close(app, &panel);
    }
    for panel in kept.into_iter().filter(|p| !is_open(&app.ui, p)).collect::<Vec<_>>() {
        if let Err(e) = float(app, &panel) {
            missing.push(format!("{panel} ({e})"));
        }
    }
    if !missing.is_empty() {
        let files = missing.join(", ");
        app.status(format!("Persistent libraries not found: {files}"));
    }
}

/// The submenu of the libraries menu that lists libraries of `category` (`None`: the top level).
pub(crate) fn submenu(category: &str) -> Option<&'static str> {
    match category {
        "gradients" => Some("Gradients"),
        "patterns" => Some("Patterns"),
        "user" => Some("User Defined"),
        "loaded" => Some("Other Libraries"),
        _ => None,
    }
}

/// Is library `id` built in? A library the user saved to the User Defined folder or loaded from a
/// file (`user/…`, `loaded/…` ids) is named by its file.
fn builtin_library(id: &str) -> bool {
    !(id.starts_with("user/") || id.starts_with("loaded/"))
}

/// Library `id`'s name as shown: a built-in library's in the UI language, any other's as it is.
pub(crate) fn library_name<'a>(id: &str, name: &'a str) -> &'a str {
    super::label_or_name(name, builtin_library(id))
}

/// The opener of library `K`'s libraries (`window.swatchLibrary {library}`): open library `library`
/// (an id or a name, or the id of a missing library's persistent or open panel, [`missing_name`])
/// in its own panel, floating, or bring its panel forward when it is open; `null` closes every
/// panel of `K`'s libraries. → {open, panel, name, count} (a missing library: its
/// [`missing_name`], count 0)
pub(crate) fn open_command<K: LibraryKind>(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let Some(key) = p.get("library").filter(|v| !v.is_null()) else {
        for panel in open_panels(&app.ui).into_iter().filter(|p| parse(p).is_some_and(|(k, _)| k.kind == K::KIND)) {
            app.run("window.panel.close", json!({ "panel": panel }))?;
        }
        return Ok(json!({ "open": null }));
    };
    let list = K::LIST;
    let key = key.as_str().ok_or_else(|| format!("`library` is a library id or name (see {list})"))?;
    let found = K::find(app, key).or_else(|| missing_name(app, kind_of::<K>(), key).map(|name| (key.to_string(), name, 0)));
    let (id, name, count) = found.ok_or_else(|| format!("no library `{key}` (see {list})"))?;
    let panel = panel_id(K::KIND, &id);
    // Brought forward, it is the one opened last.
    let at = app.ui.library_panels.iter().position(|q| q.kind == K::KIND && q.id == id);
    if let Some(s) = at.map(|i| app.ui.library_panels.remove(i)) {
        app.ui.library_panels.push(s);
    }
    settings_mut(app, kind_of::<K>(), &id);
    if is_open(&app.ui, &panel) {
        app.run("window.panel.activate", json!({ "panel": panel }))?;
    } else {
        float(app, &panel)?;
    }
    Ok(json!({"open": id, "panel": panel, "name": name, "count": count}))
}

/// Float library panel `panel`, as tall as its body in the kind's view, beside the window floated last.
pub(crate) fn float(app: &mut VectorcraftApp, panel: &str) -> Result<Value, String> {
    let large = parse(panel).is_some_and(|(k, _)| k.view == View::LargeThumb);
    let height = if large { 400.0 } else { 340.0 };
    let mut p = json!({ "panel": panel, "height": height });
    if let Some(at) = crate::panel_docking::cascaded_float_origin(app, height) {
        p["x"] = json!(at.x);
        p["y"] = json!(at.y);
    }
    app.run("window.panel.float", p)
}

/// `ui.libraryPanel {panel, library?, view?, showFind?, persistent?}`: show another library in an
/// open library panel (Previous / Next Library; one open in a panel of its own is brought forward
/// instead), and set a panel's view, Show Find Field and Persistent (also a closed persistent one's).
/// → its settings
pub(crate) fn command(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let given = p.get("panel").and_then(Value::as_str).unwrap_or_default();
    let (k, id) = parse(given).ok_or_else(|| format!("`{given}` isn't a library panel (see window.swatchLibrary)"))?;
    let mut id = id.to_string();
    let open = is_open(&app.ui, given);
    let closed = || format!("library panel `{given}` isn't open");
    if !open && !settings(&app.ui, k.kind, &id).is_some_and(|s| s.persistent) {
        return Err(closed());
    }
    let view = match p.get("view") {
        None | Some(Value::Null) => None,
        Some(v) => Some(
            serde_json::from_value::<View>(v.clone())
                .map_err(|_| "`view` is smallThumb, mediumThumb, largeThumb, smallList or largeList".to_string())?,
        ),
    };
    let show_find = crate::menus::opt_bool(p, "showFind")?;
    let persistent = crate::menus::opt_bool(p, "persistent")?;
    if let Some(key) = p.get("library").filter(|v| !v.is_null()) {
        let list = k.list;
        let key = key.as_str().ok_or_else(|| format!("`library` is a library id or name (see {list})"))?;
        let (next, ..) = (k.find)(app, key).ok_or_else(|| format!("no library `{key}` (see {list})"))?;
        // Only an open panel shows another library.
        if !open {
            return Err(closed());
        }
        let to = panel_id(k.kind, &next);
        if is_open(&app.ui, &to) {
            app.run("window.panel.activate", json!({ "panel": to }))?;
        } else if next != id {
            std::sync::Arc::make_mut(&mut app.ui.dock_workspace).rename(&given.to_string(), to.clone()).map_err(|e| e.to_string())?;
            app.ui.library_panels.retain(|q| !(q.kind == k.kind && q.id == next));
            let path = if next.starts_with("loaded/") { (k.path)(app, &next) } else { None };
            if let Some(s) = settings_mut(app, k, &id) {
                (s.id, s.path) = (next.clone(), path);
            }
        }
        id = next;
    }
    let s = settings_mut(app, k, &id).ok_or_else(closed)?;
    s.view = view.or(s.view);
    s.hide_find = show_find.map_or(s.hide_find, |show| !show);
    s.persistent = persistent.unwrap_or(s.persistent);
    let s = s.clone();
    Ok(json!({
        "panel": s.panel(),
        "kind": s.kind,
        "library": s.id,
        "view": s.view.unwrap_or(k.view),
        "showFind": !s.hide_find,
        "persistent": s.persistent,
        "open": is_open(&app.ui, &s.panel()),
    }))
}

/// `ui.libraryPanel.locate {panel, path?}`: Locate Library… of a library panel whose library can't
/// be found, Persistent or not (any other panel is an error). Load the library at `path`, else one
/// picked in an open dialog (the answer arrives on a later pass), and show it in open panel `panel`, which keeps its view, Show Find Field and
/// Persistent. When the file's library has a panel open already, `panel` closes and is forgotten,
/// and that panel is brought forward, made persistent when `panel` was. → what `ui.libraryPanel`
/// answers
pub(crate) fn locate(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let given = p.get("panel").and_then(Value::as_str).unwrap_or_default();
    let (k, id) = parse(given).ok_or_else(|| format!("`{given}` isn't a library panel (see window.swatchLibrary)"))?;
    if !is_open(&app.ui, given) {
        return Err(format!("library panel `{given}` isn't open"));
    }
    if (k.name)(app, id).is_some() {
        return Err(format!("library panel `{given}` isn't missing its library"));
    }
    let path = p.get("path").and_then(Value::as_str).map(str::to_string);
    let panel = given.to_string();
    pick_library_file(app, path, library_pick_filters(k.kind), move |app, path| located(app, &panel, &path))
}

/// Locate Library… answered with the file at `path` for missing library panel `given`.
fn located(app: &mut VectorcraftApp, given: &str, path: &str) -> Result<Value, String> {
    let (k, id) = parse(given).ok_or_else(|| format!("`{given}` isn't a library panel (see window.swatchLibrary)"))?;
    let r = app.run(k.load, json!({ "path": path }))?;
    let next = r["library"].as_str().unwrap_or_default().to_string();
    let to = panel_id(k.kind, &next);
    if to != given && is_open(&app.ui, &to) {
        let persistent = settings(&app.ui, k.kind, id).is_some_and(|s| s.persistent);
        app.run("window.panel.close", json!({ "panel": given }))?;
        app.ui.library_panels.retain(|q| q.panel() != given);
        app.run("window.panel.activate", json!({ "panel": to }))?;
        let mut params = json!({ "panel": to });
        if persistent {
            params["persistent"] = json!(true);
        }
        return command(app, &params);
    }
    command(app, &json!({ "panel": given, "library": next }))
}

/// Run `ui.libraryPanel` on panel `panel` with `params`.
fn set(app: &mut VectorcraftApp, panel: &str, mut params: Value) {
    params["panel"] = json!(panel);
    if let Err(e) = app.run("ui.libraryPanel", params) {
        app.status(e);
    }
}

/// Open-dialog filters for Other Library… / Locate Library… of library panels of kind `kind`
/// (symbol and brush libraries: any document).
pub(crate) fn library_pick_filters(kind: &str) -> Vec<(&'static str, &'static [&'static str])> {
    match kind {
        super::swatches::SwatchLibraries::KIND => fileio::library_open_filters("Swatch libraries", swatchlib::LIBRARY_PICK_EXTS).collect(),
        super::graphic_styles::GraphicStyleLibraries::KIND => fileio::library_open_filters("Graphic Style Libraries", &[STYLES_EXT]).collect(),
        _ => fileio::builtin_open_filters().collect(),
    }
}

/// Other Library… and Locate Library…: `then` loads the library file at `path`, else one picked in
/// an open dialog. The pick is asked of the app (`file_asks`) and answers on a later pass, which
/// runs `then` with the picked file; until then this returns what `file_asks::ask` returns.
pub(crate) fn pick_library_file(
    app: &mut VectorcraftApp,
    path: Option<String>,
    filters: Vec<(&'static str, &'static [&'static str])>,
    then: impl FnOnce(&mut VectorcraftApp, String) -> Result<Value, String> + 'static,
) -> Result<Value, String> {
    if let Some(path) = path {
        return then(app, path);
    }
    let request = filters.into_iter().fold(PickRequest::open("library"), |r, (name, exts)| r.filter(name, exts));
    Ok(file_asks::ask(app, request, Owner::Free, move |app, picked| match picked {
        Picked::Paths(paths) => match paths.into_iter().next() {
            Some(path) => then(app, path.to_string_lossy().into_owned()),
            None => Ok(Value::Null),
        },
        _ => Ok(Value::Null),
    }))
}

/// The User Defined library a Window menu slot (`prefix` and a number from 1) stands for, of
/// `libraries`.
pub(crate) fn user_slot(id: &str, prefix: &str, libraries: Vec<LibraryInfo>) -> Option<LibraryInfo> {
    let n: usize = id.strip_prefix(prefix)?.parse().ok()?;
    libraries.into_iter().filter(|l| l.category == "user").nth(n.checked_sub(1)?)
}

/// Test frames: the panel opened last, as the dock or a floating group draws its body.
#[cfg(test)]
pub(crate) fn show_window(app: &mut VectorcraftApp, ctx: &egui::Context) {
    let Some(panel) = last_open(&app.ui).map(LibraryPanel::panel) else { return };
    let area = egui::Area::new(egui::Id::unique("test-library-panel")).fixed_pos(egui::pos2(20.0, 20.0));
    area.show(ctx, |ui| {
        // A dock gives the panel a room of its own, which the browser scrolls in above the footer.
        ui.set_min_size(egui::vec2(320.0, 740.0));
        crate::dock::panel_body(app, ui, &panel);
    });
}

/// The saved [`View`] nearest to what the browser's toolbar picked: the inverse of [`View::preset`].
// PLAN: the zoom slider can set a zoom the five views do not name; it saves as the nearest of them.
fn saved_view(view: PresetView, zoom: f64) -> View {
    match (view == PresetView::List, zoom) {
        (false, z) if z < 7.0 / 24.0 => View::SmallThumb,
        (false, z) if z < 0.5 => View::MediumThumb,
        (false, _) => View::LargeThumb,
        (true, z) if z < 0.75 => View::SmallList,
        (true, _) => View::LargeList,
    }
}

/// The browser rows of library `id`'s `rows`, flat: a folder is a row of its own before its items, which a click
/// selects as it selects an item (production's folder tiles neither opened nor closed).
fn browser_rows<K: LibraryKind>(panel: &str, id: &str, rows: &[Row<K::Item>]) -> Vec<PresetRow<LibraryKey>> {
    rows.iter()
        .map(|r| {
            let label = K::label(id, r.name);
            let key = LibraryKey { panel: panel.into(), name: r.name.into(), folder: r.item.is_none() };
            match r.item {
                Some(item) => {
                    let mut row = K::row(id, item, &label);
                    row.key = key;
                    row
                }
                None => PresetRow::item(key, label.into_owned(), 0).icon(IconId::new("dc-folder")),
            }
        })
        .collect()
}

/// What a library's browser shows: its rows, the items their thumbnails paint and what those paint with.
struct Shown<'a, K: LibraryKind> {
    rows: &'a [PresetRow<LibraryKey>],
    items: HashMap<&'a str, &'a K::Item>,
    thumbs: K::Thumbs,
}

/// By the rows (each names its library and item, whose look a library keeps) and what the thumbnails paint with.
impl<K: LibraryKind> MemoProps for Shown<'_, K> {
    fn key(&self, key: &mut MemoKey) {
        for row in self.rows {
            row.key.key(key);
            row.label.key(key);
            row.depth.key(key);
            (row.kind == PresetKind::Item).key(key);
        }
        self.rows.len().key(key);
        self.thumbs.key(key);
    }
}

/// The selected items and folders of panel `panel`.
fn selected(app: &VectorcraftApp, panel: &str) -> Selection<LibraryKey> {
    app.ui.library_selection.get(panel).cloned().unwrap_or_default()
}

/// The browser of library `id`, the footer and, for a missing library, [`missing`].
fn body<K: LibraryKind>(app: &mut VectorcraftApp, ui: &mut Ui, id: &str) {
    let Some((name, lib)) = K::get(app, id) else {
        missing::<K>(app, ui, id);
        return;
    };
    let panel = panel_id(K::KIND, id);
    let (view, hide_find) = settings(&app.ui, K::KIND, id).map_or((K::VIEW, false), |s| (s.view.unwrap_or(K::VIEW), s.hide_find));
    let flat = K::rows(&lib, id);
    let rows = browser_rows::<K>(&panel, id, &flat);
    let items = flat.iter().filter_map(|r| r.item.map(|i| (r.name, i))).collect();
    let fns = PresetFns::new(|shown: &Shown<'_, K>, _| shown.rows.to_vec()).thumb(|shown, cx, rect, info| {
        if let Some(item) = shown.items.get(info.key.name.as_str()) {
            K::thumb(&shown.thumbs, cx, rect, item);
        }
    });
    let source = PresetSource::new(Shown::<K> { rows: &rows, items, thumbs: K::thumbs(app, &lib) }, fns);
    let selection = selected(app, &panel);
    // The search text and the open groups are the browser's; the view is the panel's setting.
    let state_id = egui::Id::unique(("library-panel-state", &panel));
    let mut state: PresetState<LibraryKey> = ui.data(|d| d.get_temp(state_id)).unwrap_or_default();
    (state.view, state.zoom) = view.preset();
    let title = library_name(id, &name).to_string();
    // The footer stays under the browser, which scrolls in the room left above it.
    let footer_out = RefCell::new(None);
    let any = !selection.items.is_empty();
    let mut shown = Panel::new(title.clone())
        .collection_body()
        .footer(|ui, _| *footer_out.borrow_mut() = Some(footer_bar::<K>(any).show_footer(ui)))
        .show(ui, |ui, _| {
            PresetBrowser::new(title.clone(), Bind::bound(&mut state), &source)
                .toolbar(true)
                .search(!hide_find)
                .select(SelectMode::Multi)
                .selection(Bind::given(selection.clone()))
                .empty(tl!("This library is empty."))
                .row_menu(PresetRowMenu::Off)
                .drags(K::drags())
                .show(ui)
        });
    let events = shown.settle(|_, out| std::mem::take(&mut out.events)).unwrap_or_default();
    let mut chosen = selection;
    let mut clicked = None;
    for event in events {
        match event {
            PresetEvent::Selection(change) => chosen.apply(change),
            // A plain click on an item also adds and applies it; folders and modifier clicks only select.
            PresetEvent::Select { key, mods, .. } if !key.folder && !mods.shift && !mods.cmd => clicked = Some(key.name),
            _ => {}
        }
    }
    let picked = saved_view(state.view, state.zoom);
    ui.data_mut(|d| d.insert_temp(state_id, state));
    if picked != view {
        set(app, &panel, json!({ "view": picked }));
    }
    let names: Vec<String> = chosen.items.iter().map(|k| k.name.clone()).collect();
    app.ui.library_selection.insert(panel.clone(), chosen);
    if let Some(name) = clicked {
        K::click(app, ui, id, &name);
    }
    if let Some(out) = footer_out.into_inner() {
        footer::<K>(app, ui, id, names, out);
    }
}

/// The panel's footer: the libraries menu, previous / next library, and Add to … , on while `any` item or folder is
/// selected.
fn footer_bar<K: LibraryKind>(any: bool) -> FooterBar {
    let icon = |name: &'static str| IconSpec::svg(IconId::new(name));
    FooterBar::actions([
        FooterAction::new(icon("library"), K::MENU).popup(PopupKind::Menu),
        FooterAction::new(icon("chevron-left"), tl!("Previous Library")),
        FooterAction::new(icon("chevron-right"), tl!("Next Library")),
        FooterAction::new(icon("dc-new-item"), K::ADD).enabled(any),
    ])
}

/// What the footer `out` reported: the libraries menu opened, a library stepped to, or Add to … for the selection
/// `names`.
fn footer<K: LibraryKind>(app: &mut VectorcraftApp, ui: &mut Ui, id: &str, names: Vec<String>, out: FooterOutput) {
    if let Some(menu) = out.responses.first() {
        Menu::new(K::MENU).opened_by(menu, Trigger::Click).show(ui, |ui, _| library_menu::<K>(app, ui));
    }
    match out.picked {
        Some(1) => step::<K>(app, id, -1),
        Some(2) => step::<K>(app, id, 1),
        Some(3) => K::add(app, id, names),
        _ => {}
    }
}

/// The body of the panel of library `id` when the library is missing ([`missing_file`]): its file,
/// and Locate Library….
fn missing<K: LibraryKind>(app: &mut VectorcraftApp, ui: &mut Ui, id: &str) {
    let Some(file) = missing_file(app, kind_of::<K>(), id) else { return };
    let text = crate::i18n::fmt(tl!("Missing: {path}"), &[("path", &file)]);
    // PLAN: `NoticeAction` has no disabled state; where the app has no picker, Locate reports it in the status bar.
    let out = Notice::new(Tone::Neutral, text).layout(NoticeLayout::Centred).action(NoticeAction::new(tl!("Locate Library…"))).show(ui);
    if out.action == Some(0)
        && let Err(e) = app.run("ui.libraryPanel.locate", json!({ "panel": panel_id(K::KIND, id) }))
    {
        app.status(e);
    }
}

/// The library `step` places after (or before, when negative) `id` in menu order, wrapping.
fn neighbour<K: LibraryKind>(app: &VectorcraftApp, id: &str, step: isize) -> Option<String> {
    let all = K::list(app);
    let i = all.iter().position(|l| l.id == id)? as isize;
    let n = all.len() as isize;
    all.into_iter().nth((i + step).rem_euclid(n) as usize).map(|l| l.id)
}

/// Show the library `step` places after (or before) `id` in its panel.
fn step<K: LibraryKind>(app: &mut VectorcraftApp, id: &str, step: isize) {
    if let Some(next) = neighbour::<K>(app, id, step) {
        set(app, &panel_id(K::KIND, id), json!({ "library": next }));
    }
}

/// Open library `id` in its panel.
fn open<K: LibraryKind>(app: &mut VectorcraftApp, id: &str) {
    if let Err(e) = app.run(K::OPEN, json!({ "library": id })) {
        app.status(e);
    }
}

/// One row of a library panel's menu with its label and state (`checked` for the rows that check), or
/// a separator.
enum MenuEntry {
    Row { command: CommandRef, label: &'static str, enabled: bool, checked: Option<bool> },
    Separator,
}

/// The rows of library panel `panel`'s menu with their states: Add to …, the views, Show Find Field,
/// Previous and Next Library (dimmed for a missing library), Persistent and Close Library.
fn menu_entries(app: &VectorcraftApp, panel: &str) -> Vec<MenuEntry> {
    let Some((k, id)) = parse(panel) else { return vec![] };
    let s = settings(&app.ui, k.kind, id);
    let (view, hide_find, persistent) = s.map_or((k.view, false, false), |s| (s.view.unwrap_or(k.view), s.hide_find, s.persistent));
    let row = |command: &str, mut params: Value, label: &'static str, enabled: bool, checked: Option<bool>| {
        params["panel"] = json!(panel);
        MenuEntry::Row { command: CommandRef::with_params(command, params), label, enabled, checked }
    };
    let has_selection = !selected(app, panel).items.is_empty();
    let mut out = vec![row("ui.libraryPanel.add", json!({}), k.add_label, has_selection, None), MenuEntry::Separator];
    out.extend(View::ALL.iter().map(|&(v, label)| row("ui.libraryPanel", json!({ "view": v }), label, true, Some(v == view))));
    out.extend([
        MenuEntry::Separator,
        row("ui.libraryPanel", json!({ "showFind": hide_find }), "Show Find Field", true, Some(!hide_find)),
        MenuEntry::Separator,
    ]);
    for (step, label) in [(-1, "Previous Library"), (1, "Next Library")] {
        let next = (k.neighbour)(app, id, step);
        let params = next.as_ref().map_or_else(|| json!({}), |next| json!({ "library": next }));
        out.push(row("ui.libraryPanel", params, label, next.is_some(), None));
    }
    out.push(row("ui.libraryPanel", json!({ "persistent": !persistent }), "Persistent", true, Some(persistent)));
    out.push(MenuEntry::Row {
        command: CommandRef::with_params("window.panel.close", json!({ "panel": panel })),
        label: "Close Library",
        enabled: true,
        checked: None,
    });
    out
}

/// Library panel `panel`'s (≡) menu rows.
pub(crate) fn menu_rows(app: &VectorcraftApp, panel: &str) -> Vec<PanelRow> {
    menu_entries(app, panel)
        .into_iter()
        .map(|e| match e {
            MenuEntry::Row { command, .. } => PanelRow::Command(command),
            MenuEntry::Separator => PanelRow::Separator,
        })
        .collect()
}

/// Gives library panel `panel`'s [`menu_rows`] their names, enabled and checked states: one command
/// names several rows with different params.
pub(crate) fn menu_states(app: &VectorcraftApp, panel: &str, states: &mut CommandStates) {
    for e in menu_entries(app, panel) {
        if let MenuEntry::Row { command, label, enabled, checked } = e {
            let mut state = CommandState::new().title(tl!(label)).enabled(enabled);
            if let Some(checked) = checked {
                state = state.checked(checked);
            }
            states.set(command, state);
        }
    }
}

/// Runs a library panel command that the engine does not know: `ui.libraryPanel.add {panel}` adds the
/// panel's selection to the document.
pub(crate) fn run(app: &mut VectorcraftApp, id: &str, params: &Value) -> Result<Value, String> {
    if id != "ui.libraryPanel.add" {
        return Err(format!("`{id}` isn't a library panel command"));
    }
    let given = params.get("panel").and_then(Value::as_str).unwrap_or_default();
    let (k, lib) = parse(given).ok_or_else(|| format!("`{given}` isn't a library panel (see window.swatchLibrary)"))?;
    let names: Vec<String> = selected(app, given).items.into_iter().map(|key| key.name).collect();
    (k.add)(app, lib, names);
    Ok(Value::Null)
}

/// The libraries `libs` as menus group them: the top-level ones, then each submenu with its libraries,
/// in the order they first appear.
fn grouped(libs: &[LibraryRef]) -> (Vec<&LibraryRef>, Vec<(&'static str, Vec<&LibraryRef>)>) {
    let top = libs.iter().filter(|l| l.submenu.is_none()).collect();
    let mut subs: Vec<(&'static str, Vec<&LibraryRef>)> = vec![];
    for (l, s) in libs.iter().filter_map(|l| l.submenu.map(|s| (l, s))) {
        match subs.iter_mut().find(|(name, _)| *name == s) {
            Some((_, members)) => members.push(l),
            None => subs.push((s, vec![l])),
        }
    }
    (top, subs)
}

/// The "Open … Library" rows of a panel menu of kind `K`: every library by submenu, each a
/// `K::OPEN {library}` command row.
// PLAN: a command row takes its label from the command; the library's name needs a label override (ch.P8).
pub(crate) fn library_rows<K: LibraryKind>(app: &VectorcraftApp) -> Vec<PanelRow> {
    let libs = K::list(app);
    let (top, subs) = grouped(&libs);
    let open = |l: &&LibraryRef| PanelRow::Command(CommandRef::with_params(K::OPEN, json!({ "library": l.id })));
    let mut rows: Vec<PanelRow> = top.iter().map(open).collect();
    rows.extend(subs.iter().map(|(name, members)| PanelRow::Submenu(tl!(name).into(), members.iter().map(open).collect())));
    rows
}

/// The libraries menu (the footer's library button, the Swatches panel's): every library by submenu,
/// the open ones checked, between `K`'s head and tail items. Shown in the body of a `Menu`.
pub(crate) fn library_menu<K: LibraryKind>(app: &mut VectorcraftApp, ui: &mut Ui) {
    K::menu_head(app, ui);
    let shown = open_panels(&app.ui);
    if let Some(id) = library_items(ui, &K::list(app), &|id| shown.contains(&panel_id(K::KIND, id))) {
        open::<K>(app, &id);
    }
    K::menu_tail(app, ui);
}

/// Menu rows for `libs`: the top-level ones, then a submenu per [`LibraryRef::submenu`], those
/// `checked` says checked. Returns the id of the one clicked.
pub(crate) fn library_items(ui: &mut Ui, libs: &[LibraryRef], checked: &dyn Fn(&str) -> bool) -> Option<String> {
    let mut chosen = None;
    let mut item = |ui: &mut Ui, l: &LibraryRef| {
        if MenuItem::new(library_name(&l.id, &l.name).to_string()).check(checked(&l.id)).show(ui).ran {
            chosen = Some(l.id.clone());
        }
    };
    let (top, subs) = grouped(libs);
    for l in top {
        item(ui, l);
    }
    for (name, members) in subs {
        Menu::submenu_row(ui, tl!(name), |ui, _| {
            for l in &members {
                item(ui, l);
            }
        });
    }
    chosen
}

#[cfg(test)]
pub(crate) mod tests;
