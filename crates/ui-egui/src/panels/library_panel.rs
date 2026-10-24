//! Library panels: read-only views of one library each (swatch, graphic style and symbol
//! libraries; any kind of library implementing [`LibraryKind`]) with thumbnail and list views, a
//! find field, the libraries menu and previous / next arrows. A plain click on an item adds it to
//! the document and applies it; Shift and Cmd/Ctrl-clicks select items (and folders) for Add to ….
//! A kind whose items drag ([`LibraryKind::drag`]) lets them be dragged out of the panel.
//!
//! As in Illustrator (<https://helpx.adobe.com/illustrator/using/using-creating-swatches.html>,
//! "Swatch libraries"), each open library is a panel of its own, `library:<kind>:<library id>`
//! ([`panel_id`]), that floats, tabs, docks and closes as any panel of the shared docking system:
//! `window.swatchLibrary`, `window.graphicStyleLibrary` and `window.symbolLibrary` open one, or bring it forward when it is
//! open. `UiState::library_panels` keeps each one's view, Show Find Field and Persistent
//! (`ui.libraryPanel`); a persistent panel reopens when the app starts ([`restore`]). A persistent
//! panel whose library can't be found shows the library as missing, with its file and Locate
//! Library… (`ui.libraryPanel.locate`). The find text and the selection are panel state.

use std::sync::{Mutex, PoisonError};

use egui::{Rect, Sense, Ui};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use vectorcraft_doc::style_libs::STYLES_EXT;
use vectorcraft_engine::cmd::fileio;
use vectorcraft_engine::cmd::swatchlib::{self, LibraryInfo};

use super::swatches::{View, click_selection, folder_tile, list_row, list_row_name, tile_grid};
use super::{pstate, set_pstate};
use crate::VectorcraftApp;
use crate::state::UiState;
use crate::widgets::{self, PanelDrag, menu_item};

/// What every library panel's id starts with.
pub const PREFIX: &str = "library:";

/// The most library panels one session opens: their ids are interned ([`intern`]).
const MAX_PANELS: usize = 256;

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
    /// The rows matching the find field's `query` (lower case; empty: all).
    fn rows<'a>(lib: &'a Self::Lib, query: &str) -> Vec<Row<'a, Self::Item>>;
    /// An item's thumbnail in `r`.
    fn draw(app: &VectorcraftApp, ui: &Ui, r: Rect, item: &Self::Item, selected: bool, hovered: bool);
    /// The icons at the right of list row `r`.
    fn row_icons(ui: &Ui, r: Rect, item: &Self::Item);
    /// An item described in words, for list tooltips.
    fn describe(item: &Self::Item) -> String;
    /// A plain click on item `name` of library `id`: add it to the document and apply it.
    fn click(app: &mut VectorcraftApp, ui: &Ui, id: &str, name: &str);
    /// Add the items and folders `names` of library `id` to the document.
    fn add(app: &mut VectorcraftApp, id: &str, names: Vec<String>);
    /// What dragging item `name` of library `id` out of the panel carries, if it drags.
    fn drag(_id: &str, _name: &str) -> Option<PanelDrag> {
        None
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
    body: fn(&mut VectorcraftApp, &mut Ui, &str),
    menu: fn(&mut VectorcraftApp, &mut Ui, &str),
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
        body: body::<K>,
        menu: panel_menu::<K>,
    }
}

/// The kind of library `kind` names.
fn kind(kind: &str) -> Option<Kind> {
    [
        kind_of::<super::swatches::SwatchLibraries>(),
        kind_of::<super::graphic_styles::GraphicStyleLibraries>(),
        kind_of::<super::symbols::SymbolLibraries>(),
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

/// Library panel `panel` as the `&'static str` the docking code keeps panel ids as
/// ([`crate::menus::normalize_panel`]); `None` when it isn't a library panel's id.
/// shortcut: each id is leaked once, at most `MAX_PANELS` of them (more libraries than one session
/// opens); make the docking code keep owned ids if that ever binds.
pub(crate) fn intern(panel: &str) -> Option<&'static str> {
    parse(panel)?;
    static IDS: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());
    let mut ids = IDS.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(id) = ids.iter().find(|id| **id == panel) {
        return Some(id);
    }
    if ids.len() >= MAX_PANELS {
        return None;
    }
    let id: &'static str = Box::leak(panel.into());
    ids.push(id);
    Some(id)
}

/// [`intern`], failing as a command does.
fn registered(panel: &str) -> Result<&'static str, String> {
    intern(panel).ok_or_else(|| "too many library panels were opened in this session".into())
}

/// Is panel `panel` open (in the dock or floating)?
pub(crate) fn is_open(ui: &UiState, panel: &str) -> bool {
    ui.docking.as_ref().is_some_and(|l| l.panels().iter().any(|p| p.as_str() == panel)) || crate::floating::group_of(ui, panel).is_some()
}

/// The open library panels.
fn open_panels(ui: &UiState) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    let docked = ui.docking.iter().flat_map(|l| l.panels());
    for p in docked.chain(ui.floating_panels.iter().flat_map(|g| &g.panels)) {
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

/// Library panel `panel`'s (≡) menu items.
pub(crate) fn menu(app: &mut VectorcraftApp, ui: &mut Ui, panel: &str) {
    if let Some((k, id)) = parse(panel) {
        (k.menu)(app, ui, id);
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
    for mut p in saved.into_iter().filter(|p| p.persistent).take(MAX_PANELS) {
        let Some(k) = kind(&p.kind) else { continue };
        if (k.name)(app, &p.id).is_none()
            && let Some(path) = p.path.clone()
            // A file now in the user library folder loads as that User Defined library.
            && let Ok(id) = app.run(k.load, json!({ "path": path })).map(|r| r["library"].as_str().unwrap_or_default().to_string())
            && id != p.id
            && rename(&mut app.ui, &p.panel(), &panel_id(k.kind, &id)).is_ok()
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
    app.ui.docking_hidden.retain(|p, _| !p.starts_with(PREFIX) || kept.contains(p));
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

/// Replace panel `from` with panel `to` (not open) where it is: in the dock or floating group it
/// is in, and as the place it goes back to.
fn rename(ui: &mut UiState, from: &str, to: &str) -> Result<(), String> {
    fn swap(v: &mut Value, from: &str, to: &str) {
        match v {
            Value::String(s) if s == from => *s = to.into(),
            Value::Array(a) => a.iter_mut().for_each(|v| swap(v, from, to)),
            Value::Object(o) => o.values_mut().for_each(|v| swap(v, from, to)),
            _ => {}
        }
    }
    // Panel ids are the only strings of a layout and a location.
    fn renamed<T: Serialize + serde::de::DeserializeOwned>(t: &T, from: &str, to: &str) -> Result<T, String> {
        let mut v = serde_json::to_value(t).map_err(|e| e.to_string())?;
        swap(&mut v, from, to);
        serde_json::from_value(v).map_err(|e| e.to_string())
    }
    registered(to)?;
    let docking = ui.docking.as_ref().map(|l| renamed(l, from, to)).transpose()?;
    let mut hidden = std::collections::BTreeMap::new();
    for (p, at) in &ui.docking_hidden {
        hidden.insert(if p == from { to.to_string() } else { p.clone() }, renamed(at, from, to)?);
    }
    ui.docking = docking;
    ui.docking_hidden = hidden;
    for p in ui.floating_panels.iter_mut().flat_map(|g| g.panels.iter_mut()).chain(ui.open_panel.as_mut()).filter(|p| p.as_str() == from) {
        *p = to.into();
    }
    Ok(())
}

/// The interaction id of the tile or row of `name` in panel `panel`.
pub(crate) fn tile_id(panel: &str, name: &str) -> egui::Id {
    egui::Id::new(("library-tile", panel, name))
}

/// The state key `key` of panel `panel`.
fn key(panel: &str, key: &str) -> String {
    format!("{panel}-{key}")
}

/// The submenu of the libraries menu that lists libraries of `category` (`None`: the top level).
pub(crate) fn submenu(category: &str) -> Option<&'static str> {
    match category {
        "gradients" => Some("Gradients"),
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
    registered(&panel)?;
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
            rename(&mut app.ui, given, &to)?;
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
/// picked in an open dialog (on the web the picked file arrives later and opens in a panel of its
/// own, as Other Library…'s does), and show it in open panel `panel`, which keeps its view, Show
/// Find Field and Persistent. When the file's library has a panel open already, `panel` closes and
/// is forgotten, and that panel is brought forward, made persistent when `panel` was. → what
/// `ui.libraryPanel` answers
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
    let Some(path) = pick_library_file(app, path, filters)? else { return Ok(Value::Null) };
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
/// (symbol libraries: any document).
pub(crate) fn library_pick_filters(kind: &str) -> Vec<(&'static str, &'static [&'static str])> {
    match kind {
        super::swatches::SwatchLibraries::KIND => fileio::library_open_filters("Swatch libraries", swatchlib::LIBRARY_EXTS).collect(),
        super::graphic_styles::GraphicStyleLibraries::KIND => fileio::library_open_filters("Graphic Style Libraries", &[STYLES_EXT]).collect(),
        _ => fileio::open_filters().collect(),
    }
}

/// Other Library…: the library file at `path`, else one picked in an open dialog; `None` when the
/// pick is asynchronous (on the web the picked file arrives later through [`crate::io::open_bytes`]).
pub(crate) fn pick_library_file(
    app: &mut VectorcraftApp,
    path: Option<String>,
    filters: Vec<(&'static str, &'static [&'static str])>,
) -> Result<Option<String>, String> {
    if path.is_none()
        && let Some(f) = app.services.open_async.as_mut()
    {
        f();
        return Ok(None);
    }
    path.or_else(|| {
        let pick = crate::FilePick { filters: crate::FilePick::filters(filters), ..Default::default() };
        crate::picks::open(app, &pick)
    })
    .map(Some)
    .ok_or_else(|| "cancelled".into())
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
    area.show(ctx, |ui| crate::dock::panel_body(app, ui, &panel, 256.0, 400.0));
}

/// The selected items and folders of panel `panel`.
fn selection(ctx: &egui::Context, panel: &str) -> Vec<String> {
    pstate(ctx, &key(panel, "selected"))
}

/// Find field, thumbnails or list rows, and the bottom bar of library `id`'s panel; for a missing
/// library, [`missing`].
fn body<K: LibraryKind>(app: &mut VectorcraftApp, ui: &mut Ui, id: &str) {
    let Some((_, lib)) = K::get(app, id) else {
        missing::<K>(app, ui, id);
        return;
    };
    let panel = panel_id(K::KIND, id);
    let (view, hide_find) = settings(&app.ui, K::KIND, id).map_or((K::VIEW, false), |s| (s.view.unwrap_or(K::VIEW), s.hide_find));
    let query = if hide_find { String::new() } else { widgets::search_field(ui, egui::Id::new(key(&panel, "find")), tl!("Find")) };
    ui.add_space(4.0);
    let rows = K::rows(&lib, &query.trim().to_lowercase());
    let selected = selection(ui.ctx(), &panel);
    let is_sel = |n: &str| selected.iter().any(|s| s == n);
    let mut clicked: Option<(String, bool, egui::Modifiers)> = None;
    let mut click = |ui: &Ui, resp: egui::Response, row: &Row<K::Item>, tip: &dyn Fn() -> String| {
        if resp.on_hover_text(tip()).clicked() {
            clicked = Some((row.name.to_string(), row.item.is_none(), ui.input(|i| i.modifiers)));
        }
    };
    widgets::list_box(ui, |ui| {
        let max_height = if view == View::LargeThumb { 280.0 } else { 220.0 };
        egui::ScrollArea::vertical().id_salt(key(&panel, "scroll")).max_height(max_height).show(ui, |ui| {
            ui.set_width(ui.available_width());
            if rows.is_empty() {
                widgets::dim_label(ui, if query.trim().is_empty() { tl!("This library is empty.") } else { tl!("No matches.") });
            } else if view.is_list() {
                for row in &rows {
                    let (r, resp, chip) = list_row(ui, tile_id(&panel, row.name), view, is_sel(row.name));
                    match row.item {
                        Some(item) => {
                            K::draw(app, ui, chip, item, false, false);
                            K::row_icons(ui, r, item);
                        }
                        None => super::swatches::draw_folder(ui, chip),
                    }
                    list_row_name(ui, r, chip, row.name);
                    drag_out::<K>(ui, &resp, id, row);
                    let tip = || row.item.map_or_else(|| row.name.to_string(), |i| format!("{} ({})", row.name, K::describe(i)));
                    click(ui, resp, row, &tip);
                }
            } else {
                for (row, cell) in tile_grid(ui, &rows, view, |r| r.item.is_none()) {
                    let resp = ui.interact(cell, tile_id(&panel, row.name), Sense::click_and_drag());
                    match row.item {
                        Some(item) => K::draw(app, ui, cell, item, is_sel(row.name), resp.hovered()),
                        None => folder_tile(ui, cell, is_sel(row.name)),
                    }
                    drag_out::<K>(ui, &resp, id, row);
                    click(ui, resp, row, &|| row.name.to_string());
                }
            }
        });
    });
    if let Some((name, folder, m)) = clicked {
        // A plain click on an item also adds and applies it; folders and modifier clicks only select.
        let only_select = folder || m.shift || m.command;
        let order: Vec<&str> = rows.iter().map(|r| r.name).collect();
        let sel = click_selection(ui, &key(&panel, "anchor"), selected, &order, &name, m);
        set_pstate(ui.ctx(), &key(&panel, "selected"), sel);
        if !only_select {
            K::click(app, ui, id, &name);
        }
    }
    bottom::<K>(app, ui, id);
}

/// An item of library `id` dragged out of its tile or row `resp` carries what `K` says it does.
fn drag_out<K: LibraryKind>(ui: &Ui, resp: &egui::Response, id: &str, row: &Row<K::Item>) {
    if row.item.is_some()
        && resp.drag_started()
        && let Some(drag) = K::drag(id, row.name)
    {
        egui::DragAndDrop::set_payload(ui.ctx(), drag);
    }
}

/// The body of the panel of library `id` when the library is missing ([`missing_file`]): its file,
/// and Locate Library… where the app has an open dialog.
fn missing<K: LibraryKind>(app: &mut VectorcraftApp, ui: &mut Ui, id: &str) {
    let Some(file) = missing_file(app, kind_of::<K>(), id) else { return };
    widgets::list_box(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.add_space(4.0);
        widgets::dim_name(ui, &crate::i18n::fmt(tl!("Missing: {path}"), &[("path", &file)]));
        ui.add_space(4.0);
    });
    ui.add_space(8.0);
    let can = crate::picks::can(app, &crate::picks::PickRequest::Open(crate::FilePick::default()));
    if ui.add_enabled(can, egui::Button::new(tl!("Locate Library…"))).clicked()
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

/// Libraries menu, previous / next library, and Add to … for the selection.
fn bottom<K: LibraryKind>(app: &mut VectorcraftApp, ui: &mut Ui, id: &str) {
    let selected = selection(ui.ctx(), &panel_id(K::KIND, id));
    widgets::bottom_bar(ui, |ui| {
        let r = widgets::icon_button(ui, "library", K::MENU, false, 24.0);
        egui::Popup::menu(&r).show(|ui| {
            ui.set_min_width(200.0);
            library_menu::<K>(app, ui);
        });
        for (icon, tip, s) in [("chevron-left", tl!("Previous Library"), -1), ("chevron-right", tl!("Next Library"), 1)] {
            if widgets::icon_button(ui, icon, tip, false, 24.0).clicked() {
                step::<K>(app, id, s);
            }
        }
        ui.add_space((ui.available_width() - 28.0).max(0.0));
        if widgets::icon_button_enabled(ui, "dc-new-item", K::ADD, false, !selected.is_empty(), 24.0).clicked() {
            K::add(app, id, selected.clone());
        }
    });
}

/// A library panel's (≡) menu.
fn panel_menu<K: LibraryKind>(app: &mut VectorcraftApp, ui: &mut Ui, id: &str) {
    let panel = panel_id(K::KIND, id);
    let selected = selection(ui.ctx(), &panel);
    if menu_item(ui, K::ADD, !selected.is_empty(), false) {
        K::add(app, id, selected);
    }
    ui.separator();
    let s = settings(&app.ui, K::KIND, id);
    let (view, hide_find, persistent) = s.map_or((K::VIEW, false, false), |s| (s.view.unwrap_or(K::VIEW), s.hide_find, s.persistent));
    for (v, label) in View::ALL {
        if menu_item(ui, label, true, v == view) {
            set(app, &panel, json!({ "view": v }));
        }
    }
    ui.separator();
    if menu_item(ui, tl!("Show Find Field"), true, !hide_find) {
        set(app, &panel, json!({ "showFind": hide_find }));
    }
    ui.separator();
    // Previous Library and Next Library are dimmed for a missing library ([`missing_file`]).
    let listed = K::get(app, id).is_some();
    for (label, s) in [(tl!("Previous Library"), -1), (tl!("Next Library"), 1)] {
        if menu_item(ui, label, listed, false) {
            step::<K>(app, id, s);
        }
    }
    if menu_item(ui, tl!("Persistent"), true, persistent) {
        set(app, &panel, json!({ "persistent": !persistent }));
    }
    if menu_item(ui, tl!("Close Library"), true, false) {
        close(app, &panel);
    }
}

/// The libraries menu (a library panel's library button, the Swatches panel's): every library by
/// submenu, the open ones checked, between `K`'s head and tail items.
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
        if widgets::menu_item_name(ui, library_name(&l.id, &l.name), true, checked(&l.id)) {
            chosen = Some(l.id.clone());
        }
    };
    for l in libs.iter().filter(|l| l.submenu.is_none()) {
        item(ui, l);
    }
    let mut subs: Vec<&str> = vec![];
    for s in libs.iter().filter_map(|l| l.submenu) {
        if !subs.contains(&s) {
            subs.push(s);
        }
    }
    for s in subs {
        ui.menu_button(tl!(s), |ui| {
            crate::widgets::menu_scroll(ui, |ui| {
                for l in libs.iter().filter(|l| l.submenu == Some(s)) {
                    item(ui, l);
                }
            });
        });
    }
    chosen
}

#[cfg(test)]
mod tests {
    use egui::{Event, Modifiers, PointerButton, Pos2, vec2};
    use serde_json::Value;
    use vectorcraft_color::Paint;
    use vectorcraft_engine::Session;

    use super::*;
    use crate::panels::swatches::SwatchLibraries;

    fn app() -> VectorcraftApp {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        app
    }

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

    fn undo_len(app: &VectorcraftApp) -> usize {
        app.session.doc().unwrap().history.undo.len()
    }

    #[test]
    fn clicking_a_library_swatch_adds_and_applies_it_in_one_step() {
        let mut app = app();
        let ctx = context();
        let r = app.run("window.swatchLibrary", json!({"library": "Pastels"})).unwrap();
        assert_eq!((r["open"].as_str(), r["count"].as_u64(), r["panel"].as_str()), (Some("pastels"), Some(24), Some("library:swatches:pastels")));
        assert_eq!(last_open(&app.ui).map(|p| (p.kind.as_str(), p.id.as_str())), Some(("swatches", "pastels")));
        let undo = undo_len(&app);
        click(&mut app, &ctx, "Pastel Teal", Modifiers::NONE, 0.0);
        let d = &app.session.doc().unwrap().doc;
        let added = d.swatch("Pastel Teal").expect("added to the document");
        assert_eq!(app.session.paint.fill, Paint::solid(added.paint.color().unwrap()), "applied to the fill");
        assert_eq!(undo_len(&app), undo + 1);
        // Alt-click paints the stroke (the inactive proxy).
        click(&mut app, &ctx, "Soft Blue", Modifiers::ALT, 2.0);
        let blue = app.session.doc().unwrap().doc.swatch("Soft Blue").and_then(|w| w.paint.color()).unwrap();
        assert_eq!(app.session.paint.stroke.color(), Some(blue));
        assert!(app.session.fill_active, "Alt keeps the active proxy");
        // A library that doesn't exist is an error; null closes the panel.
        assert!(app.run("window.swatchLibrary", json!({"library": "nope"})).is_err());
        app.run("window.swatchLibrary", json!({"library": null})).unwrap();
        assert!(last_open(&app.ui).is_none());
    }

    #[test]
    fn modifier_clicks_select_and_add_to_swatches_adds_the_selection() {
        let mut app = app();
        let ctx = context();
        app.run("window.swatchLibrary", json!({"library": "earth-tones"})).unwrap();
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
        let order: Vec<String> = SwatchLibraries::rows(&lib, "").iter().map(|r| r.name.to_string()).collect();
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
        ctx.data_mut(|d| d.insert_temp(egui::Id::new(key(panel, "find")), "#FF00".to_string()));
        frame(&mut app, &ctx, vec![], Modifiers::NONE, 0.0);
        frame(&mut app, &ctx, vec![], Modifiers::NONE, 0.5);
        assert!(ctx.read_response(tile_id(panel, "#FF0033")).is_some());
        assert!(ctx.read_response(tile_id(panel, "#00FF00")).is_none());
        let first = SWATCH_IDS[0];
        let last = SwatchLibraries::list(&app).pop().unwrap().id;
        assert_eq!(last, "radial-glows", "gradient libraries come after the swatch libraries");
        assert_eq!(neighbour::<SwatchLibraries>(&app, first, -1), Some(last), "wraps around");
        assert_eq!(neighbour::<SwatchLibraries>(&app, "web-safe-216", 1).as_deref(), Some(SWATCH_IDS[1]));
    }

    const SWATCH_IDS: [&str; 9] = [
        "web-safe-216",
        "grays-neutrals",
        "earth-tones",
        "skin-tone-ramps",
        "pastels",
        "brights",
        "metallic-gradients",
        "perceptual-scales",
        "harmony-sets",
    ];

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
        let defaults = crate::menus::menu_entries(&app).into_iter().any(|e| e.command.as_deref() == Some("swatch.resetDefaults"));
        assert!(defaults, "Default Swatches is listed");
    }

    #[test]
    fn save_dialog_writes_user_libraries_that_the_window_menu_lists() {
        let dir = std::env::temp_dir().join(format!("vc-library-panel-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut app = app();
        app.session.swatch_libraries.set_user_dir(Some(dir.to_string_lossy().to_string()));
        let user_items = |app: &VectorcraftApp| -> Vec<(String, String)> {
            crate::menus::menu_entries(app)
                .into_iter()
                .filter(|e| e.path == ["Window", "Swatch Libraries", "User Defined"])
                .map(|e| (e.label, e.command.unwrap_or_default()))
                .collect()
        };
        assert!(user_items(&app).is_empty(), "empty slots are hidden");
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
        assert_eq!(user_items(&app), [("Loud".to_string(), "window.userSwatchLibrary1".to_string())]);
        app.run("window.userSwatchLibrary1", json!({})).unwrap();
        assert_eq!(last_open(&app.ui).map(|p| p.id.as_str()), Some("user/Loud.vcswatches"));
        let (_, lib) = SwatchLibraries::get(&app, "user/Loud.vcswatches").unwrap();
        assert_eq!((lib.groups.len(), lib.len()), (1, 5), "only the selected group");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn saving_to_a_file_and_opening_a_palette_file() {
        let written = std::rc::Rc::new(std::cell::RefCell::new(vec![]));
        let w = written.clone();
        let services = crate::Services {
            pick_save: Some(Box::new(|p: &crate::FilePick| Some(format!("/tmp/{}", p.name)))),
            write: Some(Box::new(move |p: &str, b: &[u8]| {
                w.borrow_mut().push((p.to_string(), b.to_vec()));
                Ok(())
            })),
            ..Default::default()
        };
        let mut app = VectorcraftApp::new(Session::new(), services);
        app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
        app.run("ui.saveSwatchLibrary", json!({})).unwrap();
        let d = app.ui.dialog.as_mut().unwrap();
        assert!(!d.bool("user") && !d.bool("__user"), "no user folder: a file");
        d.fields.insert("format".into(), json!("gpl"));
        crate::dialogs::confirm(&mut app).unwrap();
        let (path, bytes) = written.borrow()[0].clone();
        assert!(path.ends_with(".gpl"), "{path}");
        let text = String::from_utf8(bytes).unwrap();
        assert!(
            text.starts_with(
                "GIMP Palette
"
            ) && text.contains(
                "	Red
"
            ),
            "{text}"
        );
        // Opening a palette file opens it in the library panel.
        crate::io::open_bytes(&mut app, "Saved.gpl", text.as_bytes(), None).unwrap();
        let open = last_open(&app.ui).cloned().unwrap();
        assert!(open.id.starts_with("loaded/"), "{open:?}");
        assert_eq!(app.session.documents().len(), 1, "not opened as a document");
        let other = SwatchLibraries::list(&app).into_iter().find(|l| l.id == open.id).unwrap();
        assert_eq!(other.submenu, Some("Other Libraries"));
        // Built-in libraries' names are interface labels; a file's is shown as it is.
        for l in SwatchLibraries::list(&app) {
            assert_eq!(builtin_library(&l.id), !matches!(l.submenu, Some("User Defined" | "Other Libraries")), "{}", l.id);
        }
        assert!(!builtin_library(&open.id) && builtin_library(vectorcraft_engine::cmd::swatchlib::DOCUMENT_SWATCHES));
    }

    #[test]
    fn the_save_dialog_writes_swatch_exchange_files() {
        let written = std::rc::Rc::new(std::cell::RefCell::new(vec![]));
        let w = written.clone();
        let services = crate::Services {
            pick_save: Some(Box::new(|p: &crate::FilePick| Some(format!("/tmp/{}", p.name)))),
            write: Some(Box::new(move |p: &str, b: &[u8]| {
                w.borrow_mut().push((p.to_string(), b.to_vec()));
                Ok(())
            })),
            ..Default::default()
        };
        let mut app = VectorcraftApp::new(Session::new(), services);
        app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
        app.run("ui.saveSwatchLibrary", json!({})).unwrap();
        let d = app.ui.dialog.as_mut().unwrap();
        d.fields.insert("format".into(), json!("ase"));
        d.fields.insert("name".into(), json!("Brand Kit"));
        let text = crate::tests_labels::painted_text(&mut app, |app, ui| crate::dialogs::show(app, ui.ctx()));
        assert!(text.contains("Swatch Exchange (.ase)") && text.contains("Swatch exchange files keep solid colors only."), "{text}");
        crate::dialogs::confirm(&mut app).unwrap();
        let (path, bytes) = written.borrow()[0].clone();
        assert_eq!(path, "/tmp/Brand Kit.ase", "the save panel suggests the name");
        let lib = vectorcraft_color::palette_io::read_bytes(&bytes, "x").unwrap();
        let solid = app.session.doc().unwrap().doc.swatches_iter().filter(|w| w.paint.color().is_some()).count();
        assert_eq!(lib.len(), solid, "every solid color of the document");
        // The web downloads the file under that name.
        let downloads = std::rc::Rc::new(std::cell::RefCell::new(vec![]));
        let dl = downloads.clone();
        let services = crate::Services {
            download: Some(Box::new(move |name: &str, _: &[u8]| {
                dl.borrow_mut().push(name.to_string());
                Ok(())
            })),
            ..Default::default()
        };
        let mut web = VectorcraftApp::new(Session::new(), services);
        web.run("file.new", json!({"width": 100, "height": 100})).unwrap();
        web.run("ui.saveSwatchLibrary", json!({})).unwrap();
        let d = web.ui.dialog.as_mut().unwrap();
        d.fields.insert("format".into(), json!("ase"));
        d.fields.insert("name".into(), json!(" Brand: Kit. "));
        crate::dialogs::confirm(&mut web).unwrap();
        assert_eq!(*downloads.borrow(), ["Brand- Kit.ase"], "the name made safe as a file name, as for User Defined");
    }

    #[test]
    fn swatch_exchange_files_without_a_path_open_in_the_library_panel() {
        // A file opened on the web has no path; open_bytes passes its bytes as dataBase64.
        let mut app = app();
        crate::io::open_bytes(&mut app, "Brand.ase", &vectorcraft_testkit::ase::sample(), None).unwrap();
        let open = last_open(&app.ui).cloned().unwrap();
        assert_eq!(open.id, "loaded/Brand.ase");
        let (_, lib) = SwatchLibraries::get(&app, &open.id).unwrap();
        assert_eq!((lib.len(), lib.groups.len()), (4, 1));
        assert_eq!(app.session.documents().len(), 1, "not opened as a document");
    }

    fn docking(app: &VectorcraftApp) -> &craft_ui::docking::Layout<String> {
        app.ui.docking.as_ref().expect("library panels live in the shared layout")
    }

    fn panel_of(reply: Value) -> String {
        reply["panel"].as_str().expect("the reply names the panel").to_string()
    }

    fn groups(app: &VectorcraftApp) -> Vec<Vec<String>> {
        docking(app).floating.iter().map(|g| g.panels.clone()).collect()
    }

    #[test]
    fn each_library_opens_in_a_panel_of_its_own() {
        let mut app = app();
        // Without the shared layout yet, `window.panel` floats a library panel too.
        app.run("window.panel", json!({"panel": "library:swatches:earth-tones"})).unwrap();
        assert!(is_open(&app.ui, "library:swatches:earth-tones"));
        app.run("window.panel.close", json!({"panel": "library:swatches:earth-tones"})).unwrap();
        let pastels = panel_of(app.run("window.swatchLibrary", json!({"library": "pastels"})).unwrap());
        let brights = panel_of(app.run("window.swatchLibrary", json!({"library": "Brights"})).unwrap());
        let styles = panel_of(app.run("window.graphicStyleLibrary", json!({"library": "hand-drawn"})).unwrap());
        assert_eq!([&pastels, &brights, &styles], ["library:swatches:pastels", "library:swatches:brights", "library:graphicStyles:hand-drawn"]);
        assert_eq!(groups(&app), [vec![pastels.clone()], vec![brights.clone()], vec![styles.clone()]], "each floats on its own");
        assert_ne!(docking(&app).floating[0].rect, docking(&app).floating[1].rect, "one beside another");
        assert_eq!(tab_label(&app, &brights).as_deref(), Some("Brights"));
        // Opening an open library brings its panel forward.
        app.run("window.swatchLibrary", json!({"library": "pastels"})).unwrap();
        assert_eq!(groups(&app).len(), 3);
        assert_eq!(last_open(&app.ui).map(LibraryPanel::panel), Some(pastels.clone()));
        // They group, dock and close as any panel, and `window.panel` shows one again.
        app.run("window.panel.float", json!({"panel": brights, "onto": pastels})).unwrap();
        assert!(groups(&app).contains(&vec![pastels.clone(), brights.clone()]), "{:?}", groups(&app));
        app.run("window.panel.dock", json!({"panel": styles})).unwrap();
        assert!(craft_ui::docking::Layout { root: docking(&app).root.clone(), floating: vec![] }.contains(&styles));
        app.run("window.panel.close", json!({"panel": brights})).unwrap();
        assert!(!is_open(&app.ui, &brights) && is_open(&app.ui, &pastels));
        app.run("window.panel", json!({"panel": brights})).unwrap();
        assert!(is_open(&app.ui, &brights));
        assert!(crate::panel_docking::valid(docking(&app)));
        // `library: null` closes the panels of its kind.
        app.run("window.swatchLibrary", json!({"library": null})).unwrap();
        assert!(!any_open::<SwatchLibraries>(&app.ui) && is_open(&app.ui, &styles));
        // What isn't a library panel's id.
        for bad in ["library:nope:x", "library:swatches:", "library:swatches", "Library:swatches:pastels"] {
            assert!(crate::menus::normalize_panel(bad).is_none(), "{bad}");
        }
        assert!(app.run("ui.libraryPanel", json!({"panel": "layers"})).is_err());
        assert!(app.run("ui.libraryPanel", json!({"panel": pastels, "persistent": true})).is_err(), "closed and not persistent");
        assert!(app.run("ui.libraryPanel", json!({"panel": styles, "view": "huge"})).is_err());
    }

    /// A context whose frames report their accessibility tree.
    fn accessible() -> egui::Context {
        let ctx = context();
        ctx.enable_accesskit();
        ctx
    }

    /// One frame of `draw` with `events`, after the frames `ctx` ran.
    fn draw_frame(
        app: &mut VectorcraftApp,
        ctx: &egui::Context,
        draw: &dyn Fn(&mut VectorcraftApp, &mut Ui),
        events: Vec<Event>,
    ) -> egui::FullOutput {
        let screen_rect = Some(Rect::from_min_size(Pos2::ZERO, vec2(1200.0, 800.0)));
        let time = Some(ctx.input(|i| i.time) + 0.1);
        let mut out = ctx.run_ui(egui::RawInput { events, time, screen_rect, ..Default::default() }, |ui| draw(app, ui));
        out.textures_delta.clear();
        out
    }

    /// Whether the control `draw` shows that screen readers name `label` is enabled, and its center.
    fn control(app: &mut VectorcraftApp, ctx: &egui::Context, draw: &dyn Fn(&mut VectorcraftApp, &mut Ui), label: &str) -> (bool, Pos2) {
        draw_frame(app, ctx, draw, vec![]);
        let out = draw_frame(app, ctx, draw, vec![]);
        let nodes = &out.platform_output.accesskit_update.as_ref().expect("an accessibility tree").nodes;
        let (_, node) = nodes.iter().find(|(_, n)| n.label().map(str::trim) == Some(label)).unwrap_or_else(|| panic!("no control named {label:?}"));
        let b = node.bounds().expect("bounds");
        (!node.is_disabled(), Pos2::new((b.x0 + b.x1) as f32 * 0.5, (b.y0 + b.y1) as f32 * 0.5))
    }

    /// A click at `at` in a frame of `draw`.
    fn click_at(app: &mut VectorcraftApp, ctx: &egui::Context, draw: &dyn Fn(&mut VectorcraftApp, &mut Ui), at: Pos2) {
        let button = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
        draw_frame(app, ctx, draw, vec![Event::PointerMoved(at)]);
        draw_frame(app, ctx, draw, vec![button(true), button(false)]);
    }

    /// The app restarted with the preferences `app` saves.
    fn restarted(app: &VectorcraftApp) -> VectorcraftApp {
        let saved: crate::UiState = serde_json::from_slice(&serde_json::to_vec(&app.ui).unwrap()).unwrap();
        let mut next = VectorcraftApp::new(Session::new(), Default::default());
        next.ui = saved.sanitized();
        restore(&mut next);
        next
    }

    #[test]
    fn persistent_panels_reopen_at_startup_with_their_settings() {
        let dir = std::env::temp_dir().join(format!("vc-persistent-libraries-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let gpl = dir.join("Brand.gpl");
        std::fs::write(&gpl, "GIMP Palette\nName: Brand\n#\n255   0   0\tRed\n").unwrap();
        let mut app = app();
        let pastels = panel_of(app.run("window.swatchLibrary", json!({"library": "pastels"})).unwrap());
        let brights = panel_of(app.run("window.swatchLibrary", json!({"library": "brights"})).unwrap());
        let brand = panel_of(app.run("window.swatchLibrary.other", json!({"path": gpl.to_string_lossy()})).unwrap());
        let r = app.run("ui.libraryPanel", json!({"panel": pastels, "persistent": true, "view": "largeList", "showFind": false})).unwrap();
        assert_eq!((r["persistent"].as_bool(), r["view"].as_str(), r["showFind"].as_bool()), (Some(true), Some("largeList"), Some(false)));
        app.run("ui.libraryPanel", json!({"panel": brand, "persistent": true})).unwrap();
        app.run("window.panel.float", json!({"panel": pastels, "x": 40, "y": 90})).unwrap();
        // A persistent panel reopens even when it was closed.
        app.run("window.panel.close", json!({"panel": brand})).unwrap();
        let next = restarted(&app);
        assert!(is_open(&next.ui, &pastels) && is_open(&next.ui, &brand), "{}", next.ui.status);
        assert!(!is_open(&next.ui, &brights), "a panel that isn't persistent closed with the app");
        let group = docking(&next).floating.iter().find(|g| g.panels.contains(&pastels)).unwrap();
        assert_eq!(group.rect[..2], [40.0, 90.0], "where it was");
        let s = settings(&next.ui, "swatches", "pastels").unwrap();
        assert_eq!((s.view, s.hide_find, s.persistent), (Some(View::LargeList), true, true));
        let (_, id) = parse(&brand).unwrap();
        assert!(SwatchLibraries::get(&next, id).is_some_and(|(_, lib)| lib.swatch("Red").is_some()), "read again from its file");
        assert!(next.ui.status.is_empty(), "{}", next.ui.status);
        assert!(next.ui.docking_hidden.keys().all(|p| p != &brights));
        // Unchecked, a panel no longer reopens; one whose file is gone reopens as missing, with a note.
        let mut next = next;
        next.run("ui.libraryPanel", json!({"panel": pastels, "persistent": false})).unwrap();
        std::fs::remove_file(&gpl).unwrap();
        let last = restarted(&next);
        assert!(!is_open(&last.ui, &pastels) && is_open(&last.ui, &brand));
        assert!(last.ui.status.contains("Brand.gpl"), "{}", last.ui.status);
        assert_eq!(last.ui.library_panels.iter().map(LibraryPanel::panel).collect::<Vec<_>>(), [brand]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_persistent_panel_whose_file_is_gone_is_listed_as_missing_until_located() {
        let dir = std::env::temp_dir().join(format!("vc-missing-library-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("moved")).unwrap();
        let palette = "GIMP Palette\nName: Brand\n#\n255   0   0\tRed\n";
        let gpl = dir.join("Brand.gpl");
        std::fs::write(&gpl, palette).unwrap();
        let mut app = app();
        let brand = panel_of(app.run("window.swatchLibrary.other", json!({"path": gpl.to_string_lossy()})).unwrap());
        app.run("ui.libraryPanel", json!({"panel": brand, "persistent": true, "view": "smallList"})).unwrap();
        std::fs::remove_file(&gpl).unwrap();
        // The panel reopens showing its library as missing, and the preferences keep it.
        let mut next = restarted(&app);
        tidy(&mut next);
        assert!(is_open(&next.ui, &brand), "{}", next.ui.status);
        assert!(next.ui.status.contains("Brand.gpl"), "{}", next.ui.status);
        let (_, gone) = parse(&brand).unwrap();
        let s = settings(&next.ui, "swatches", gone).cloned().unwrap();
        assert_eq!((s.path, s.persistent, s.view), (Some(gpl.to_string_lossy().into_owned()), true, Some(View::SmallList)));
        assert_eq!(tab_label(&next, &brand).as_deref(), Some("Brand"));
        // Closed, it opens again by its library's id.
        next.run("window.panel.close", json!({"panel": brand})).unwrap();
        let r = next.run("window.swatchLibrary", json!({"library": gone})).unwrap();
        assert_eq!((r["panel"].as_str(), r["name"].as_str(), r["count"].as_u64()), (Some(brand.as_str()), Some("Brand"), Some(0)));
        assert!(is_open(&next.ui, &brand));
        let text = crate::tests_labels::painted_text(&mut next, |app, ui| show(app, ui, &brand));
        let shown = format!("Missing: {}", gpl.to_string_lossy());
        assert!(text.contains(&shown), "{text}");
        let ctx = accessible();
        let body = |app: &mut VectorcraftApp, ui: &mut Ui| show(app, ui, &brand);
        assert!(!control(&mut next, &ctx, &body, "Locate Library…").0, "dimmed without an open dialog");
        let panel_menu = |app: &mut VectorcraftApp, ui: &mut Ui| menu(app, ui, &brand);
        assert!(!control(&mut next, &ctx, &panel_menu, "Next Library").0, "no library to step from");
        let mut next = restarted(&next);
        assert!(is_open(&next.ui, &brand), "still listed after another restart");
        // Locate Library… reads the file where it is now and shows it in the same panel.
        assert!(next.run("ui.libraryPanel.locate", json!({"panel": "layers"})).is_err());
        let moved = dir.join("moved").join("Brand.gpl");
        std::fs::write(&moved, palette).unwrap();
        let picked = moved.to_string_lossy().into_owned();
        next.services.pick_open = Some(Box::new(move |_: &crate::FilePick| Some(picked.clone())));
        let (enabled, at) = control(&mut next, &ctx, &body, "Locate Library…");
        assert!(enabled);
        click_at(&mut next, &ctx, &body, at);
        let located = panel_id("swatches", &format!("loaded/{}", moved.to_string_lossy()));
        assert!(!is_open(&next.ui, &brand) && is_open(&next.ui, &located), "{}", next.ui.status);
        let (_, id) = parse(&located).unwrap();
        assert!(SwatchLibraries::get(&next, id).is_some_and(|(_, lib)| lib.swatch("Red").is_some()));
        let s = settings(&next.ui, "swatches", id).cloned().unwrap();
        assert_eq!((s.path, s.persistent, s.view), (Some(moved.to_string_lossy().into_owned()), true, Some(View::SmallList)));
        assert!(settings(&next.ui, "swatches", gone).is_none());
        let located_menu = |app: &mut VectorcraftApp, ui: &mut Ui| menu(app, ui, &located);
        assert!(control(&mut next, &ctx, &located_menu, "Next Library").0);
        let last = restarted(&next);
        assert!(is_open(&last.ui, &located) && last.ui.status.is_empty(), "{}", last.ui.status);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn locating_a_library_open_in_its_own_panel_brings_that_panel_forward() {
        let dir = std::env::temp_dir().join(format!("vc-locate-open-library-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let palette = "GIMP Palette\nName: Brand\n#\n255   0   0\tRed\n";
        let gpl = dir.join("Brand.gpl");
        std::fs::write(&gpl, palette).unwrap();
        let mut app = app();
        let brand = panel_of(app.run("window.swatchLibrary.other", json!({"path": gpl.to_string_lossy()})).unwrap());
        app.run("ui.libraryPanel", json!({"panel": brand, "persistent": true})).unwrap();
        std::fs::remove_file(&gpl).unwrap();
        let mut next = restarted(&app);
        let copy = dir.join("Copy.gpl");
        std::fs::write(&copy, palette).unwrap();
        let other = panel_of(next.run("window.swatchLibrary.other", json!({"path": copy.to_string_lossy()})).unwrap());
        next.run("window.panel.close", json!({"panel": brand})).unwrap();
        assert!(next.run("ui.libraryPanel.locate", json!({"panel": brand, "path": copy.to_string_lossy()})).is_err(), "a closed panel");
        next.run("window.panel", json!({"panel": brand})).unwrap();
        let r = next.run("ui.libraryPanel.locate", json!({"panel": brand, "path": copy.to_string_lossy()})).unwrap();
        assert_eq!((r["panel"].as_str(), r["persistent"].as_bool()), (Some(other.as_str()), Some(true)));
        assert!(!is_open(&next.ui, &brand) && is_open(&next.ui, &other));
        assert!(next.ui.library_panels.iter().all(|p| p.panel() != brand), "the missing panel is forgotten");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn locate_library_leaves_a_panel_whose_library_is_there_alone() {
        let dir = std::env::temp_dir().join(format!("vc-locate-found-library-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let gpl = dir.join("Brand.gpl");
        std::fs::write(&gpl, "GIMP Palette\nName: Brand\n#\n255   0   0\tRed\n").unwrap();
        let mut app = app();
        let pastels = panel_of(app.run("window.swatchLibrary", json!({"library": "pastels"})).unwrap());
        for persistent in [false, true] {
            app.run("ui.libraryPanel", json!({"panel": pastels, "persistent": persistent})).unwrap();
            let e = app.run("ui.libraryPanel.locate", json!({"panel": pastels, "path": gpl.to_string_lossy()})).unwrap_err();
            assert!(e.contains("isn't missing its library"), "{e}");
            assert!(is_open(&app.ui, &pastels), "persistent: {persistent}");
            assert_eq!(settings(&app.ui, "swatches", "pastels").map(|s| s.persistent), Some(persistent));
        }
        assert!(SwatchLibraries::get(&app, &format!("loaded/{}", gpl.to_string_lossy())).is_none(), "the file isn't loaded");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn locate_library_reads_a_missing_library_whose_panel_is_no_longer_persistent() {
        let dir = std::env::temp_dir().join(format!("vc-locate-not-persistent-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("moved")).unwrap();
        let palette = "GIMP Palette\nName: Brand\n#\n255   0   0\tRed\n";
        let gpl = dir.join("Brand.gpl");
        std::fs::write(&gpl, palette).unwrap();
        let mut app = app();
        let brand = panel_of(app.run("window.swatchLibrary.other", json!({"path": gpl.to_string_lossy()})).unwrap());
        app.run("ui.libraryPanel", json!({"panel": brand, "persistent": true})).unwrap();
        std::fs::remove_file(&gpl).unwrap();
        let mut next = restarted(&app);
        // After a command turns Persistent off, the panel stays open on its missing library until the next frame's tidy.
        next.run("ui.libraryPanel", json!({"panel": brand, "persistent": false})).unwrap();
        assert!(is_open(&next.ui, &brand));
        let moved = dir.join("moved").join("Brand.gpl");
        std::fs::write(&moved, palette).unwrap();
        let r = next.run("ui.libraryPanel.locate", json!({"panel": brand, "path": moved.to_string_lossy()})).unwrap();
        let located = panel_id("swatches", &format!("loaded/{}", moved.to_string_lossy()));
        assert_eq!((r["panel"].as_str(), r["persistent"].as_bool()), (Some(located.as_str()), Some(false)));
        assert!(!is_open(&next.ui, &brand) && is_open(&next.ui, &located), "{}", next.ui.status);
        let (_, id) = parse(&located).unwrap();
        assert!(SwatchLibraries::get(&next, id).is_some_and(|(_, lib)| lib.swatch("Red").is_some()));
        tidy(&mut next);
        assert!(is_open(&next.ui, &located), "its library is there now");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn an_open_panel_whose_library_is_missing_is_brought_forward_by_its_id_when_not_persistent() {
        let dir = std::env::temp_dir().join(format!("vc-open-not-persistent-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let gpl = dir.join("Brand.gpl");
        std::fs::write(&gpl, "GIMP Palette\nName: Brand\n#\n255   0   0\tRed\n").unwrap();
        let mut app = app();
        let brand = panel_of(app.run("window.swatchLibrary.other", json!({"path": gpl.to_string_lossy()})).unwrap());
        app.run("ui.libraryPanel", json!({"panel": brand, "persistent": true})).unwrap();
        std::fs::remove_file(&gpl).unwrap();
        let mut next = restarted(&app);
        // After a command turns Persistent off, the panel stays open on its missing library until the next frame's tidy.
        next.run("ui.libraryPanel", json!({"panel": brand, "persistent": false})).unwrap();
        let (_, gone) = parse(&brand).unwrap();
        let r = next.run("window.swatchLibrary", json!({"library": gone})).unwrap();
        assert_eq!((r["panel"].as_str(), r["name"].as_str(), r["count"].as_u64()), (Some(brand.as_str()), Some("Brand"), Some(0)));
        assert!(is_open(&next.ui, &brand));
        tidy(&mut next);
        assert!(!is_open(&next.ui, &brand), "a panel that isn't persistent still closes");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn previous_and_next_show_the_neighbouring_library_in_the_same_panel() {
        let mut app = app();
        let first = panel_of(app.run("window.swatchLibrary", json!({"library": "pastels"})).unwrap());
        app.run("window.panel.float", json!({"panel": first, "x": 50, "y": 60})).unwrap();
        app.run("ui.libraryPanel", json!({"panel": first, "persistent": true, "view": "smallList"})).unwrap();
        step::<SwatchLibraries>(&mut app, "pastels", 1);
        let next = panel_id("swatches", SWATCH_IDS[5]);
        assert!(!is_open(&app.ui, &first) && is_open(&app.ui, &next), "{}", app.ui.status);
        let group = docking(&app).floating.iter().find(|g| g.panels.contains(&next)).unwrap();
        assert_eq!((group.panels.len(), group.rect[0], group.rect[1]), (1, 50.0, 60.0), "in its place");
        let s = settings(&app.ui, "swatches", SWATCH_IDS[5]).unwrap();
        assert_eq!((s.view, s.persistent), (Some(View::SmallList), true), "the panel keeps its settings");
        assert!(settings(&app.ui, "swatches", "pastels").is_none());
        assert!(crate::panel_docking::valid(docking(&app)));
        // A library open in a panel of its own is brought forward instead.
        app.run("window.swatchLibrary", json!({"library": "pastels"})).unwrap();
        let r = app.run("ui.libraryPanel", json!({"panel": next, "library": "Pastels"})).unwrap();
        assert_eq!(r["panel"], json!(first));
        assert!(is_open(&app.ui, &next) && is_open(&app.ui, &first));
    }

    #[test]
    fn a_panel_closes_when_its_library_is_gone() {
        let dir = std::env::temp_dir().join(format!("vc-gone-library-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("Gone.gpl"), "GIMP Palette\nName: Gone\n#\n0 0 0\tInk\n").unwrap();
        std::fs::write(dir.join("Kept.gpl"), "GIMP Palette\nName: Kept\n#\n0 0 0\tInk\n").unwrap();
        let mut app = app();
        app.session.swatch_libraries.set_user_dir(Some(dir.to_string_lossy().to_string()));
        let panel = panel_of(app.run("window.swatchLibrary", json!({"library": "user/Gone.gpl"})).unwrap());
        let kept = panel_of(app.run("window.swatchLibrary", json!({"library": "user/Kept.gpl"})).unwrap());
        app.run("ui.libraryPanel", json!({"panel": kept, "persistent": true})).unwrap();
        tidy(&mut app);
        assert!(is_open(&app.ui, &panel));
        std::fs::remove_file(dir.join("Gone.gpl")).unwrap();
        std::fs::remove_file(dir.join("Kept.gpl")).unwrap();
        app.session.swatch_libraries.rescan();
        tidy(&mut app);
        assert!(!is_open(&app.ui, &panel));
        // A persistent panel stays open and shows its library as missing.
        assert!(is_open(&app.ui, &kept));
        assert_eq!(tab_label(&app, &kept).as_deref(), Some("Kept"));
        let text = crate::tests_labels::painted_text(&mut app, |app, ui| show(app, ui, &kept));
        assert!(text.contains("Missing: user/Kept.gpl"), "{text}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_panel_menu_has_persistent() {
        let mut app = app();
        let panel = panel_of(app.run("window.swatchLibrary", json!({"library": "pastels"})).unwrap());
        let text = crate::tests_labels::painted_text(&mut app, |app, ui| menu(app, ui, &panel));
        for item in ["Add to Swatches", "Small List View", "Show Find Field", "Previous Library", "Persistent", "Close Library"] {
            assert!(text.contains(item), "{item}: {text}");
        }
    }
}
