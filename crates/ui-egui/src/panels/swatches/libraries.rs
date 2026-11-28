use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::Arc;

use craft::ui::contract::{Entry, MemoKey, MemoProps, Show as _};
use craft::ui::display::Divider;
use craft::ui::patterns::presets::PresetRow;
use craft::ui::primitives::ContentPainterCx;
use egui::{Rect, Ui};
use serde_json::{Value, json};
use vectorcraft_color::libraries::{BuiltinLibrary, GRADIENT_LIBRARIES, SWATCH_LIBRARIES};
use vectorcraft_color::{Swatch, SwatchLibrary};
use vectorcraft_doc::Document;
use vectorcraft_engine::cmd::patternlib::PATTERN_LIBRARIES;
use vectorcraft_engine::cmd::swatchlib;

use super::actions::deletable;
use super::library_panel::{self, LibraryKey, LibraryKind, LibraryRef, Row};
use super::{describe, item, kind_icons, paint_swatch, selected};
use crate::VectorcraftApp;
use crate::menus::{Node, Sep, c, cp, sub};

fn library_refs(libs: Vec<swatchlib::LibraryInfo>) -> Vec<LibraryRef> {
    libs.into_iter().map(|l| LibraryRef { submenu: library_panel::submenu(l.category), id: l.id, name: l.name }).collect()
}

/// The swatch libraries with solid colours (not the gradient-only ones), for Limit to Library.
pub(crate) fn colour_libraries(app: &VectorcraftApp) -> Vec<LibraryRef> {
    let has_colours = |id: &str| swatchlib::library(&app.session, id).is_some_and(|(_, lib)| lib.iter().any(|w| w.paint.color().is_some()));
    library_refs(swatchlib::libraries(&app.session).into_iter().filter(|l| l.category != "gradients" && has_colours(&l.id)).collect())
}

/// The library choices of a `Select`: the document's swatches, a separator, the colour libraries at
/// the top level of the library menu, then each submenu's libraries under its heading.
pub(crate) fn library_entries(app: &VectorcraftApp) -> Vec<Entry<String>> {
    let libs = colour_libraries(app);
    let entry = |l: &LibraryRef| Entry::new(l.id.clone(), library_panel::library_name(&l.id, &l.name).to_string());
    let mut entries = vec![Entry::new(swatchlib::DOCUMENT_SWATCHES.to_string(), tl!(DOCUMENT_SWATCHES)), Entry::separator()];
    entries.extend(libs.iter().filter(|l| l.submenu.is_none()).map(entry));
    let mut submenus: Vec<&'static str> = Vec::new();
    for submenu in libs.iter().filter_map(|l| l.submenu) {
        if !submenus.contains(&submenu) {
            submenus.push(submenu);
        }
    }
    for submenu in submenus {
        entries.push(Entry::heading(tl!(submenu)));
        entries.extend(libs.iter().filter(|l| l.submenu == Some(submenu)).map(entry));
    }
    entries
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

/// The documents a library's pattern swatches draw from, by swatch name.
pub(crate) struct PatternSources(HashMap<String, Arc<Document>>);

/// Covered by the browser's rows: they name the library, whose pattern documents stay as it loaded them.
impl MemoProps for PatternSources {
    fn key(&self, _: &mut MemoKey) {}
}

impl LibraryKind for SwatchLibraries {
    const KIND: &'static str = "swatches";
    const OPEN: &'static str = "window.swatchLibrary";
    const LIST: &'static str = "swatch.library.list";
    const LOAD: &'static str = "swatch.library.load";
    const MENU: &'static str = "Swatch Libraries Menu";
    const ADD: &'static str = "Add to Swatches";
    type Lib = Arc<SwatchLibrary>;
    type Item = Swatch;
    type Thumbs = PatternSources;

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
    /// Ungrouped swatches, then each colour group's folder and swatches, as in the Swatches panel; the browser's Find
    /// narrows them.
    fn rows<'a>(lib: &'a Self::Lib, _id: &str) -> Vec<Row<'a, Swatch>> {
        let row = |w: &'a Swatch| Row { name: &w.name, item: Some(w) };
        let mut out: Vec<Row<Swatch>> = lib.swatches.iter().map(row).collect();
        for g in &lib.groups {
            if !g.swatches.is_empty() {
                out.push(Row { name: &g.name, item: None });
                out.extend(g.swatches.iter().map(row));
            }
        }
        out
    }
    /// A built-in library's swatch or color group in the UI language, a numbered one ("Warm Gray
    /// 3") with its number as it is; a User Defined or loaded library's as it is named.
    fn label<'a>(id: &str, name: &'a str) -> Cow<'a, str> {
        let built_in = SWATCH_LIBRARIES.iter().chain(GRADIENT_LIBRARIES).any(|b| b.id == id) || PATTERN_LIBRARIES.iter().any(|b| b.id == id);
        if built_in { crate::i18n::numbered_label(crate::i18n::current(), name) } else { Cow::Borrowed(name) }
    }
    /// The browser's Find matches the shown name or the stored one (`search_name`).
    fn row(id: &str, w: &Swatch, label: &str) -> PresetRow<LibraryKey> {
        let key = LibraryKey { panel: library_panel::panel_id(Self::KIND, id), name: w.name.clone(), folder: false };
        let mut row = PresetRow::item(key, label, 0);
        row.trailing = kind_icons(&w.paint, w.global, w.spot);
        row.tooltip = Some(format!("{label} ({})", describe(&w.paint, w.global, w.spot)));
        row.search_name = Some(w.name.clone());
        row
    }
    fn thumbs(app: &VectorcraftApp, lib: &Self::Lib) -> PatternSources {
        PatternSources(lib.iter().filter_map(|w| Some((w.name.clone(), swatchlib::pattern_source(&app.session, w)?))).collect())
    }
    /// A pattern library's pattern, or a loaded library's, drawn from the library.
    fn thumb(sources: &PatternSources, cx: &ContentPainterCx<'_>, rect: Rect, w: &Swatch) {
        paint_swatch(cx, rect, &w.paint, false, (w.global, w.spot), sources.0.get(&w.name).map(|d| &**d));
    }
    fn drags() -> bool {
        true
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
        if item(ui, tl!("Default Swatches"), app.session.active().is_some(), None) {
            app.run("swatch.resetDefaults", json!({})).ok();
        }
        Divider::horizontal().show(ui);
    }
    fn menu_tail(app: &mut VectorcraftApp, ui: &mut Ui) {
        Divider::horizontal().show(ui);
        // As a command, so its file dialog, shown off the UI thread, loads the library when it
        // answers.
        if item(ui, tl!("Other Library…"), true, None)
            && let Err(e) = app.run("window.swatchLibrary.other", json!({}))
        {
            app.status(e);
        }
        if item(ui, tl!("Save Swatch Library…"), app.session.active().is_some(), None) {
            let names = deletable(app, &selected(app));
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

/// Other Library…: load the library (or document) at `path`, else one picked in an open dialog.
pub(crate) fn other_library(app: &mut VectorcraftApp, path: Option<String>) -> Result<Value, String> {
    library_panel::pick_library_file(app, path, library_panel::library_pick_filters(SwatchLibraries::KIND), load_library)
}

/// The id prefix of the Window → Swatch Libraries → User Defined slots.
pub(crate) const USER_SLOT: &str = "window.userSwatchLibrary";

/// The User Defined library slot `id` (`window.userSwatchLibrary3`) stands for.
pub(crate) fn user_library(app: &VectorcraftApp, id: &str) -> Option<swatchlib::LibraryInfo> {
    library_panel::user_slot(id, USER_SLOT, swatchlib::libraries(&app.session))
}

/// Window → Swatch Libraries.
pub(crate) fn window_menu() -> Vec<Node> {
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
    let open = |b: &'static BuiltinLibrary| cp(b.name, "window.swatchLibrary", json!({ "library": b.id }));
    let patterns = PATTERN_LIBRARIES.iter().map(open_pattern).collect();
    let mut items = vec![c("Default Swatches", "swatch.resetDefaults"), Sep];
    items.extend(SWATCH_LIBRARIES.iter().map(open));
    items.extend([
        sub("Gradients", GRADIENT_LIBRARIES.iter().map(open).collect()),
        sub("Patterns", patterns),
        Sep,
        sub("User Defined", SLOTS.iter().map(|id| c("User Library", id)).collect()),
        Sep,
        c("Other Library…", "window.swatchLibrary.other"),
        c("Save Swatch Library…", "ui.saveSwatchLibrary"),
    ]);
    items
}

fn open_pattern(b: &vectorcraft_engine::cmd::patternlib::BuiltinPatternLibrary) -> Node {
    cp(b.name, "window.swatchLibrary", json!({ "library": b.id }))
}
