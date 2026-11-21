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
use std::collections::HashMap;

use craft_behavior::IconId;
use craft_theme::variant::Tone;
use craft_ui::contract::{Bind, SelectMode, Selection, SelectionModel, Trigger};
use craft_ui::menu::{Menu, MenuItem, PanelRow};
use craft_ui::notices::{Notice, NoticeAction, NoticeLayout};
use craft_ui::patterns::{PresetBrowser, PresetEvent, PresetRow, PresetRowMenu, PresetState, PresetView};
use craft_ui::primitives::{ContentPainterCx, IconSpec};
use craft_ui::runtime::access::PopupKind;
use craft_ui::runtime::commands::CommandRef;
use craft_ui::sections::{FooterAction, FooterBar};
use egui::{Rect, Ui};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use vectorcraft_doc::style_libs::STYLES_EXT;
use vectorcraft_engine::cmd::fileio;
use vectorcraft_engine::cmd::swatchlib::{self, LibraryInfo};

use super::swatches::View;
use crate::VectorcraftApp;
use crate::file_asks::{self, Owner, PickRequest, Picked};
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
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
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
    type Item;
    /// Every library, in menu order.
    fn list(app: &VectorcraftApp) -> Vec<LibraryRef>;
    /// Library `id`: its name and contents.
    fn get(app: &VectorcraftApp, id: &str) -> Option<(String, Self::Lib)>;
    /// Library `key` (an id or a name).
    fn find(app: &VectorcraftApp, key: &str) -> Found;
    /// The file library `id` was read from, if any.
    fn path(app: &VectorcraftApp, id: &str) -> Option<String>;
    /// The rows of library `id` matching `query` (lower case; empty: all). The panel asks for all
    /// of them: the browser's search filters what it shows.
    fn rows<'a>(lib: &'a Self::Lib, id: &str, query: &str) -> Vec<Row<'a, Self::Item>>;
    /// Row `name` of library `id` as its list row and tooltips show it (by default as it is). The
    /// row keeps `name` for its tile, selection, clicks, drags and the commands they run.
    fn label<'a>(_id: &str, name: &'a str) -> Cow<'a, str> {
        Cow::Borrowed(name)
    }
    /// The browser row of `item`, named `label`, with the tooltip that describes it. The panel
    /// sets the row's key.
    fn row(id: &str, item: &Self::Item, label: &str) -> PresetRow<LibraryKey>;
    /// An item's thumbnail in `rect`.
    fn thumb(app: &VectorcraftApp, cx: &ContentPainterCx, rect: Rect, item: &Self::Item);
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
    // PLAN: assumed `Workspace::<String>::contains(&String)` (ch.P1)
    ui.dock_workspace.contains(&panel.to_string())
}

/// The open library panels.
fn open_panels(ui: &UiState) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    // PLAN: assumed `Workspace::<String>::panels()` over `&String` (ch.P1)
    for p in ui.dock_workspace.panels() {
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
            // PLAN: assumed `Workspace::<String>::rename(&String, &String) -> Result<_, impl Display>` (ch.K27)
            && app.ui.dock_workspace.rename(&p.panel(), &panel_id(k.kind, &id)).is_ok()
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

/// Float library panel `panel`, as tall as its body in the kind's view.
pub(crate) fn float(app: &mut VectorcraftApp, panel: &str) -> Result<Value, String> {
    let large = parse(panel).is_some_and(|(k, _)| k.view == View::LargeThumb);
    app.run("window.panel.float", json!({ "panel": panel, "height": if large { 400 } else { 340 } }))
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
            app.ui.dock_workspace.rename(&given.to_string(), &to).map_err(|e| e.to_string())?;
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
/// picked in an open dialog (the answer arrives on a later pass and runs this command again with
/// its path), and show it in open panel `panel`, which keeps its view, Show Find Field and
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
    let filters = library_pick_filters(k.kind);
    let then = ("ui.libraryPanel.locate", json!({ "panel": given }));
    let Some(path) = pick_library_file(app, path, filters, then)? else { return Ok(Value::Null) };
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

/// Other Library…: the library file at `path`, else one picked in an open dialog. The pick is asked
/// of the app (`file_asks`) and answers on a later pass: `None` then, and when it answers `then`
/// (a command and its params) runs again with `path` added. A picker the app lacks reports it in
/// the status bar.
// PLAN: `then` is a fourth parameter (the sheet has three): the later answer must say which command to continue.
pub(crate) fn pick_library_file(
    app: &mut VectorcraftApp,
    path: Option<String>,
    filters: Vec<(&'static str, &'static [&'static str])>,
    then: (&'static str, Value),
) -> Result<Option<String>, String> {
    if path.is_some() {
        return Ok(path);
    }
    // PLAN: assumed `file_asks::ask(&mut VectorcraftApp, PickRequest, Owner, impl FnOnce(&mut VectorcraftApp, Picked) + 'static)` (fl.A3)
    let request = PickRequest::open(Default::default()).filters(filters);
    file_asks::ask(app, request, Owner::Free, move |app, picked| match picked {
        Picked::Path(path) => {
            let (command, mut params) = then;
            params["path"] = json!(path);
            if let Err(e) = app.run(command, params) {
                app.status(e);
            }
        }
        Picked::Unavailable => app.status("This app can't open a file dialog."),
        _ => {}
    });
    Ok(None)
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
    let area = egui::Area::new(egui::Id::new("test-library-panel")).fixed_pos(egui::pos2(20.0, 20.0));
    area.show(ctx, |ui| crate::dock::panel_body(app, ui, &panel));
}

/// The browser's view and thumbnail zoom for a saved [`View`].
fn preset_view(view: View) -> (PresetView, f64) {
    match view {
        View::SmallThumb => (PresetView::Grid, 0.5),
        View::MediumThumb => (PresetView::Grid, 0.75),
        View::LargeThumb => (PresetView::Grid, 1.0),
        View::SmallList => (PresetView::List, 0.5),
        View::LargeList => (PresetView::List, 1.0),
    }
}

/// The saved [`View`] nearest to what the browser's toolbar picked: the inverse of [`preset_view`].
// PLAN: the zoom slider can set a zoom the five views do not name; it saves as the nearest of them.
fn saved_view(view: PresetView, zoom: f64) -> View {
    match (view == PresetView::List, zoom) {
        (false, z) if z < 0.625 => View::SmallThumb,
        (false, z) if z < 0.875 => View::MediumThumb,
        (false, _) => View::LargeThumb,
        (true, z) if z < 0.75 => View::SmallList,
        (true, _) => View::LargeList,
    }
}

/// The browser rows of library `id`'s `rows`: a folder is a group, and the items after it are inside it.
fn browser_rows<K: LibraryKind>(panel: &str, id: &str, rows: &[Row<K::Item>]) -> Vec<PresetRow<LibraryKey>> {
    let mut depth = 0;
    rows.iter()
        .map(|r| {
            let label = K::label(id, r.name);
            let key = LibraryKey { panel: panel.into(), name: r.name.into(), folder: r.item.is_none() };
            match r.item {
                Some(item) => {
                    let mut row = K::row(id, item, &label);
                    row.key = key;
                    row.depth = depth;
                    row
                }
                None => {
                    depth = 1;
                    PresetRow::group(key, label.into_owned(), 0)
                }
            }
        })
        .collect()
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
    let flat = K::rows(&lib, id, "");
    let rows = browser_rows::<K>(&panel, id, &flat);
    let items: HashMap<&str, &K::Item> = flat.iter().filter_map(|r| r.item.map(|i| (r.name, i))).collect();
    let selection = selected(app, &panel);
    // The search text and the open groups are the browser's; the view is the panel's setting.
    let state_id = egui::Id::new(("library-panel-state", &panel));
    let mut state: PresetState<LibraryKey> = ui.data(|d| d.get_temp(state_id)).unwrap_or_default();
    (state.view, state.zoom) = preset_view(view);
    let shown = &*app;
    let out = PresetBrowser::new(library_name(id, &name).to_string(), Bind::bound(&mut state), &rows)
        .toolbar(true)
        .search(!hide_find)
        .select(SelectMode::Multi)
        .selection(Bind::given(selection.clone()))
        .thumb(|cx, rect, info| {
            if let Some(item) = items.get(info.key.name.as_str()) {
                K::thumb(shown, cx, rect, item);
            }
        })
        .empty(tl!("This library is empty."))
        .row_menu(PresetRowMenu::Off)
        .show(ui);
    let mut chosen = selection;
    let mut clicked = None;
    for event in out.events {
        match event {
            PresetEvent::Selection(change) => chosen.apply(change),
            // A plain click on an item also adds and applies it; folders and modifier clicks only select.
            PresetEvent::Select { key, mods } if !key.folder && !mods.shift && !mods.cmd => clicked = Some(key.name),
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
    footer::<K>(app, ui, id, names);
}

/// The panel's footer: the libraries menu, previous / next library, and Add to … for the selection `names`.
fn footer<K: LibraryKind>(app: &mut VectorcraftApp, ui: &mut Ui, id: &str, names: Vec<String>) {
    let icon = |name: &'static str| IconSpec::svg(IconId::new(name));
    // PLAN: assumed `FooterBar::actions(..).show(ui) -> FooterOutput` (ui-panels footer)
    let out = FooterBar::actions([
        FooterAction::new(icon("library"), K::MENU).popup(PopupKind::Menu),
        FooterAction::new(icon("chevron-left"), tl!("Previous Library")),
        FooterAction::new(icon("chevron-right"), tl!("Next Library")),
        FooterAction::new(icon("dc-new-item"), K::ADD).enabled(!names.is_empty()),
    ])
    .show(ui);
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

/// One row of a library panel's menu with its state, or a separator.
enum MenuEntry {
    Row { command: CommandRef, enabled: bool, checked: bool },
    Separator,
}

/// The rows of library panel `panel`'s menu with their states: Add to …, the views, Show Find Field,
/// Previous and Next Library (dimmed for a missing library), Persistent and Close Library.
fn menu_entries(app: &VectorcraftApp, panel: &str) -> Vec<MenuEntry> {
    let Some((k, id)) = parse(panel) else { return vec![] };
    let s = settings(&app.ui, k.kind, id);
    let (view, hide_find, persistent) = s.map_or((k.view, false, false), |s| (s.view.unwrap_or(k.view), s.hide_find, s.persistent));
    let row = |id: &str, params: Value, enabled: bool, checked: bool| {
        let mut params = params;
        params["panel"] = json!(panel);
        MenuEntry::Row { command: CommandRef::with_params(id, params), enabled, checked }
    };
    let has_selection = !selected(app, panel).items.is_empty();
    let mut out = vec![row("ui.libraryPanel.add", json!({}), has_selection, false), MenuEntry::Separator];
    out.extend(View::ALL.iter().map(|&(v, _)| row("ui.libraryPanel", json!({ "view": v }), true, v == view)));
    out.extend([MenuEntry::Separator, row("ui.libraryPanel", json!({ "showFind": hide_find }), true, !hide_find), MenuEntry::Separator]);
    for step in [-1, 1] {
        let next = (k.neighbour)(app, id, step);
        let params = next.as_ref().map_or_else(|| json!({}), |next| json!({ "library": next }));
        out.push(row("ui.libraryPanel", params, next.is_some(), false));
    }
    out.push(row("ui.libraryPanel", json!({ "persistent": !persistent }), true, persistent));
    out.push(MenuEntry::Row { command: CommandRef::with_params("window.panel.close", json!({ "panel": panel })), enabled: true, checked: false });
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

/// Fills the enabled and checked state of library panel `panel`'s [`menu_rows`].
pub(crate) fn menu_states(app: &VectorcraftApp, panel: &str, states: &mut crate::panels::PanelMenuStates) {
    for e in menu_entries(app, panel) {
        if let MenuEntry::Row { command, enabled, checked } = e {
            // PLAN: assumed `PanelMenuStates::set(&CommandRef, enabled, checked)` (ch.P8 / ty.A15)
            states.set(&command, enabled, checked);
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
mod tests;

@@@ EDIT crates/ui-egui/src/panels/library_panel/tests.rs
<<<<<<< SEARCH
fn context() -> egui::Context {
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    ctx
}

/// One frame of the library panel with `events` and `modifiers` held.
fn frame(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<Event>, modifiers: Modifiers, time: f64) {
    let screen_rect = Some(Rect::from_min_size(Pos2::ZERO, vec2(1200.0, 800.0)));
    let events = std::iter::once(Event::ModifiersChanged(modifiers)).chain(events).collect();
    let input = egui::RawInput { events, time: Some(time), screen_rect, ..Default::default() };
    let mut out = ctx.run_ui(input, |ui| show_window(app, ui.ctx()));
    out.textures_delta.clear();
}

/// The panel opened last.
fn shown(app: &VectorcraftApp) -> String {
    last_open(&app.ui).expect("a library panel is open").panel()
}

/// Click the tile or row of `name` in the panel opened last with `modifiers` (frames from
/// `time` to `time + 0.5`; a new window takes a sizing frame first).
fn click(app: &mut VectorcraftApp, ctx: &egui::Context, name: &str, modifiers: Modifiers, time: f64) {
    frame(app, ctx, vec![], Modifiers::NONE, time);
    frame(app, ctx, vec![], Modifiers::NONE, time + 0.25);
    let at = ctx.read_response(tile_id(&shown(app), name)).unwrap_or_else(|| panic!("no tile for {name}")).rect.center();
    let button = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers };
    frame(app, ctx, vec![Event::PointerMoved(at), button(true), button(false)], modifiers, time + 0.5);
}
=======
/// A context whose frames report their accessibility tree, so rows are found by their names.
fn context() -> egui::Context {
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    ctx.enable_accesskit();
    ctx
}

/// The panel opened last.
fn shown(app: &VectorcraftApp) -> String {
    last_open(&app.ui).expect("a library panel is open").panel()
}

/// The names of the items and folders selected in panel `panel`.
fn selected_names(app: &VectorcraftApp, panel: &str) -> Vec<String> {
    app.ui.library_selection.get(panel).map(|s| s.items.iter().map(|k| k.name.clone()).collect()).unwrap_or_default()
}

/// Click the control of the panel opened last that screen readers name `name` (a row or a footer
// PLAN: assumed to be replaced by the framework pointer driver (`Fixture`) once the harness lands here.
/// action) with `modifiers` held; `_time` is kept for the callers' spacing of clicks.
fn click(app: &mut VectorcraftApp, ctx: &egui::Context, name: &str, modifiers: Modifiers, _time: f64) {
    let panel = shown(app);
    let draw = |app: &mut VectorcraftApp, ui: &mut Ui| show(app, ui, &panel);
    let (_, at) = control(app, ctx, &draw, name);
    let button = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers };
    draw_frame(app, ctx, &draw, vec![Event::ModifiersChanged(modifiers), Event::PointerMoved(at)]);
    draw_frame(app, ctx, &draw, vec![button(true), button(false)]);
    draw_frame(app, ctx, &draw, vec![Event::ModifiersChanged(Modifiers::NONE)]);
}

/// Set the fields of the hosted save dialog that `app` shows and press its OK.
fn confirm_save(app: &mut VectorcraftApp, fields: &[(&str, Value)]) {
    // PLAN: assumed hosted-dialog driver of appearance-w18: `host::{set_json_field, confirm_by_id, pump}`.
    let id = crate::dialogs::host::open_dialog_id(app).expect("a save dialog is open");
    for (name, value) in fields {
        crate::dialogs::host::set_json_field(app, &id, name, value.clone());
    }
    crate::dialogs::host::confirm_by_id(app, &id);
    crate::dialogs::host::pump(app);
}
>>>>>>> REPLACE
<<<<<<< SEARCH
    click(&mut app, &ctx, "Clay 1", Modifiers::COMMAND, 0.0);
    click(&mut app, &ctx, "Ochre", Modifiers::NONE, 2.0);
    assert_eq!(selection(&ctx, "library:swatches:earth-tones"), ["Ochre"], "a folder click selects its group");
    click(&mut app, &ctx, "Clay 3", Modifiers::COMMAND, 4.0);
    assert!(app.session.doc().unwrap().doc.swatch("Clay 3").is_none(), "modifier clicks only select");
    let sel = selection(&ctx, "library:swatches:earth-tones");
    SwatchLibraries::add(&mut app, "earth-tones", sel);
    let d = &app.session.doc().unwrap().doc;
    assert_eq!(d.swatch_groups.iter().find(|g| g.name == "Ochre").map(|g| g.swatches.len()), Some(6));
    assert!(d.swatches.iter().any(|w| w.name == "Clay 3"));
    // The selection belongs to its library's panel.
    app.run("window.swatchLibrary", json!({"library": "brights"})).unwrap();
    assert!(selection(&ctx, "library:swatches:brights").is_empty());
}

#[test]
fn a_plain_click_sets_where_a_shift_click_range_starts() {
    let mut app = app();
    let ctx = context();
    app.run("window.swatchLibrary", json!({"library": "pastels"})).unwrap();
    let (_, lib) = SwatchLibraries::get(&app, "pastels").unwrap();
    let order: Vec<String> = SwatchLibraries::rows(&lib, "pastels", "").iter().map(|r| r.name.to_string()).collect();
    click(&mut app, &ctx, &order[1], Modifiers::NONE, 0.0);
    click(&mut app, &ctx, &order[4], Modifiers::SHIFT, 2.0);
    assert_eq!(selection(&ctx, "library:swatches:pastels"), order[1..=4], "the range starts at the plain click");
    // A plain click after a Cmd/Ctrl-click moves the anchor from the Cmd/Ctrl-clicked item.
    click(&mut app, &ctx, &order[8], Modifiers::COMMAND, 4.0);
    click(&mut app, &ctx, &order[2], Modifiers::NONE, 6.0);
    click(&mut app, &ctx, &order[3], Modifiers::SHIFT, 8.0);
    assert_eq!(selection(&ctx, "library:swatches:pastels"), order[2..=3]);
    let d = &app.session.doc().unwrap().doc;
    assert!(d.swatch(&order[2]).is_some(), "a plain click still adds the swatch");
    assert!(d.swatch(&order[3]).is_none(), "a Shift-click only selects");
}

#[test]
fn find_filters_list_view_and_arrows_step_through_the_libraries() {
    let mut app = app();
    let ctx = context();
    app.run("window.swatchLibrary", json!({"library": "web-safe-216"})).unwrap();
    let panel = "library:swatches:web-safe-216";
    app.run("ui.libraryPanel", json!({"panel": panel, "view": "smallList"})).unwrap();
    ctx.data_mut(|d| d.insert_temp(egui::Id::unique(key(panel, "find")), "#FF00".to_string()));
    frame(&mut app, &ctx, vec![], Modifiers::NONE, 0.0);
    frame(&mut app, &ctx, vec![], Modifiers::NONE, 0.5);
    assert!(ctx.read_response(tile_id(panel, "#FF0033")).is_some());
    assert!(ctx.read_response(tile_id(panel, "#00FF00")).is_none());
    let first = SWATCH_IDS[0];
    let last = SwatchLibraries::list(&app).pop().unwrap().id;
    assert_eq!(last, "optical-effects", "gradient and then pattern libraries come after the swatch libraries");
    assert_eq!(neighbour::<SwatchLibraries>(&app, "radial-glows", 1).as_deref(), Some("geometric-patterns"));
    assert_eq!(neighbour::<SwatchLibraries>(&app, "geometric-patterns", 1).as_deref(), Some("grit-textures"));
    assert_eq!(neighbour::<SwatchLibraries>(&app, "grit-textures", 1).as_deref(), Some("optical-effects"));
    assert_eq!(neighbour::<SwatchLibraries>(&app, first, -1), Some(last), "wraps around");
    assert_eq!(neighbour::<SwatchLibraries>(&app, "web-safe-216", 1).as_deref(), Some(SWATCH_IDS[1]));
}
=======
    let panel = "library:swatches:earth-tones";
    click(&mut app, &ctx, "Clay 1", Modifiers::COMMAND, 0.0);
    click(&mut app, &ctx, "Ochre", Modifiers::NONE, 2.0);
    assert_eq!(selected_names(&app, panel), ["Ochre"], "a folder click selects its group");
    click(&mut app, &ctx, "Clay 3", Modifiers::COMMAND, 4.0);
    assert!(app.session.doc().unwrap().doc.swatch("Clay 3").is_none(), "modifier clicks only select");
    assert_eq!(selected_names(&app, panel), ["Ochre", "Clay 3"]);
    // Add to Swatches adds what the browser selected.
    click(&mut app, &ctx, "Add to Swatches", Modifiers::NONE, 6.0);
    let d = &app.session.doc().unwrap().doc;
    assert_eq!(d.swatch_groups.iter().find(|g| g.name == "Ochre").map(|g| g.swatches.len()), Some(6));
    assert!(d.swatches.iter().any(|w| w.name == "Clay 3"));
    // The selection belongs to its library's panel.
    app.run("window.swatchLibrary", json!({"library": "brights"})).unwrap();
    assert!(selected_names(&app, "library:swatches:brights").is_empty());
}
>>>>>>> REPLACE
<<<<<<< SEARCH
#[test]
fn the_window_submenu_lists_every_library() {
    let app = app();
    let ids: Vec<Value> = crate::menus::menu_entries(&app)
        .into_iter()
        .filter(|e| e.path == ["Window", "Swatch Libraries"] && e.command.as_deref() == Some("window.swatchLibrary"))
        .map(|e| e.params["library"].clone())
        .collect();
    let all = vectorcraft_engine::cmd::swatchlib::libraries(&app.session);
    let listed = |category: &str| all.iter().filter(|l| l.category == category).map(|l| json!(l.id)).collect::<Vec<_>>();
    assert_eq!(ids, listed("builtIn"));
    assert_eq!(ids, SWATCH_IDS.map(|i| json!(i)));
    let gradients: Vec<Value> = crate::menus::menu_entries(&app)
        .into_iter()
        .filter(|e| e.path == ["Window", "Swatch Libraries", "Gradients"])
        .map(|e| e.params["library"].clone())
        .collect();
    assert!(gradients.len() >= 5);
    assert_eq!(gradients, listed("gradients"));
    let patterns: Vec<Value> = crate::menus::menu_entries(&app)
        .into_iter()
        .filter(|e| e.path == ["Window", "Swatch Libraries", "Patterns"])
        .map(|e| e.params["library"].clone())
        .collect();
    assert!(!patterns.is_empty());
    assert_eq!(patterns, listed("patterns"));
    let defaults = crate::menus::menu_entries(&app).into_iter().any(|e| e.command.as_deref() == Some("swatch.resetDefaults"));
    assert!(defaults, "Default Swatches is listed");
}

#[test]
fn save_dialog_writes_user_libraries_that_the_window_menu_lists() {
=======
#[test]
fn save_dialog_writes_user_libraries_that_the_window_menu_lists() {
>>>>>>> REPLACE
<<<<<<< SEARCH
    app.run("ui.saveSwatchLibrary", json!({"names": ["Brights"]})).unwrap();
    let d = app.ui.dialog.as_mut().expect("Save Swatch Library opens");
    assert_eq!((d.kind.as_str(), d.bool("user")), (crate::dialogs::save_swatch_library::KIND, true));
    d.fields.insert("name".into(), json!("Loud"));
    d.fields.insert("selectedOnly".into(), json!(true));
    let ctx = context();
    let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1200.0, 800.0))), ..Default::default() };
    ctx.run_ui(input, |ui| crate::dialogs::show(&mut app, ui.ctx())).textures_delta.clear();
    assert!(app.ui.dialog.is_some(), "the dialog draws and stays open");
    crate::dialogs::confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none());
    assert_eq!(user_items(&app),
=======
    app.run("ui.saveSwatchLibrary", json!({"names": ["Brights"]})).unwrap();
    confirm_save(&mut app, &[("name", json!("Loud")), ("selectedOnly", json!(true))]);
    assert_eq!(user_items(&app),
>>>>>>> REPLACE
<<<<<<< SEARCH
    app.run("ui.saveSwatchLibrary", json!({})).unwrap();
    let d = app.ui.dialog.as_mut().unwrap();
    assert!(!d.bool("user") && !d.bool("__user"), "no user folder: a file");
    d.fields.insert("format".into(), json!("gpl"));
    crate::dialogs::confirm(&mut app).unwrap();
    let (path, bytes) = written.borrow()[0].clone();
    assert!(path.ends_with(".gpl"), "{path}");
=======
    app.run("ui.saveSwatchLibrary", json!({})).unwrap();
    confirm_save(&mut app, &[("format", json!("gpl"))]);
    let (path, bytes) = written.borrow()[0].clone();
    assert!(path.ends_with(".gpl"), "{path}");
>>>>>>> REPLACE
<<<<<<< SEARCH
    app.run("ui.saveSwatchLibrary", json!({})).unwrap();
    let d = app.ui.dialog.as_mut().unwrap();
    d.fields.insert("format".into(), json!("ase"));
    d.fields.insert("name".into(), json!("Brand Kit"));
    let text = crate::tests_labels::painted_text(&mut app, |app, ui| crate::dialogs::show(app, ui.ctx()));
    assert!(text.contains("Swatch Exchange (.ase)") && text.contains("Swatch exchange files keep solid colors only."), "{text}");
    crate::dialogs::confirm(&mut app).unwrap();
    let (path, bytes) = written.borrow()[0].clone();
=======
    app.run("ui.saveSwatchLibrary", json!({})).unwrap();
    confirm_save(&mut app, &[("format", json!("ase")), ("name", json!("Brand Kit"))]);
    let (path, bytes) = written.borrow()[0].clone();
>>>>>>> REPLACE
<<<<<<< SEARCH
    web.run("ui.saveSwatchLibrary", json!({})).unwrap();
    let d = web.ui.dialog.as_mut().unwrap();
    d.fields.insert("format".into(), json!("ase"));
    d.fields.insert("name".into(), json!(" Brand: Kit. "));
    crate::dialogs::confirm(&mut web).unwrap();
=======
    web.run("ui.saveSwatchLibrary", json!({})).unwrap();
    confirm_save(&mut web, &[("format", json!("ase")), ("name", json!(" Brand: Kit. "))]);
>>>>>>> REPLACE
<<<<<<< SEARCH
fn docking(app: &VectorcraftApp) -> &craft_ui::docking::Layout<String> {
    app.ui.docking.as_ref().expect("library panels live in the shared layout")
}
=======
fn docking(app: &VectorcraftApp) -> &craft_ui::docking::Layout<String> {
    // PLAN: assumed `Workspace::layout() -> &Layout<String>` (ch.P1)
    app.ui.dock_workspace.layout()
}
>>>>>>> REPLACE
<<<<<<< SEARCH
    assert!(crate::panel_docking::valid(docking(&app)));
    // `library: null` closes the panels of its kind.
=======
    assert!(app.ui.dock_workspace.validate().is_ok());
    // `library: null` closes the panels of its kind.
>>>>>>> REPLACE
<<<<<<< SEARCH
/// A context whose frames report their accessibility tree.
fn accessible() -> egui::Context {
    let ctx = context();
    ctx.enable_accesskit();
    ctx
}

=======
>>>>>>> REPLACE
<<<<<<< SEARCH
    assert!(next.ui.docking_hidden.keys().all(|p| p != &brights));
    // Unchecked,
=======
    // Unchecked,
>>>>>>> REPLACE
<<<<<<< SEARCH
    let ctx = accessible();
    let body = |app: &mut VectorcraftApp, ui: &mut Ui| show(app, ui, &brand);
    assert!(!control(&mut next, &ctx, &body, "Locate Library…").0, "dimmed without an open dialog");
    let panel_menu = |app: &mut VectorcraftApp, ui: &mut Ui| menu(app, ui, &brand);
    assert!(!control(&mut next, &ctx, &panel_menu, "Next Library").0, "no library to step from");
    let mut next = restarted(&next);
=======
    let ctx = context();
    let body = |app: &mut VectorcraftApp, ui: &mut Ui| show(app, ui, &brand);
    // Without a picker, Locate Library… reports it and the panel stays missing.
    let (_, at) = control(&mut next, &ctx, &body, "Locate Library…");
    click_at(&mut next, &ctx, &body, at);
    draw_frame(&mut next, &ctx, &body, vec![]);
    assert!(!next.ui.status.is_empty() && is_open(&next.ui, &brand), "{}", next.ui.status);
    assert!(neighbour::<SwatchLibraries>(&next, gone, 1).is_none(), "no library to step from");
    let mut next = restarted(&next);
>>>>>>> REPLACE
<<<<<<< SEARCH
    let (enabled, at) = control(&mut next, &ctx, &body, "Locate Library…");
    assert!(enabled);
    click_at(&mut next, &ctx, &body, at);
    let located
=======
    let (_, at) = control(&mut next, &ctx, &body, "Locate Library…");
    click_at(&mut next, &ctx, &body, at);
    // The picker's answer arrives on a later pass.
    draw_frame(&mut next, &ctx, &body, vec![]);
    draw_frame(&mut next, &ctx, &body, vec![]);
    let located
>>>>>>> REPLACE
<<<<<<< SEARCH
    let located_menu = |app: &mut VectorcraftApp, ui: &mut Ui| menu(app, ui, &located);
    assert!(control(&mut next, &ctx, &located_menu, "Next Library").0);
=======
    assert!(neighbour::<SwatchLibraries>(&next, id, 1).is_some(), "a library to step from");
>>>>>>> REPLACE
<<<<<<< SEARCH
#[test]
fn previous_and_next_show_the_neighbouring_library_in_the_same_panel() {
    let mut app = app();
=======
#[test]
fn previous_and_next_show_the_neighbouring_library_in_the_same_panel() {
    let mut app = app();
    let first_id = SWATCH_IDS[0];
    let last = SwatchLibraries::list(&app).pop().unwrap().id;
    assert_eq!(last, "optical-effects", "gradient and then pattern libraries come after the swatch libraries");
    assert_eq!(neighbour::<SwatchLibraries>(&app, "radial-glows", 1).as_deref(), Some("geometric-patterns"));
    assert_eq!(neighbour::<SwatchLibraries>(&app, "geometric-patterns", 1).as_deref(), Some("grit-textures"));
    assert_eq!(neighbour::<SwatchLibraries>(&app, "grit-textures", 1).as_deref(), Some("optical-effects"));
    assert_eq!(neighbour::<SwatchLibraries>(&app, first_id, -1), Some(last), "wraps around");
    assert_eq!(neighbour::<SwatchLibraries>(&app, "web-safe-216", 1).as_deref(), Some(SWATCH_IDS[1]));
>>>>>>> REPLACE
<<<<<<< SEARCH
    assert!(settings(&app.ui, "swatches", "pastels").is_none());
    assert!(crate::panel_docking::valid(docking(&app)));
=======
    assert!(settings(&app.ui, "swatches", "pastels").is_none());
    assert!(app.ui.dock_workspace.validate().is_ok());
>>>>>>> REPLACE
<<<<<<< SEARCH
#[test]
fn the_panel_menu_has_persistent() {
    let mut app = app();
    let panel = panel_of(app.run("window.swatchLibrary", json!({"library": "pastels"})).unwrap());
    let text = crate::tests_labels::painted_text(&mut app, |app, ui| menu(app, ui, &panel));
    for item in ["Add to Swatches", "Small List View", "Show Find Field", "Previous Library", "Persistent", "Close Library"] {
        assert!(text.contains(item), "{item}: {text}");
    }
}

/// In a panel made taller the library list fills the space above the panel's footer instead of
/// scrolling inside 220 pt (#2992); outside a sized panel it keeps that height.
#[test]
fn the_list_fills_a_tall_panel_above_its_footer() {
    let mut app = app();
    let ctx = context();
    app.run("window.swatchLibrary", json!({"library": "web-safe-216"})).unwrap();
    let panel = "library:swatches:web-safe-216";
    app.run("ui.libraryPanel", json!({"panel": panel, "view": "smallList"})).unwrap();
    // The clip rect of the rows' names (the colors' hex codes): the list's visible area.
    let mut list = |bottom: Option<f32>| {
        let mut clip = None;
        for _ in 0..3 {
            let raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(400.0, 800.0))), ..Default::default() };
            let mut out = ctx.run_ui(raw, |ui| {
                let info = egui::UiStackInfo::default().with_tag_value(widgets::PANEL_FOOTER, bottom);
                ui.scope_builder(egui::UiBuilder::new().ui_stack_info(info), |ui| crate::dock::panel_body(&mut app, ui, panel, 256.0, 760.0));
            });
            out.textures_delta.clear();
            clip = out.shapes.iter().find(|c| matches!(&c.shape, egui::Shape::Text(t) if t.galley.text().starts_with('#'))).map(|c| c.clip_rect);
        }
        clip.expect("a row is drawn")
    };
    let tall = list(Some(700.0));
    assert!(tall.height() > 400.0 && tall.bottom() <= 700.0 - 31.0, "the list fills the panel above its footer: {tall:?}");
    let loose = list(None);
    assert!(loose.height() <= 220.5, "unsized, the list keeps its height: {loose:?}");
}

=======
>>>>>>> REPLACE
<<<<<<< SEARCH
    app.run("ui.libraryPanel", json!({"panel": shown(&app), "view": "smallList"})).unwrap();
    let mut texts = vec![];
    for time in [0.0, 0.25] {
        let screen_rect = Some(Rect::from_min_size(Pos2::ZERO, vec2(1200.0, 800.0)));
        let mut out = ctx.run_ui(egui::RawInput { time: Some(time), screen_rect, ..Default::default() }, |ui| show_window(&mut app, ui.ctx()));
        out.textures_delta.clear();
        texts =
=======
    let panel = shown(&app);
    app.run("ui.libraryPanel", json!({"panel": panel, "view": "smallList"})).unwrap();
    let draw = |app: &mut VectorcraftApp, ui: &mut Ui| show(app, ui, &panel);
    let mut texts = vec![];
    for _ in 0..2 {
        let out = draw_frame(&mut app, &ctx, &draw, vec![]);
        texts =
>>>>>>> REPLACE
<<<<<<< SEARCH
    click(&mut app, &ctx, "Clay 1", Modifiers::NONE, 1.0);
    assert!(app.session.doc().unwrap().doc.swatch("Clay 1").is_some(), "added under its own name");
=======
    click(&mut app, &ctx, "Argile 1", Modifiers::NONE, 1.0);
    assert!(app.session.doc().unwrap().doc.swatch("Clay 1").is_some(), "added under its own name");
>>>>>>> REPLACE
