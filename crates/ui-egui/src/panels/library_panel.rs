//! Library panels: read-only views of one library each (swatch, graphic style, symbol and brush
//! libraries; any kind of library implementing [`LibraryKind`]) with thumbnail and list views, a
//! find field, the libraries menu and previous / next arrows. A plain click on an item adds it to
//! the document and applies it; Shift and Cmd/Ctrl-clicks select items (and folders) for Add to ….
//! A kind whose items drag ([`LibraryKind::drag`]) lets them be dragged out of the panel.
//!
//! As in Illustrator (<https://helpx.adobe.com/illustrator/using/using-creating-swatches.html>,
//! "Swatch libraries"), each open library is a panel of its own, `library:<kind>:<library id>`
//! ([`panel_id`]), that floats, tabs, docks and closes as any panel of the shared docking system:
//! `window.swatchLibrary`, `window.graphicStyleLibrary`, `window.symbolLibrary` and `window.brushLibrary` open one, or bring it forward when it is
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
    crate::panel_docking::layout_of(ui).contains(&panel.to_string())
}

/// The open library panels.
fn open_panels(ui: &UiState) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    let docked = ui.docking.iter().flat_map(|l| l.panels());
    for p in docked {
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
    let mut docking = ui.docking.as_ref().map(|l| renamed(l, from, to)).transpose()?;
    // A floating window keeps its id, and with it its OS window.
    if let (Some(new), Some(old)) = (docking.as_mut(), ui.docking.as_ref()) {
        for (window, was) in new.floating.iter_mut().zip(&old.floating) {
            window.id = was.id;
        }
    }
    let mut hidden = std::collections::BTreeMap::new();
    for (p, at) in &ui.docking_hidden {
        hidden.insert(if p == from { to.to_string() } else { p.clone() }, renamed(at, from, to)?);
    }
    ui.docking = docking;
    ui.docking_hidden = hidden;
    if ui.open_panel.as_deref() == Some(from) {
        ui.open_panel = Some(to.into());
    }
    Ok(())
}

/// The interaction id of the tile or row of `name` in panel `panel`.
pub(crate) fn tile_id(panel: &str, name: &str) -> egui::Id {
    egui::Id::unique(("library-tile", panel, name))
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
/// (symbol and brush libraries: any document).
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
    let area = egui::Area::new(egui::Id::unique("test-library-panel")).fixed_pos(egui::pos2(20.0, 20.0));
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
    let query = if hide_find { String::new() } else { widgets::search_field(ui, egui::Id::unique(key(&panel, "find")), tl!("Find")) };
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
mod tests;
