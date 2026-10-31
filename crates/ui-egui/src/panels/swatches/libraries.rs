use super::actions::deletable;
use super::dnd::selection;
use super::tiles::{describe, global_mark, list_icons};
use super::{
    Arc, BuiltinLibrary, GRADIENT_LIBRARIES, Item, LibraryKind, LibraryRef, Rect, Row, SWATCH_LIBRARIES, Swatch, SwatchLibrary, Ui, Value,
    VectorcraftApp, json, library_panel, menu_item, swatch_tile, swatchlib,
};

fn library_refs(libs: Vec<swatchlib::LibraryInfo>) -> Vec<LibraryRef> {
    libs.into_iter().map(|l| LibraryRef { submenu: library_panel::submenu(l.category), id: l.id, name: l.name }).collect()
}

/// The swatch libraries with solid colours (not the gradient-only ones), for Limit to Library.
pub(crate) fn colour_libraries(app: &VectorcraftApp) -> Vec<LibraryRef> {
    let has_colours = |id: &str| swatchlib::library(&app.session, id).is_some_and(|(_, lib)| lib.iter().any(|w| w.paint.color().is_some()));
    library_refs(swatchlib::libraries(&app.session).into_iter().filter(|l| l.category != "gradients" && has_colours(&l.id)).collect())
}

/// What Limit to Library menus call the document's own swatches ([`swatchlib::DOCUMENT_SWATCHES`]).
pub(crate) const DOCUMENT_SWATCHES: &str = "Document Swatches";

/// The name of Limit to Library choice `key` (a library id or [`swatchlib::DOCUMENT_SWATCHES`]);
/// `None` for no limit or a library that is gone.
pub(crate) fn limit_name(app: &VectorcraftApp, key: &str) -> Option<String> {
    match key {
        "" => None,
        swatchlib::DOCUMENT_SWATCHES => Some(DOCUMENT_SWATCHES.into()),
        _ => swatchlib::library(&app.session, key).map(|(info, _)| info.name),
    }
}

/// The Limit to Library key for `l`: a library's id (from its id or name), or
/// [`swatchlib::DOCUMENT_SWATCHES`] as it is.
pub(crate) fn limit_key(app: &VectorcraftApp, l: &str) -> Result<String, String> {
    if l == swatchlib::DOCUMENT_SWATCHES {
        return Ok(l.into());
    }
    swatchlib::library(&app.session, l).map(|(info, _)| info.id).ok_or_else(|| format!("no swatch library `{l}` (see swatch.library.list)"))
}

/// Swatch libraries in library panels (engine: `swatch.library.*`).
pub(crate) struct SwatchLibraries;

impl LibraryKind for SwatchLibraries {
    const KIND: &'static str = "swatches";
    const OPEN: &'static str = "window.swatchLibrary";
    const LIST: &'static str = "swatch.library.list";
    const LOAD: &'static str = "swatch.library.load";
    const MENU: &'static str = "Swatch Libraries Menu";
    const ADD: &'static str = "Add to Swatches";
    type Lib = Arc<SwatchLibrary>;
    type Item = Swatch;

    fn list(app: &VectorcraftApp) -> Vec<LibraryRef> {
        library_refs(swatchlib::libraries(&app.session))
    }
    fn get(app: &VectorcraftApp, id: &str) -> Option<(String, Self::Lib)> {
        swatchlib::library(&app.session, id).map(|(info, lib)| (info.name, lib))
    }
    fn find(app: &VectorcraftApp, key: &str) -> Option<(String, String, usize)> {
        swatchlib::library(&app.session, key).map(|(info, lib)| (info.id, info.name, lib.len()))
    }
    fn path(app: &VectorcraftApp, id: &str) -> Option<String> {
        app.session.swatch_libraries.path(id).map(str::to_string)
    }
    /// Ungrouped swatches, then each colour group's folder and swatches (a group whose name
    /// matches shows all of them), as in the Swatches panel.
    fn rows<'a>(lib: &'a Self::Lib, query: &str) -> Vec<Row<'a, Swatch>> {
        let found = |n: &str| query.is_empty() || n.to_lowercase().contains(query);
        let row = |w: &'a Swatch| Row { name: &w.name, item: Some(w) };
        let mut out: Vec<Row<Swatch>> = lib.swatches.iter().filter(|w| found(&w.name)).map(row).collect();
        for g in &lib.groups {
            let all = found(&g.name);
            let mut items = g.swatches.iter().filter(|w| all || found(&w.name)).map(row).peekable();
            if items.peek().is_some() {
                out.push(Row { name: &g.name, item: None });
                out.extend(items);
            }
        }
        out
    }
    fn draw(_: &VectorcraftApp, ui: &Ui, r: Rect, w: &Swatch, selected: bool, hovered: bool) {
        swatch_tile(ui, r, &w.paint, selected, hovered);
        if w.global {
            global_mark(ui, r, w.spot);
        }
    }
    fn row_icons(ui: &Ui, r: Rect, w: &Swatch) {
        list_icons(ui, r, &w.paint, w.global, w.spot);
    }
    fn describe(w: &Swatch) -> String {
        describe(&w.paint, w.global, w.spot)
    }
    /// Adds the swatch and applies it to the active proxy (Alt: the inactive one), as one step. A
    /// color swatch also becomes the Color Guide's base color.
    fn click(app: &mut VectorcraftApp, ui: &Ui, id: &str, name: &str) {
        if let Some(c) = swatchlib::library(&app.session, id).and_then(|(_, lib)| lib.iter().find(|w| w.name == name).and_then(|w| w.paint.color())) {
            crate::panels::color_guide::choose_base(app, c);
        }
        let alt = crate::panels::alt_held(ui);
        let apply = if crate::panels::proxy_cmd(app, alt) == "paint.setFill" { "fill" } else { "stroke" };
        add_from_library(app, json!({"library": id, "names": [name], "apply": apply, "focus": !alt}));
    }
    fn add(app: &mut VectorcraftApp, id: &str, names: Vec<String>) {
        add_from_library(app, json!({"library": id, "names": names}));
    }
    fn menu_head(app: &mut VectorcraftApp, ui: &mut Ui) {
        if menu_item(ui, tl!("Default Swatches"), app.session.active().is_some(), false) {
            app.run("swatch.resetDefaults", json!({})).ok();
        }
        ui.separator();
    }
    fn menu_tail(app: &mut VectorcraftApp, ui: &mut Ui) {
        ui.separator();
        // As a command, so its file dialog, shown off the UI thread, loads the library when it
        // answers.
        if menu_item(ui, tl!("Other Library…"), true, false)
            && let Err(e) = app.run("window.swatchLibrary.other", json!({}))
        {
            app.status(e);
        }
        if menu_item(ui, tl!("Save Swatch Library…"), app.session.active().is_some(), false) {
            let names = deletable(&selection(app, ui));
            app.run("ui.saveSwatchLibrary", json!({ "names": names })).ok();
        }
    }
}

fn add_from_library(app: &mut VectorcraftApp, params: Value) {
    if let Err(e) = app.run("swatch.library.add", params) {
        app.status(e);
    }
}

/// `window.swatchLibrary {library}`: open a swatch library in its own panel (`library` null closes
/// every swatch library panel).
pub(crate) fn open_library(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    library_panel::open_command::<SwatchLibraries>(app, p)
}

/// `swatch.library.load` params, then open the library in its panel.
pub(crate) fn load_library(app: &mut VectorcraftApp, params: Value) -> Result<Value, String> {
    let r = app.run("swatch.library.load", params)?;
    open_library(app, &json!({ "library": r["library"] }))
}

/// Other Library…: load the library (or document) at `path`, else one picked in an open dialog
/// (on the web the picked file arrives later and opens through [`crate::io::open_bytes`]).
pub(crate) fn other_library(app: &mut VectorcraftApp, path: Option<String>) -> Result<Value, String> {
    match library_panel::pick_library_file(app, path, library_panel::library_pick_filters(SwatchLibraries::KIND))? {
        Some(path) => load_library(app, json!({ "path": path })),
        None => Ok(Value::Null),
    }
}

/// The id prefix of the Window → Swatch Libraries → User Defined slots.
pub(crate) const USER_SLOT: &str = "window.userSwatchLibrary";

/// The User Defined library slot `id` (`window.userSwatchLibrary3`) stands for.
pub(crate) fn user_library(app: &VectorcraftApp, id: &str) -> Option<swatchlib::LibraryInfo> {
    library_panel::user_slot(id, USER_SLOT, swatchlib::libraries(&app.session))
}

/// Window → Swatch Libraries.
pub(crate) fn window_menu() -> Vec<Item> {
    const SLOTS: [&str; 10] = [
        "window.userSwatchLibrary1",
        "window.userSwatchLibrary2",
        "window.userSwatchLibrary3",
        "window.userSwatchLibrary4",
        "window.userSwatchLibrary5",
        "window.userSwatchLibrary6",
        "window.userSwatchLibrary7",
        "window.userSwatchLibrary8",
        "window.userSwatchLibrary9",
        "window.userSwatchLibrary10",
    ];
    let open = |b: &'static BuiltinLibrary| Item::Cmd(b.name, "window.swatchLibrary", json!({ "library": b.id }));
    let mut items = vec![Item::Cmd("Default Swatches", "swatch.resetDefaults", Value::Null), Item::Sep];
    items.extend(SWATCH_LIBRARIES.iter().map(open));
    items.extend([
        Item::Sub("Gradients", GRADIENT_LIBRARIES.iter().map(open).collect()),
        Item::Sep,
        Item::Sub("User Defined", SLOTS.iter().map(|id| Item::Cmd("User Library", id, Value::Null)).collect()),
        Item::Sep,
        Item::Cmd("Other Library…", "window.swatchLibrary.other", Value::Null),
        Item::Cmd("Save Swatch Library…", "ui.saveSwatchLibrary", Value::Null),
    ]);
    items
}
