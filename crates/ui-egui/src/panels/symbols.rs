//! Symbols panel: the document's symbols as rendered thumbnails or a list. Click selects a symbol
//! (it becomes the Symbol Sprayer's current symbol); double-click places an instance. Symbol
//! libraries open in library panels ([`SymbolLibraries`]).

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

use egui::{Sense, Stroke, StrokeKind, Ui, vec2};
use serde_json::{Value, json};
use vectorcraft_doc::{Document, Node, NodeKind};
use vectorcraft_engine::cmd::swatchlib::LibraryInfo;
use vectorcraft_engine::cmd::symbollib::{self, SymbolLibrary};

use super::library_panel::{self, LibraryKind, LibraryRef, Row};
use super::{pstate, set_pstate};
use crate::VectorcraftApp;
use crate::menus::Item;
use crate::theme::Tokens;
use crate::widgets::{self, PanelDrag, menu_item};

/// A thumbnail of symbol `name` (an instance at its natural size), cached by the identity of the
/// art it shows: its own and `resources`, [`resources_key`] of `doc`.
fn thumb(ui: &Ui, doc: &Document, resources: u64, name: &str, size: f32) -> Option<egui::TextureHandle> {
    thread_local! {
        static RENDERER: RefCell<vectorcraft_render::Renderer> = RefCell::new(vectorcraft_render::Renderer::new());
        static CACHE: crate::graphics::TexCache<HashMap<(usize, u64, String, u32), egui::TextureHandle>> = crate::graphics::TexCache::default();
    }
    let sym = doc.symbols.iter().find(|s| s.name == name)?;
    let px = (size * ui.ctx().pixels_per_point()).round().max(8.0) as u32;
    let key = (Arc::as_ptr(&sym.art) as usize, resources, name.to_string(), px);
    if let Some(t) = CACHE.with(|c| c.borrow().get(&key).cloned()) {
        return Some(t);
    }
    let img = RENDERER.with(|r| render_thumb(&mut r.borrow_mut(), doc, name, px))?;
    let color = crate::canvas::color_image(&img);
    let tex = ui.ctx().load_texture(format!("sym-{name}-{px}"), color, egui::TextureOptions::LINEAR);
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        if c.len() > 512 {
            c.clear();
        }
        c.insert(key, tex.clone());
    });
    Some(tex)
}

/// The identity of `doc`'s symbol and pattern art, which a symbol's thumbnail may show. It takes a
/// pass over them: work it out once per panel frame, not per thumbnail.
pub(crate) fn resources_key(doc: &Document) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for s in &doc.symbols {
        (Arc::as_ptr(&s.art) as usize).hash(&mut h);
    }
    for p in &doc.patterns {
        p.art.iter().for_each(|a| (Arc::as_ptr(a) as usize).hash(&mut h));
    }
    h.finish()
}

/// Symbol `name` of `doc` rendered `px` pixels wide: an instance at its natural size, alone on a
/// copy of `doc` that keeps its resources (the symbols it nests, patterns, images, swatches,
/// brushes, graphic styles) but none of its art.
fn render_thumb(r: &mut vectorcraft_render::Renderer, doc: &Document, name: &str, px: u32) -> Option<vectorcraft_render::Rendered> {
    let (w, h) = symbol_size(doc, name);
    let mut d = doc.clone();
    let layer = d.alloc_id();
    d.layers = vec![Arc::new(Node::layer(layer, "Preview", vectorcraft_doc::LayerColor::Preset(0)))];
    let id = d.alloc_id();
    let xf = vectorcraft_geom::Affine::scale_non_uniform(w / 20.0, h / 20.0);
    d.insert(Some(layer), 0, Node::new(id, NodeKind::SymbolInstance { symbol: name.into(), xf })).ok()?;
    r.render_thumbnail(&d, id, px)
}

/// Symbol `name`'s thumbnail on white in `r` (the panel's tiles and the chip a dragged symbol
/// shows at the pointer); `resources` is [`resources_key`] of `doc`.
pub(crate) fn chip(ui: &Ui, doc: &Document, resources: u64, r: egui::Rect, name: &str) {
    ui.painter().rect_filled(r, 0.0, egui::Color32::WHITE);
    if let Some(tex) = thumb(ui, doc, resources, name, r.width()) {
        ui.painter().image(tex.id(), r, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
    }
}

fn current(app: &mut VectorcraftApp) -> Option<String> {
    app.run("symbol.list", json!({})).ok().and_then(|v| v["current"].as_str().map(str::to_string))
}

fn has_instances_selected(app: &VectorcraftApp) -> bool {
    app.session.active().is_some_and(|st| {
        st.selection.objects.iter().any(|id| {
            let mut any = false;
            if let Some(n) = st.doc.node(*id) {
                n.walk(&mut |c| any |= matches!(c.kind, NodeKind::SymbolInstance { .. }));
            }
            any
        })
    })
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let Some(doc) = app.session.active().map(|d| d.doc.clone()) else { return };
    let names: Vec<String> = doc.symbols.iter().map(|s| s.name.clone()).collect();
    let resources = resources_key(&doc);
    let list: bool = pstate(ui.ctx(), "sym-list");
    let sel = current(app);
    let mut clicked: Option<(String, bool)> = None;
    let list_rect = widgets::list_box(ui, |ui| {
        ui.set_min_height(110.0);
        ui.set_width(ui.available_width());
        if names.is_empty() {
            super::empty_state(ui, "spray-can", tl!("No symbols in this document"), tl!("Select art and click New Symbol to make one."));
            return ui.min_rect();
        }
        if list {
            for n in &names {
                let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::click_and_drag());
                // Dragged onto the canvas, the symbol is placed there.
                widgets::drag_source(ui, &resp, || PanelDrag::Symbol(n.clone()));
                if sel.as_deref() == Some(n.as_str()) {
                    ui.painter().rect_filled(r, 0.0, t.row_selected);
                } else if resp.hovered() {
                    ui.painter().rect_filled(r, 0.0, t.hover);
                }
                chip(ui, &doc, resources, egui::Rect::from_center_size(r.left_center() + vec2(14.0, 0.0), vec2(22.0, 22.0)), n);
                ui.painter().text(r.left_center() + vec2(32.0, 0.0), egui::Align2::LEFT_CENTER, n, egui::FontId::proportional(12.5), t.text);
                if resp.double_clicked() {
                    clicked = Some((n.clone(), true));
                } else if resp.clicked() {
                    clicked = Some((n.clone(), false));
                }
            }
        } else {
            ui.horizontal_wrapped(|ui| {
                for n in &names {
                    let (r, resp) = ui.allocate_exact_size(vec2(44.0, 44.0), Sense::click_and_drag());
                    widgets::drag_source(ui, &resp, || PanelDrag::Symbol(n.clone()));
                    chip(ui, &doc, resources, r.shrink(2.0), n);
                    let on = sel.as_deref() == Some(n.as_str());
                    ui.painter().rect_stroke(
                        r,
                        0.0,
                        Stroke::new(if on { 2.0 } else { 1.0 }, if on { t.accent } else { t.border }),
                        StrokeKind::Inside,
                    );
                    let resp = resp.on_hover_text(n);
                    if resp.double_clicked() {
                        clicked = Some((n.clone(), true));
                    } else if resp.clicked() {
                        clicked = Some((n.clone(), false));
                    }
                }
            });
        }
        ui.min_rect()
    });
    let zone = ui.interact(list_rect, ui.id().with("symbols-drop"), Sense::hover());
    if let Some(ids) = widgets::art_drop(ui, &zone)
        && let Err(e) = app.run("symbol.new", json!({ "ids": ids }))
    {
        app.status(e);
    }
    if let Some((n, place)) = clicked {
        app.run("symbol.setCurrent", json!({"name": n})).ok();
        if place {
            app.run("symbol.place", json!({"name": n})).ok();
        }
    }
    let has_sel = app.session.active().is_some_and(|d| !d.selection.is_empty());
    let inst = has_instances_selected(app);
    widgets::bottom_bar(ui, |ui| {
        let open = library_panel::any_open::<SymbolLibraries>(&app.ui);
        let lr = widgets::icon_button(ui, "library", SymbolLibraries::MENU, open, 24.0);
        egui::Popup::menu(&lr).show(|ui| {
            ui.set_min_width(200.0);
            library_panel::library_menu::<SymbolLibraries>(app, ui);
        });
        if widgets::icon_button_enabled(ui, "dc-place-symbol", tl!("Place Symbol Instance"), false, sel.is_some(), 24.0).clicked() {
            app.run("symbol.place", json!({})).ok();
        }
        if widgets::icon_button_enabled(ui, "link-2-off", tl!("Break Link to Symbol"), false, inst, 24.0).clicked() {
            app.run("symbol.breakLink", json!({})).ok();
        }
        ui.add_space((ui.available_width() - 2.0 * 28.0).max(0.0));
        if widgets::icon_button_enabled(ui, "dc-new-item", tl!("New Symbol"), false, has_sel, 24.0).clicked() {
            // Alt skips Symbol Options: the symbol is made under the default name.
            let cmd = if ui.input(|i| i.modifiers.alt) { "symbol.new" } else { "ui.symbolOptions" };
            app.run(cmd, json!({})).ok();
        }
        if widgets::icon_button_enabled(ui, "trash-2", tl!("Delete Symbol"), false, sel.is_some(), 24.0).clicked()
            && let Some(n) = &sel
        {
            app.run("symbol.delete", json!({"name": n})).ok();
        }
    });
}

pub fn menu(app: &mut VectorcraftApp, ui: &mut Ui) {
    let sel = current(app);
    let has_sel = app.session.active().is_some_and(|d| !d.selection.is_empty());
    let inst = has_instances_selected(app);
    let name = json!({"name": sel});
    let items: [(&str, bool, &str, Value); 10] = [
        (tl!("New Symbol…"), has_sel, "ui.symbolOptions", json!({})),
        (tl!("Redefine Symbol"), has_sel && sel.is_some(), "symbol.update", name.clone()),
        (tl!("Duplicate Symbol"), sel.is_some(), "symbol.duplicate", name.clone()),
        (tl!("Delete Symbol"), sel.is_some(), "symbol.delete", name.clone()),
        (tl!("Symbol Options…"), sel.is_some(), "ui.symbolOptions", json!({"edit": true, "name": sel})),
        (tl!("Edit Symbol"), inst, "symbol.edit", json!({})),
        (tl!("Place Symbol Instance"), sel.is_some(), "symbol.place", name.clone()),
        (tl!("Replace Symbol"), inst && sel.is_some(), "symbol.replace", name.clone()),
        (tl!("Break Link to Symbol"), inst, "symbol.breakLink", json!({})),
        (tl!("Select All Instances"), sel.is_some(), "symbol.selectInstances", name),
    ];
    for (label, enabled, cmd, params) in items {
        if menu_item(ui, label, enabled, false) {
            app.run(cmd, params).ok();
        }
    }
    ui.separator();
    let list: bool = pstate(ui.ctx(), "sym-list");
    if menu_item(ui, tl!("Thumbnail View"), true, !list) {
        set_pstate(ui.ctx(), "sym-list", false);
    }
    if menu_item(ui, tl!("List View"), true, list) {
        set_pstate(ui.ctx(), "sym-list", true);
    }
    ui.separator();
    ui.menu_button(tl!("Open Symbol Library"), |ui| library_panel::library_menu::<SymbolLibraries>(app, ui));
    let has_symbols = app.session.active().is_some_and(|d| !d.doc.symbols.is_empty());
    if menu_item(ui, tl!("Save Symbol Library…"), has_symbols, false) {
        save_library(app, sel.into_iter().collect());
    }
}

/// Save Symbol Library… for the document's symbols (`names`: the one selected in the panel).
fn save_library(app: &mut VectorcraftApp, names: Vec<String>) {
    if let Err(e) = app.run("ui.saveSymbolLibrary", json!({ "names": names })) {
        app.status(e);
    }
}

// ---------- libraries ----------

/// A symbol of a library as the library panel shows it.
pub(crate) struct LibSymbol {
    lib: Arc<SymbolLibrary>,
    index: usize,
    /// [`resources_key`] of the library's document.
    resources: u64,
}

impl LibSymbol {
    fn name(&self) -> &str {
        self.lib.doc.symbols.get(self.index).map_or("", |s| s.name.as_str())
    }
}

/// Symbol libraries in library panels (engine: `symbol.libraries`, `symbol.addFromLibrary`…).
pub(crate) struct SymbolLibraries;

impl LibraryKind for SymbolLibraries {
    const KIND: &'static str = "symbols";
    const OPEN: &'static str = "window.symbolLibrary";
    const LIST: &'static str = "symbol.libraries";
    const LOAD: &'static str = "symbol.loadLibrary";
    const MENU: &'static str = "Symbol Libraries Menu";
    const ADD: &'static str = "Add to Symbols";
    /// Symbols need room to show their art.
    const VIEW: super::swatches::View = super::swatches::View::LargeThumb;
    type Lib = Arc<Vec<LibSymbol>>;
    type Item = LibSymbol;

    fn list(app: &VectorcraftApp) -> Vec<LibraryRef> {
        let libs = symbollib::libraries(&app.session).into_iter();
        libs.map(|l| LibraryRef { submenu: library_panel::submenu(l.category), id: l.id, name: l.name }).collect()
    }
    fn get(app: &VectorcraftApp, id: &str) -> Option<(String, Self::Lib)> {
        let (info, lib) = symbollib::library(&app.session, id)?;
        let resources = resources_key(&lib.doc);
        let shown = (0..lib.len()).map(|index| LibSymbol { lib: lib.clone(), index, resources }).collect();
        Some((info.name, Arc::new(shown)))
    }
    fn find(app: &VectorcraftApp, key: &str) -> library_panel::Found {
        symbollib::library(&app.session, key).map(|(info, lib)| (info.id, info.name, lib.len()))
    }
    fn path(app: &VectorcraftApp, id: &str) -> Option<String> {
        app.session.symbol_libraries.path(id).map(str::to_string)
    }
    fn rows<'a>(lib: &'a Self::Lib, query: &str) -> Vec<Row<'a, LibSymbol>> {
        let found = |n: &str| query.is_empty() || n.to_lowercase().contains(query);
        lib.iter().filter(|s| found(s.name())).map(|s| Row { name: s.name(), item: Some(s) }).collect()
    }
    /// The symbol's art on white, as the Symbols panel shows it.
    fn draw(_: &VectorcraftApp, ui: &Ui, r: egui::Rect, s: &LibSymbol, selected: bool, hovered: bool) {
        chip(ui, &s.lib.doc, s.resources, r, s.name());
        super::graphic_styles::outline(ui, r, selected, hovered);
    }
    fn row_icons(_: &Ui, _: egui::Rect, _: &LibSymbol) {}
    /// Its natural size.
    fn describe(s: &LibSymbol) -> String {
        let (w, h) = symbol_size(&s.lib.doc, s.name());
        let num = |v: f64| format!("{}", (v * 100.0).round() / 100.0);
        crate::i18n::fmt(tl!("{w} × {h} pt"), &[("w", &num(w)), ("h", &num(h))])
    }
    /// Adds the symbol to the document's Symbols, as the current symbol.
    fn click(app: &mut VectorcraftApp, _: &Ui, id: &str, name: &str) {
        add(app, json!({"library": id, "name": name}));
    }
    fn add(app: &mut VectorcraftApp, id: &str, names: Vec<String>) {
        add(app, json!({"library": id, "names": names}));
    }
    /// Dragged onto the canvas, the symbol is added and an instance placed where it is dropped.
    fn drag(id: &str, name: &str) -> Option<PanelDrag> {
        Some(PanelDrag::LibrarySymbol { library: id.to_string(), name: name.to_string() })
    }
    fn menu_tail(app: &mut VectorcraftApp, ui: &mut Ui) {
        ui.separator();
        // As a command, so its file dialog, shown off the UI thread, loads the library when it
        // answers.
        if menu_item(ui, tl!("Other Library…"), true, false)
            && let Err(e) = app.run("window.symbolLibrary.other", json!({}))
        {
            app.status(e);
        }
        let has_symbols = app.session.active().is_some_and(|d| !d.doc.symbols.is_empty());
        if menu_item(ui, tl!("Save Symbols…"), has_symbols, false) {
            let sel = current(app);
            save_library(app, sel.into_iter().collect());
        }
    }
}

/// Run `symbol.addFromLibrary` with `p`; a failure goes to the status bar.
fn add(app: &mut VectorcraftApp, p: Value) {
    if let Err(e) = app.run("symbol.addFromLibrary", p) {
        app.status(e);
    }
}

/// Symbol `name`'s natural size in `doc`.
fn symbol_size(doc: &Document, name: &str) -> (f64, f64) {
    let size = doc.unknown.get(vectorcraft_doc::SYMBOL_SIZES).and_then(|m| m.get(name));
    size.and_then(|v| Some((v.get(0)?.as_f64()?, v.get(1)?.as_f64()?)))
        .unwrap_or((2.0 * vectorcraft_doc::SYMBOL_HALF, 2.0 * vectorcraft_doc::SYMBOL_HALF))
}

/// `window.symbolLibrary {library}`: open a symbol library in its own panel (`library` null closes
/// every symbol library panel).
pub(crate) fn open_library(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    library_panel::open_command::<SymbolLibraries>(app, p)
}

/// `symbol.loadLibrary` params, then open the library in its panel.
pub(crate) fn load_library(app: &mut VectorcraftApp, params: Value) -> Result<Value, String> {
    let r = app.run("symbol.loadLibrary", params)?;
    open_library(app, &json!({ "library": r["library"] }))
}

/// Other Library…: load the symbols of the document at `path`, else of one picked in an open
/// dialog.
pub(crate) fn other_library(app: &mut VectorcraftApp, path: Option<String>) -> Result<Value, String> {
    match library_panel::pick_library_file(app, path, library_panel::library_pick_filters(SymbolLibraries::KIND))? {
        Some(path) => load_library(app, json!({ "path": path })),
        None => Ok(Value::Null),
    }
}

/// The id prefix of the Window → Symbol Libraries → User Defined slots.
pub(crate) const USER_SLOT: &str = "window.userSymbolLibrary";

/// The User Defined library slot `id` (`window.userSymbolLibrary3`) stands for.
pub(crate) fn user_library(app: &VectorcraftApp, id: &str) -> Option<LibraryInfo> {
    library_panel::user_slot(id, USER_SLOT, symbollib::libraries(&app.session))
}

/// Window → Symbol Libraries.
pub(crate) fn window_menu() -> Vec<Item> {
    const SLOTS: [&str; 10] = [
        "window.userSymbolLibrary1",
        "window.userSymbolLibrary2",
        "window.userSymbolLibrary3",
        "window.userSymbolLibrary4",
        "window.userSymbolLibrary5",
        "window.userSymbolLibrary6",
        "window.userSymbolLibrary7",
        "window.userSymbolLibrary8",
        "window.userSymbolLibrary9",
        "window.userSymbolLibrary10",
    ];
    let open = |b: &'static symbollib::BuiltinSymbolLibrary| Item::Cmd(b.name, SymbolLibraries::OPEN, json!({ "library": b.id }));
    let mut items: Vec<Item> = symbollib::BUILTIN.iter().map(open).collect();
    items.extend([
        Item::Sep,
        Item::Sub("User Defined", SLOTS.iter().map(|id| Item::Cmd("User Library", id, Value::Null)).collect()),
        Item::Sep,
        Item::Cmd("Other Library…", "window.symbolLibrary.other", Value::Null),
        Item::Cmd("Save Symbols…", "ui.saveSymbolLibrary", Value::Null),
    ]);
    items
}

#[cfg(test)]
mod tests {
    use egui::{Pos2, Rect, pos2};
    use vectorcraft_doc::NodeId;
    use vectorcraft_engine::Session;

    use super::*;

    /// One headless frame of the panel, 236 pt wide as in the dock.
    fn frame(ctx: &egui::Context, app: &mut VectorcraftApp, events: Vec<egui::Event>) {
        let raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(236.0, 400.0))), events, ..Default::default() };
        let mut out = ctx.run_ui(raw, |ui| show(app, ui));
        out.textures_delta.clear();
    }

    fn button(at: Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() }
    }

    fn symbols(app: &mut VectorcraftApp) -> Vec<String> {
        let list = app.run("symbol.list", json!({})).unwrap();
        list["symbols"].as_array().unwrap().iter().map(|s| s["name"].as_str().unwrap_or_default().to_string()).collect()
    }

    /// Pixels of `img` that something paints opaquely, and those of them that are red.
    fn painted(img: &vectorcraft_render::Rendered) -> (usize, usize) {
        let painted: Vec<&[u8; 4]> = img.pixels.as_chunks::<4>().0.iter().filter(|p| p[3] > 200).collect();
        let red = painted.iter().filter(|p| p[0] > 150 && p[1] < 80 && p[2] < 80).count();
        (painted.len(), red)
    }

    /// A symbol that nests another (a library's included) shows the nested art in its
    /// thumbnail: the preview keeps the document's symbols and patterns, not its art.
    #[test]
    fn thumbnails_show_the_symbols_a_symbol_nests() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
        s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 40, "height": 40})).unwrap();
        s.execute("paint.setFill", &json!({"color": "#ff0000"})).unwrap();
        s.execute("paint.setStroke", &json!({"none": true})).unwrap();
        // The instance "Dot" leaves behind becomes all of "Badge".
        s.execute("symbol.new", &json!({"name": "Dot"})).unwrap();
        s.execute("symbol.new", &json!({"name": "Badge"})).unwrap();
        // Other art of the document stays out of the preview.
        s.execute("shape.ellipse", &json!({"x": 0, "y": 0, "width": 200, "height": 200})).unwrap();
        s.execute("paint.setFill", &json!({"color": "#0000ff"})).unwrap();
        let lib = vectorcraft_engine::cmd::symbollib::SymbolLibrary::from_document(&s.doc().unwrap().doc, &["Badge".into()], "Kit".into()).unwrap();
        let mut r = vectorcraft_render::Renderer::new();
        for doc in [&*s.doc().unwrap().doc, &lib.doc] {
            let img = render_thumb(&mut r, doc, "Badge", 32).expect("a thumbnail");
            let (painted, red) = painted(&img);
            assert!(painted > 0 && red * 10 >= painted * 9, "the nested Dot paints it: {painted} painted, {red} red");
        }
    }

    #[test]
    fn art_dropped_on_the_panel_becomes_a_symbol_and_a_symbol_drags_out() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
        let id = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 30, "height": 20})).unwrap()["id"].as_u64().unwrap();
        // Selected, as art dragged off the canvas is.
        let ctx = egui::Context::default();
        frame(&ctx, &mut app, vec![]);
        // Art dragged off the canvas and released over the list.
        let over = pos2(100.0, 50.0);
        egui::DragAndDrop::set_payload(&ctx, PanelDrag::Art(vec![NodeId(id)]));
        frame(&ctx, &mut app, vec![egui::Event::PointerMoved(over), button(over, false)]);
        let names = symbols(&mut app);
        assert_eq!(names.len(), 1, "{names:?}");
        let doc = &app.session.active().unwrap().doc;
        let first = doc.layers[0].children().unwrap().first().unwrap();
        assert!(matches!(first.kind, NodeKind::SymbolInstance { .. }), "the art became an instance");
        // Dragging the symbol's tile out of the panel carries the symbol.
        frame(&ctx, &mut app, vec![]);
        let tile = pos2(26.0, 26.0);
        frame(&ctx, &mut app, vec![egui::Event::PointerMoved(tile), button(tile, true)]);
        frame(&ctx, &mut app, vec![egui::Event::PointerMoved(tile + vec2(40.0, 40.0))]);
        frame(&ctx, &mut app, vec![egui::Event::PointerMoved(tile + vec2(80.0, 80.0))]);
        assert_eq!(egui::DragAndDrop::payload::<PanelDrag>(&ctx).as_deref(), Some(&PanelDrag::Symbol(names[0].clone())));
    }

    /// One headless frame of the panel menu; returns its painted labels.
    fn menu_frame(ctx: &egui::Context, app: &mut VectorcraftApp, events: Vec<egui::Event>) -> Vec<(String, Rect)> {
        let raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(236.0, 400.0))), events, ..Default::default() };
        let mut out = ctx.run_ui(raw, |ui| menu(app, ui));
        out.textures_delta.clear();
        crate::tests_removeanchors::shapes_text(&out.shapes.iter().map(|c| c.shape.clone()).collect::<Vec<_>>())
    }

    #[test]
    fn new_symbol_asks_for_symbol_options_before_making_the_symbol() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
        app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 30, "height": 20})).unwrap();
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let labels = menu_frame(&ctx, &mut app, vec![]);
        let item = labels.iter().find(|(t, _)| t.trim() == "New Symbol…").map(|(_, r)| r.center()).expect("New Symbol… in the menu");
        menu_frame(&ctx, &mut app, vec![egui::Event::PointerMoved(item), button(item, true)]);
        menu_frame(&ctx, &mut app, vec![button(item, false)]);
        assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some("symbolOptions"));
        assert!(symbols(&mut app).is_empty(), "no symbol before OK");
        assert_eq!(app.ui.dialog.as_ref().unwrap().str("name"), "New Symbol");
        assert_eq!(app.ui.dialog.as_ref().unwrap().str("exportType"), "movieClip");
        crate::dialogs::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
        let list = app.run("symbol.list", json!({})).unwrap();
        assert_eq!(list["symbols"][0]["exportType"], "movieClip");
        // The next one is offered a name no symbol has, and OK keeps the name typed and Export Type.
        app.run("shape.rectangle", json!({"x": 60, "y": 10, "width": 30, "height": 20})).unwrap();
        app.run("ui.symbolOptions", json!({})).unwrap();
        assert_eq!(app.ui.dialog.as_ref().unwrap().str("name"), "New Symbol 2");
        app.ui.dialog.as_mut().unwrap().fields.insert("name".into(), json!("Badge"));
        app.ui.dialog.as_mut().unwrap().fields.insert("exportType".into(), json!("graphic"));
        crate::dialogs::confirm(&mut app).unwrap();
        assert_eq!(symbols(&mut app), ["New Symbol", "Badge"]);
        let list = app.run("symbol.list", json!({})).unwrap();
        assert_eq!(list["symbols"][1]["exportType"], "graphic");
        // Symbol Options on the current symbol renames and keeps Export Type.
        app.run("ui.symbolOptions", json!({"edit": true, "name": "Badge"})).unwrap();
        assert_eq!(app.ui.dialog.as_ref().unwrap().str("exportType"), "graphic");
        app.ui.dialog.as_mut().unwrap().fields.insert("name".into(), json!("Mark"));
        crate::dialogs::confirm(&mut app).unwrap();
        assert_eq!(symbols(&mut app), ["New Symbol", "Mark"]);
        assert_eq!(app.run("symbol.list", json!({})).unwrap()["symbols"][1]["exportType"], "graphic");
        // Enable Guides for 9-Slice Scaling stores the guides (percent in the dialog) and reopens as set.
        assert!(app.run("symbol.list", json!({})).unwrap()["symbols"][1]["nineSlice"].is_null());
        app.run("ui.symbolOptions", json!({"edit": true, "name": "Mark"})).unwrap();
        let dialog = app.ui.dialog.as_mut().unwrap();
        dialog.fields.insert("nineSlice".into(), json!(true));
        dialog.fields.insert("sliceLeft".into(), json!(20.0));
        crate::dialogs::confirm(&mut app).unwrap();
        let nine = app.run("symbol.list", json!({})).unwrap()["symbols"][1]["nineSlice"].clone();
        assert_eq!(nine[0], json!(0.2));
        app.run("ui.symbolOptions", json!({"edit": true, "name": "Mark"})).unwrap();
        assert!(app.ui.dialog.as_ref().unwrap().bool("nineSlice"));
        assert_eq!(app.ui.dialog.as_ref().unwrap().f64("sliceLeft", 0.0), 20.0);
    }

    /// F8 is Symbol Options… (`ui.symbolOptions`) by default, on every platform. Shift+F8 and Cmd+F8
    /// are still the Transform and Info panels' keys, and Cmd+Shift+F8 is still Edit Pattern's. No
    /// other command, tool or panel has F8 by default.
    #[test]
    fn f8_is_symbol_options() {
        use crate::shortcut_editor::{PANEL_SHORTCUTS, command_for_key, default_command_shortcut};
        assert_eq!(default_command_shortcut("ui.symbolOptions"), Some("F8"));
        assert_eq!(command_for_key("F8"), Some("ui.symbolOptions"));
        assert_eq!(command_for_key("Cmd+Shift+F8"), Some("object.pattern.edit"));
        let panel = |key: &str| PANEL_SHORTCUTS.iter().find(|(_, sc)| *sc == key).map(|(p, _)| *p);
        assert_eq!((panel("Shift+F8"), panel("Cmd+F8"), panel("F8")), (Some("transform"), Some("info"), None));
        let chord = crate::shortcuts::parse("F8").expect("parses");
        assert_eq!((chord.modifiers, chord.logical_key), (egui::Modifiers::NONE, egui::Key::F8));
        assert_eq!(crate::menus::pretty_shortcut("F8"), "F8");
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        let listed = app.run("shortcuts.list", json!({"query": "ui.symbolOptions"})).unwrap();
        let rows: Vec<_> = listed.as_array().expect("rows").iter().map(|r| (r["label"].clone(), r["default"].clone())).collect();
        assert_eq!(rows, [(json!("Symbol Options…"), json!("F8"))], "Keyboard Shortcuts lists it");
        assert_eq!(app.run("shortcuts.conflicts", json!({})).unwrap(), json!([]), "no other default has the key");
    }

    /// F8, pressed through the shortcut handler, opens Symbol Options for a new symbol from the
    /// selection, as the panel menu's New Symbol… does, and no symbol is made until OK. OK makes the
    /// symbol and puts an instance of it in the art's place, and Cancel makes none. With nothing
    /// selected, neither F8 nor New Symbol… opens a dialog or makes a symbol, and after F8 the
    /// status bar says that nothing is selected.
    #[test]
    fn f8_opens_symbol_options_for_a_new_symbol_as_new_symbol_does() {
        fn press(app: &mut VectorcraftApp) {
            let ctx = egui::Context::default();
            let ev = egui::Event::Key { key: egui::Key::F8, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() };
            let mut out = ctx.run_ui(egui::RawInput { events: vec![ev], ..Default::default() }, |ui| crate::shortcuts::handle(app, ui.ctx()));
            out.textures_delta.clear();
        }
        /// A document with one selected rectangle.
        fn one_rect() -> VectorcraftApp {
            let mut app = VectorcraftApp::new(Session::new(), Default::default());
            app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
            app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 30, "height": 20})).unwrap();
            app
        }
        /// Click the panel menu's New Symbol… item.
        fn click_new_symbol(app: &mut VectorcraftApp) {
            let ctx = egui::Context::default();
            crate::theme::install_fonts(&ctx);
            let labels = menu_frame(&ctx, app, vec![]);
            let item = labels.iter().find(|(t, _)| t.trim() == "New Symbol…").map(|(_, r)| r.center()).expect("New Symbol… in the menu");
            menu_frame(&ctx, app, vec![egui::Event::PointerMoved(item), button(item, true)]);
            menu_frame(&ctx, app, vec![button(item, false)]);
        }
        let dialog = |app: &VectorcraftApp| app.ui.dialog.as_ref().map(|d| (d.kind.clone(), d.fields.clone()));
        let mut keyed = one_rect();
        press(&mut keyed);
        let mut clicked = one_rect();
        click_new_symbol(&mut clicked);
        let opened = dialog(&keyed).expect("the key opens a dialog");
        assert_eq!(
            (opened.0.as_str(), opened.1.get("__edit"), opened.1.get("name")),
            ("symbolOptions", Some(&json!(false)), Some(&json!("New Symbol")))
        );
        assert_eq!(dialog(&clicked), Some(opened), "the key opens what New Symbol… opens");
        assert!(symbols(&mut keyed).is_empty(), "no symbol until OK");
        for app in [&mut keyed, &mut clicked] {
            crate::dialogs::confirm(app).unwrap();
        }
        assert_eq!(symbols(&mut keyed), ["New Symbol"]);
        assert_eq!(symbols(&mut clicked), ["New Symbol"]);
        let doc = &keyed.session.active().unwrap().doc;
        let art = doc.layers[0].children().unwrap();
        assert_eq!(art.len(), 1);
        assert!(matches!(&art[0].kind, NodeKind::SymbolInstance { symbol, .. } if symbol == "New Symbol"), "the rectangle became an instance");
        // Cancel makes no symbol.
        keyed.run("shape.rectangle", json!({"x": 60, "y": 10, "width": 30, "height": 20})).unwrap();
        press(&mut keyed);
        assert!(keyed.ui.dialog.is_some());
        crate::dialogs::cancel(&mut keyed);
        assert!(keyed.ui.dialog.is_none());
        assert_eq!(symbols(&mut keyed), ["New Symbol"]);
        // With nothing selected, neither the key nor New Symbol… opens a dialog.
        for app in [&mut keyed, &mut clicked] {
            app.run("select.none", json!({})).unwrap();
        }
        press(&mut keyed);
        click_new_symbol(&mut clicked);
        for app in [&mut keyed, &mut clicked] {
            assert!(app.ui.dialog.is_none());
            assert_eq!(symbols(app), ["New Symbol"]);
        }
        assert_eq!(keyed.ui.status, "nothing selected");
    }

    #[test]
    fn an_imported_symbols_thumbnail_frames_its_whole_art() {
        // The symbol of the SVG import tests: an orange 20 × 20 square from (0, 0) with a white dot.
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="120"><defs><symbol id="Star" data-name="Gold Star"><path d="M0 0H20V20H0Z" fill="#e0a000"/><circle cx="10" cy="10" r="4" fill="#fff"/></symbol></defs><use href="#Star" x="20" y="30"/></svg>"##;
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("document.open", json!({"name": "star.svg", "dataBase64": vectorcraft_format::base64_encode(svg.as_bytes())})).unwrap();
        let ctx = egui::Context::default();
        let raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(236.0, 400.0))), ..Default::default() };
        let mut out = ctx.run_ui(raw, |ui| show(&mut app, ui));
        // The tile's thumbnail: 40 × 40 pixels.
        let thumb = out.textures_delta.set.values().flatten().find_map(|d| match &d.image {
            egui::ImageData::Color(img) if img.size == [40, 40] => Some(img.clone()),
            _ => None,
        });
        out.textures_delta.clear();
        let thumb = thumb.expect("the thumbnail");
        // The art fills it, a pixel from each edge: its top-left corner is orange too, not only the
        // quarter the 20 × 20 square around the origin would frame.
        let orange = |x: usize, y: usize| {
            let [r, g, b, a] = thumb.pixels[y * 40 + x].to_array();
            a == 255 && r > 200 && (130..190).contains(&g) && b < 40
        };
        assert!(orange(3, 3) && orange(36, 36), "{:?} {:?}", thumb.pixels[3 * 40 + 3], thumb.pixels[36 * 40 + 36]);
    }
}
