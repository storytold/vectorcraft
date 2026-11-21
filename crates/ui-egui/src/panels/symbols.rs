//! Symbols panel: the document's symbols as rendered thumbnails or in a small or large list. Click
//! selects a symbol (it becomes the Symbol Sprayer's current symbol); double-click opens it in symbol editing mode. Symbol
//! libraries open in library panels ([`SymbolLibraries`]).

use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{Arc, Weak};

use egui::{Sense, Stroke, StrokeKind, Ui, vec2};
use serde_json::{Value, json};
use vectorcraft_doc::{Document, Node, NodeKind};
use vectorcraft_engine::DocState;
use vectorcraft_engine::cmd::clipboard::{ArtResources, art_resources};
use vectorcraft_engine::cmd::swatchlib::LibraryInfo;
use vectorcraft_engine::cmd::symbollib::{self, SymbolLibrary};

use super::library_panel::{self, LibraryKind, LibraryRef, Row};
use super::memo::Memo;
use super::{pstate, set_pstate};
use crate::VectorcraftApp;
use crate::caches::SymbolInstances;
use crate::menus::Item;
use crate::theme::Tokens;
use crate::widgets::{self, PanelDrag, menu_item};

/// The largest side of a symbol thumbnail, in pixels. A larger chip draws its thumbnail scaled up.
const MAX_THUMB_PX: f32 = 1024.0;

/// The pixels on a side of the thumbnail for a chip `size` points wide at `pixels_per_point`: 8
/// to [`MAX_THUMB_PX`].
fn thumb_px(size: f32, pixels_per_point: f32) -> u32 {
    let px = (size * pixels_per_point).round();
    if px.is_nan() { 8 } else { px.clamp(8.0, MAX_THUMB_PX) as u32 }
}

/// A thumbnail of symbol `name` (an instance at its natural size), cached by its key in `keys`
/// ([`Thumbs`]).
fn thumb(ui: &Ui, doc: &Document, keys: &Thumbs, name: &str, size: f32) -> Option<egui::TextureHandle> {
    thread_local! {
        static RENDERER: RefCell<vectorcraft_render::Renderer> = RefCell::new(vectorcraft_render::Renderer::new());
        static CACHE: crate::graphics::TexCache<HashMap<(u64, u32), egui::TextureHandle>> = crate::graphics::TexCache::default();
    }
    let px = thumb_px(size, ui.ctx().pixels_per_point());
    let key = (*keys.keys.get(name)?, px);
    if let Some(t) = CACHE.with(|c| c.borrow().get(&key).cloned()) {
        return Some(t);
    }
    #[cfg(test)]
    counters::RENDERED.with(|c| c.set(c.get() + 1));
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

/// The thumbnail keys of a document version's or a library's symbols, worked out once for it
/// ([`document_symbols`], [`library_symbols`]): a symbol's key is its name, its art and every
/// resource the art uses ([`art_resources`]: the symbols it nests, patterns with their options,
/// swatches and colour groups, gradients, images, styles…), so a thumbnail renders again when one
/// of those changes and stays cached when anything else does.
pub(crate) struct Thumbs {
    keys: HashMap<String, u64>,
    /// Keeps the addresses the keys count from being reused.
    _kept: Vec<(Weak<Node>, ArtResources)>,
}

impl Thumbs {
    fn of(doc: &Document) -> Self {
        use std::hash::{Hash, Hasher};
        let mut keys = HashMap::new();
        let mut kept = vec![];
        for s in &doc.symbols {
            #[cfg(test)]
            counters::KEYED.with(|c| c.set(c.get() + 1));
            let deps = art_resources(doc, [&*s.art]);
            let mut h = std::collections::hash_map::DefaultHasher::new();
            (&s.name, Arc::as_ptr(&s.art) as usize, deps.key).hash(&mut h);
            keys.entry(s.name.clone()).or_insert(h.finish());
            kept.push((Arc::downgrade(&s.art), deps));
        }
        Self { keys, _kept: kept }
    }

    /// The thumbnail key of symbol `name` (none when there is no such symbol).
    #[cfg(test)]
    pub(crate) fn key(&self, name: &str) -> Option<u64> {
        self.keys.get(name).copied()
    }
}

/// [`Thumbs`] of the document version `st` shows.
pub(crate) fn document_symbols(st: &DocState) -> Rc<Thumbs> {
    thread_local! {
        static MEMO: RefCell<Memo<Document, Thumbs>> = RefCell::new(Memo::default());
    }
    MEMO.with(|m| m.borrow_mut().get(&st.doc, st.revision, || Thumbs::of(&st.doc)))
}

/// [`Thumbs`] of symbol library `lib`.
pub(crate) fn library_symbols(lib: &Arc<SymbolLibrary>) -> Rc<Thumbs> {
    thread_local! {
        static MEMO: RefCell<Memo<SymbolLibrary, Thumbs>> = RefCell::new(Memo::default());
    }
    MEMO.with(|m| m.borrow_mut().get(lib, 0, || Thumbs::of(&lib.doc)))
}

/// Symbol thumbnails' keys worked out and thumbnails rendered on this test thread.
#[cfg(test)]
pub(crate) mod counters {
    thread_local! {
        pub(crate) static KEYED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
        pub(crate) static RENDERED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    }
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

/// Symbol `name`'s thumbnail (keyed by `keys`, [`document_symbols`] or [`library_symbols`] of
/// `doc`) on white in `r`: the panel's tiles and the chip a dragged symbol shows at the pointer.
pub(crate) fn chip(ui: &Ui, doc: &Document, keys: &Thumbs, r: egui::Rect, name: &str) {
    ui.painter().rect_filled(r, 0.0, egui::Color32::WHITE);
    if let Some(tex) = thumb(ui, doc, keys, name, r.width()) {
        ui.painter().image(tex.id(), r, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
    }
}

/// The cache key of an [`art_chip`]: the symbol's key, the pixels across and down, and the bits of
/// the art's bounds.
type ArtKey = (u64, u32, u32, [u64; 4]);

/// Symbol `name`'s art on white in `r`, its bounds `art` (at its own size, as a mapped graphic
/// measures them) stretched to fill `r`, as a box the art is scaled into shows it. The art is
/// rendered at most [`MAX_THUMB_PX`] pixels on a side, cached by its key in `keys`.
pub(crate) fn art_chip(ui: &Ui, doc: &Document, keys: &Thumbs, r: egui::Rect, name: &str, art: vectorcraft_geom::Rect) {
    thread_local! {
        static RENDERER: RefCell<vectorcraft_render::Renderer> = RefCell::new(vectorcraft_render::Renderer::new());
        static CACHE: crate::graphics::TexCache<HashMap<ArtKey, egui::TextureHandle>> = crate::graphics::TexCache::default();
    }
    ui.painter().rect_filled(r, 0.0, egui::Color32::WHITE);
    let ppp = ui.ctx().pixels_per_point();
    let (w, h) = (thumb_px(r.width(), ppp), thumb_px(r.height(), ppp));
    let Some(&symbol) = keys.keys.get(name) else { return };
    let key = (symbol, w, h, [art.x0, art.y0, art.x1, art.y1].map(f64::to_bits));
    let tex = match CACHE.with(|c| c.borrow().get(&key).cloned()) {
        Some(t) => t,
        None => {
            let Some(img) = RENDERER.with(|rr| render_art(&mut rr.borrow_mut(), doc, name, art, (w, h))) else { return };
            let tex = ui.ctx().load_texture(format!("sym-art-{name}-{w}x{h}"), crate::canvas::color_image(&img), egui::TextureOptions::LINEAR);
            CACHE.with(|c| {
                let mut c = c.borrow_mut();
                if c.len() > 64 {
                    c.clear();
                }
                c.insert(key, tex.clone());
            });
            tex
        }
    };
    ui.painter().image(tex.id(), r, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
}

/// Symbol `name` of `doc` (an instance at its natural size, as [`render_thumb`] places it) drawn
/// into `w` × `h` pixels with its bounds `art` stretched to fill them. `None` for bounds with no
/// area.
fn render_art(
    r: &mut vectorcraft_render::Renderer,
    doc: &Document,
    name: &str,
    art: vectorcraft_geom::Rect,
    (w, h): (u32, u32),
) -> Option<vectorcraft_render::Rendered> {
    let (aw, ah) = (art.width(), art.height());
    if !(aw.is_finite() && ah.is_finite() && aw > 0.0 && ah > 0.0) {
        return None;
    }
    let (sw, sh) = symbol_size(doc, name);
    let mut d = doc.clone();
    let layer = d.alloc_id();
    d.layers = vec![Arc::new(Node::layer(layer, "Preview", vectorcraft_doc::LayerColor::Preset(0)))];
    let id = d.alloc_id();
    let xf = vectorcraft_geom::Affine::scale_non_uniform(sw / 20.0, sh / 20.0);
    let n = Arc::new(Node::new(id, NodeKind::SymbolInstance { symbol: name.into(), xf }));
    let view =
        vectorcraft_geom::Affine::scale_non_uniform(f64::from(w) / aw, f64::from(h) / ah) * vectorcraft_geom::Affine::translate((-art.x0, -art.y0));
    r.render_node(&d, &n, u16::try_from(w).ok()?, u16::try_from(h).ok()?, view)
}

/// The active document's current symbol (`symbol.list`'s `current`), read without running a
/// command; `None` without a document or symbols.
fn current(app: &VectorcraftApp) -> Option<String> {
    app.session.active().and_then(|st| vectorcraft_engine::cmd::current_symbol(&st.doc))
}

#[cfg(test)]
thread_local! {
    /// How many times [`selected_instances`] walked the selection on this thread.
    static INSTANCE_SCANS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// The symbol instances in the selection and in its groups, found in one walk of the selected
/// subtrees per (document uid, revision) ([`crate::caches::CanvasCache::symbol_instances`]);
/// `None` without a document.
fn selected_instances(app: &mut VectorcraftApp) -> Option<&mut SymbolInstances> {
    let st = app.session.active()?;
    let key = (st.uid, st.revision);
    if app.canvas.symbol_instances.as_ref().is_none_or(|(k, _)| *k != key) {
        #[cfg(test)]
        INSTANCE_SCANS.with(|n| n.set(n.get() + 1));
        let mut found = SymbolInstances::default();
        for n in st.selection.objects.iter().filter_map(|id| st.doc.node(*id)) {
            n.walk(&mut |c| {
                if matches!(c.kind, NodeKind::SymbolInstance { .. }) {
                    found.changed |= !c.extras().symbol_overrides.is_empty();
                    found.ids.push(c.id);
                }
            });
        }
        app.canvas.symbol_instances = Some((key, found));
    }
    app.canvas.symbol_instances.as_mut().map(|(_, found)| found)
}

fn has_instances_selected(app: &mut VectorcraftApp) -> bool {
    selected_instances(app).is_some_and(|found| !found.ids.is_empty())
}

/// Collect the IDs of selected instances of symbol `name` (including inside groups).
fn selected_symbol_instances(app: &VectorcraftApp, name: &str) -> Vec<u64> {
    let mut ids = vec![];
    if let Some(st) = app.session.active() {
        for id in &st.selection.objects {
            if let Some(n) = st.doc.node(*id) {
                n.walk(&mut |c| {
                    if matches!(&c.kind, NodeKind::SymbolInstance { symbol, .. } if symbol == name) {
                        ids.push(c.id.0);
                    }
                });
            }
        }
    }
    ids
}

/// A symbol item's context menu.
fn item_context_menu(app: &mut VectorcraftApp, ui: &mut Ui, name: &str) {
    let center = app
        .view()
        .map(|v| v.center)
        .or_else(|| app.session.active().and_then(|st| st.doc.artboards.first().map(|a| a.rect.center())))
        .unwrap_or(vectorcraft_geom::Point::ZERO);

    if menu_item(ui, tl!("Place Symbol Instance"), true, false) {
        if let Err(e) = app.run("symbol.place", json!({ "name": name, "x": center.x, "y": center.y })) {
            app.status(e);
        }
        ui.close();
    }
    if menu_item(ui, tl!("Edit Symbol"), true, false) {
        if let Err(e) = app.run("symbol.edit", json!({ "name": name })) {
            app.status(e);
        }
        ui.close();
    }
    if menu_item(ui, tl!("Duplicate Symbol"), true, false) {
        if let Err(e) = app.run("symbol.duplicate", json!({ "name": name })) {
            app.status(e);
        }
        ui.close();
    }
    if menu_item(ui, tl!("Delete Symbol"), true, false) {
        delete(app, &[], Some(name), false);
        ui.close();
    }
    let break_ids = selected_symbol_instances(app, name);
    let break_enabled = !break_ids.is_empty();
    if menu_item(ui, tl!("Break Link to Symbol"), break_enabled, false) {
        if let Err(e) = app.run("symbol.breakLink", json!({ "ids": break_ids })) {
            app.status(e);
        }
        ui.close();
    }
    if menu_item(ui, tl!("Select All Instances"), true, false) {
        if let Err(e) = app.run("symbol.selectInstances", json!({ "name": name })) {
            app.status(e);
        }
        ui.close();
    }
}

/// Does the selection hold a symbol instance with appearance changes of its own (a dynamic
/// symbol's), which Reset drops?
pub(crate) fn changed_instances_selected(app: &mut VectorcraftApp) -> bool {
    selected_instances(app).is_some_and(|found| found.changed)
}

/// Does the selection hold a symbol instance whose scale, rotation or shear differs from its
/// symbol's, which Reset Transformation restores?
fn transformed_instances_selected(app: &mut VectorcraftApp) -> bool {
    let Some(doc) = app.session.active().map(|st| st.doc.clone()) else { return false };
    let Some(found) = selected_instances(app) else { return false };
    let ids = &found.ids;
    *found
        .transformed
        .get_or_insert_with(|| ids.iter().filter_map(|id| doc.node(*id)).any(|n| vectorcraft_engine::cmd::reset_transformation_changes(&doc, n)))
}

/// The symbols of the selected objects when every one is a symbol instance; `None` otherwise.
fn selected_instance_symbols(app: &VectorcraftApp) -> Option<Vec<String>> {
    let st = app.session.active()?;
    let symbols: Option<Vec<String>> = st
        .selection
        .objects
        .iter()
        .map(|id| match st.doc.node(*id).map(|n| &n.kind) {
            Some(NodeKind::SymbolInstance { symbol, .. }) => Some(symbol.clone()),
            _ => None,
        })
        .collect();
    symbols.filter(|s| !s.is_empty())
}

/// The Control bar's Replace menu while symbol instances alone are selected: the document's
/// symbols, showing the one the instances share (blank when they differ); picking another
/// replaces them all, as "Select a new symbol from the Replace dropdown in the Control panel"
/// (<https://helpx.adobe.com/illustrator/desktop/manage-objects/traces-mockups-symbols/edit-symbols.html>),
/// and the Reset button, which gives them their symbol's scale, rotation and shear again
/// (<https://helpx.adobe.com/illustrator/desktop/manage-objects/traces-mockups-symbols/transform-symbols.html>).
/// Edit Symbol, Replace with its menu and Reset each show only while the row has room for them and
/// `reserve` after them, and the controls after one left out are left out too.
pub(crate) fn control_bar(app: &mut VectorcraftApp, ui: &mut Ui, reserve: f32) {
    let Some(symbols) = selected_instance_symbols(app) else { return };
    let Some(st) = app.session.active() else { return };
    let names: Vec<String> = st.doc.symbols.iter().map(|s| s.name.clone()).collect();
    let shared = symbols.first().filter(|first| symbols.iter().all(|s| s == *first)).cloned().unwrap_or_default();
    let mut fit = crate::chrome::Fit::new(reserve);
    // Edit Symbol: the instances' symbol in symbol editing mode, when they share one.
    if !shared.is_empty()
        && fit.room(ui, &[24.0])
        && widgets::icon_button(ui, "dc-place-symbol", tl!("Edit Symbol"), false, 24.0).clicked()
        && let Err(e) = app.run("symbol.edit", json!({}))
    {
        app.status(e);
    }
    let replace = egui::RichText::new(tl!("Replace:")).size(12.0).color(Tokens::get(ui.ctx()).text);
    if fit.room(ui, &[widgets::text_width(ui, replace.clone()), 140.0]) {
        ui.label(replace);
        let options: Vec<&str> = names.iter().map(String::as_str).collect();
        if let Some(name) = widgets::dropdown_names(ui, "cb-symbol-replace", &shared, &options, 140.0).and_then(|i| names.get(i))
            && *name != shared
            && let Err(e) = app.run("symbol.replace", json!({ "name": name }))
        {
            app.status(e);
        }
    }
    // Reset: Reset Transformation, enabled while an instance's scale, rotation or shear differs
    // from its symbol's.
    let reset = transformed_instances_selected(app);
    if fit.room(ui, &[widgets::flat_button_fit_width(ui, tl!("Reset"), 48.0)])
        && ui.add_enabled_ui(reset, |ui| widgets::flat_button_fit(ui, tl!("Reset"), 48.0).on_hover_text(tl!("Reset Transformation"))).inner.clicked()
        && let Err(e) = app.run("symbol.instance.resetTransform", json!({}))
    {
        app.status(e);
    }
    if ui.available_width() >= reserve + 4.0 {
        ui.add_space(4.0);
    }
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let Some((doc, keys, document)) = app.session.active().map(|d| (d.doc.clone(), document_symbols(d), d.uid)) else { return };
    let names: Vec<String> = doc.symbols.iter().map(|s| s.name.clone()).collect();
    let view = app.ui.symbols_view;
    let sel = current(app);
    let uid = app.session.active().map(|d| d.uid);
    let marked = marked(ui.ctx(), uid, &names);
    // The symbols drawn selected: the ones Select All Unused picked, else the current one.
    let on = |n: &str| if marked.is_empty() { sel.as_deref() == Some(n) } else { marked.iter().any(|m| m == n) };
    let mut clicked: Option<(String, bool)> = None;
    // Art released on a symbol with Alt (Option) held redefines that symbol; while Alt art is over
    // a symbol, the list does not take it as a new symbol.
    let mut redefine: Option<(String, Vec<u64>)> = None;
    let mut over_symbol = false;
    let list_rect = widgets::list_box(ui, |ui| {
        ui.set_min_height(110.0);
        ui.set_width(ui.available_width());
        if names.is_empty() {
            super::empty_state(ui, "spray-can", tl!("No symbols in this document"), tl!("Select art and click New Symbol to make one."));
            return ui.min_rect();
        }
        if let Some((h, side, text)) = view.row() {
            for n in &names {
                let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), h), Sense::click_and_drag());
                // Dragged onto the canvas, the symbol is placed there.
                widgets::drag_source(ui, &resp, || PanelDrag::Symbol { document, name: n.clone() });
                if on(n) {
                    ui.painter().rect_filled(r, 0.0, t.row_selected);
                } else if resp.hovered() {
                    ui.painter().rect_filled(r, 0.0, t.hover);
                }
                chip(ui, &doc, &keys, egui::Rect::from_center_size(r.left_center() + vec2(3.0 + side / 2.0, 0.0), vec2(side, side)), n);
                ui.painter().text(r.left_center() + vec2(side + 10.0, 0.0), egui::Align2::LEFT_CENTER, n, egui::FontId::proportional(text), t.text);
                if let Some(ids) = art_redefine_drop(ui, &resp, &doc, uid, &mut over_symbol) {
                    redefine = Some((n.clone(), ids));
                }
                let resp = resp.on_hover_text(n);
                if resp.double_clicked() {
                    clicked = Some((n.clone(), true));
                } else if resp.clicked() {
                    clicked = Some((n.clone(), false));
                }
                if resp.secondary_clicked() && (sel.as_deref() != Some(n) || !marked.is_empty()) {
                    set_pstate(ui.ctx(), MARKED, (uid, Vec::<String>::new()));
                    if let Err(e) = app.run("symbol.setCurrent", json!({ "name": n })) {
                        app.status(e);
                    }
                }
                resp.context_menu(|ui| item_context_menu(app, ui, n));
            }
        } else {
            ui.horizontal_wrapped(|ui| {
                for n in &names {
                    let (r, resp) = ui.allocate_exact_size(vec2(44.0, 44.0), Sense::click_and_drag());
                    widgets::drag_source(ui, &resp, || PanelDrag::Symbol { document, name: n.clone() });
                    chip(ui, &doc, &keys, r.shrink(2.0), n);
                    let on = on(n);
                    ui.painter().rect_stroke(
                        r,
                        0.0,
                        Stroke::new(if on { 2.0 } else { 1.0 }, if on { t.accent } else { t.border }),
                        StrokeKind::Inside,
                    );
                    if let Some(ids) = art_redefine_drop(ui, &resp, &doc, uid, &mut over_symbol) {
                        redefine = Some((n.clone(), ids));
                    }
                    let resp = resp.on_hover_text(n);
                    if resp.double_clicked() {
                        clicked = Some((n.clone(), true));
                    } else if resp.clicked() {
                        clicked = Some((n.clone(), false));
                    }
                    if resp.secondary_clicked() && (sel.as_deref() != Some(n) || !marked.is_empty()) {
                        set_pstate(ui.ctx(), MARKED, (uid, Vec::<String>::new()));
                        if let Err(e) = app.run("symbol.setCurrent", json!({ "name": n })) {
                            app.status(e);
                        }
                    }
                    resp.context_menu(|ui| item_context_menu(app, ui, n));
                }
            });
        }
        ui.min_rect()
    });
    let zone = ui.interact(list_rect, ui.scope_id().with("symbols-drop"), Sense::hover());
    if !over_symbol && let Some(ids) = widgets::art_drop(ui, &zone) {
        // Shift held at the release that ends the drag keeps the dropped art as ordinary art.
        let mut p = json!({ "ids": ids, "type": "dynamic" });
        if modifiers_at_release(ui).shift {
            p["keepArt"] = json!(true);
        }
        if let Err(e) = app.run("symbol.new", p) {
            app.status(e);
        }
    }
    // Redefine Symbol with the dropped art, which becomes an instance of it, in one undo step.
    if let Some((name, ids)) = redefine
        && let Err(e) = app.run("symbol.update", json!({ "name": name, "ids": ids }))
    {
        app.status(e);
    }
    // A double-click edits the symbol in symbol editing mode.
    if let Some((n, edit)) = clicked {
        // A click picks one symbol again.
        set_pstate(ui.ctx(), MARKED, (uid, Vec::<String>::new()));
        app.run("symbol.setCurrent", json!({"name": n})).ok();
        if edit && let Err(e) = app.run("symbol.edit", json!({"name": n})) {
            app.status(e);
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
        // Opens Symbol Options on the current symbol, as the panel menu's Symbol Options… does;
        // Alt does the same.
        if widgets::icon_button_enabled(ui, "dc-options", tl!("Symbol Options"), false, sel.is_some(), 24.0).clicked() {
            app.run("ui.symbolOptions", json!({"edit": true, "name": sel})).ok();
        }
        ui.add_space((ui.available_width() - 2.0 * 28.0).max(0.0));
        let new = widgets::icon_button_enabled(ui, "dc-new-item", tl!("New Symbol"), false, has_sel, 24.0);
        // A symbol dragged from the panel and dropped on New Symbol is duplicated, with or without
        // art selected.
        if let Some(name) = symbol_dropped(ui, &new, &names, uid) {
            if let Err(e) = app.run("symbol.duplicate", json!({ "name": name })) {
                app.status(e);
            }
        } else if new.clicked() {
            // Alt skips Symbol Options: the symbol is made under the default name, dynamic as the
            // dialog's default. Shift keeps the selected art as ordinary art, with or without Alt.
            // Both are read as the button went up on it.
            let m = modifiers_released_in(ui, new.interact_rect, new.clicked_by(egui::PointerButton::Primary));
            let (cmd, mut p) = if m.alt { ("symbol.new", json!({"type": "dynamic"})) } else { ("ui.symbolOptions", json!({})) };
            if m.shift {
                p["keepArt"] = json!(true);
            }
            app.run(cmd, p).ok();
        }
        // Alt deletes an unused symbol without asking; a symbol with instances still asks.
        let trash = widgets::icon_button_enabled(ui, "trash-2", tl!("Delete Symbol"), false, sel.is_some() || !marked.is_empty(), 24.0);
        // A symbol dragged from the panel and dropped on Delete Symbol is deleted as a click deletes
        // it: the dragged symbol, or every symbol Select All Unused picked when it is one of them.
        if let Some(name) = symbol_dropped(ui, &trash, &names, uid) {
            let picked = if marked.contains(&name) { &marked[..] } else { &[] };
            delete(app, picked, Some(&name), alt_at_release(ui));
        } else if trash.clicked() {
            let pointer = trash.clicked_by(egui::PointerButton::Primary);
            delete(app, &marked, sel.as_deref(), modifiers_released_in(ui, trash.interact_rect, pointer).alt);
        }
    });
}

pub fn menu(app: &mut VectorcraftApp, ui: &mut Ui) {
    let sel = current(app);
    let names: Vec<String> = app.session.active().map(|d| d.doc.symbols.iter().map(|s| s.name.clone()).collect()).unwrap_or_default();
    let uid = app.session.active().map(|d| d.uid);
    let marked = marked(ui.ctx(), uid, &names);
    let has_sel = app.session.active().is_some_and(|d| !d.selection.is_empty());
    let inst = has_instances_selected(app);
    let name = json!({"name": sel});
    // Shift, held as the button went up on New Symbol… or Redefine Symbol, keeps the selected art
    // as ordinary art instead of making it an instance.
    let items: [(&str, bool, &str, Value, bool); 3] = [
        (tl!("New Symbol…"), has_sel, "ui.symbolOptions", json!({}), true),
        (tl!("Redefine Symbol"), has_sel && sel.is_some(), "symbol.update", name.clone(), true),
        (tl!("Duplicate Symbol"), sel.is_some(), "symbol.duplicate", name.clone(), false),
    ];
    for (label, enabled, cmd, mut params, keeps_art) in items {
        let top = ui.cursor().min.y;
        let item = widgets::menu_item_response(ui, label, enabled, false);
        if item.clicked() {
            let row = egui::Rect::from_x_y_ranges(ui.max_rect().x_range(), top..=ui.cursor().min.y);
            if keeps_art && modifiers_released_in(ui, row, item.clicked_by(egui::PointerButton::Primary)).shift {
                params["keepArt"] = json!(true);
            }
            app.run(cmd, params).ok();
        }
    }
    // Asks first, as the panel's button does; the menu item has no Alt shortcut.
    if menu_item(ui, tl!("Delete Symbol"), sel.is_some() || !marked.is_empty(), false) {
        delete(app, &marked, sel.as_deref(), false);
    }
    let items: [(&str, bool, &str, Value); 8] = [
        (tl!("Symbol Options…"), sel.is_some(), "ui.symbolOptions", json!({"edit": true, "name": sel})),
        // The selected instance, else the symbol selected in the panel.
        (tl!("Edit Symbol"), inst || sel.is_some(), "symbol.edit", if inst { json!({}) } else { name.clone() }),
        (tl!("Place Symbol Instance"), sel.is_some(), "symbol.place", name.clone()),
        (tl!("Replace Symbol"), inst && sel.is_some(), "symbol.replace", name.clone()),
        (tl!("Break Link to Symbol"), inst, "symbol.breakLink", json!({})),
        (tl!("Reset Symbol Instance"), changed_instances_selected(app), "symbol.instance.reset", json!({})),
        (tl!("Reset Transformation"), transformed_instances_selected(app), "symbol.instance.resetTransform", json!({})),
        (tl!("Select All Instances"), sel.is_some(), "symbol.selectInstances", name),
    ];
    for (label, enabled, cmd, params) in items {
        if menu_item(ui, label, enabled, false) {
            app.run(cmd, params).ok();
        }
    }
    ui.separator();
    // Select All Unused (#4918): the symbols nothing places show selected, and Delete Symbol
    // deletes them all.
    if menu_item(ui, tl!("Select All Unused"), !names.is_empty(), false)
        && let Ok(r) = app.run("symbol.unused", json!({}))
    {
        let unused: Vec<String> = serde_json::from_value(r["names"].clone()).unwrap_or_default();
        if unused.is_empty() {
            app.status(tl!("No unused symbols"));
        }
        set_pstate(ui.ctx(), MARKED, (uid, unused));
    }
    // The document's symbols in name order, as one undo step.
    if menu_item(ui, tl!("Sort by Name"), names.len() > 1, false)
        && let Err(e) = app.run("symbol.sortByName", json!({}))
    {
        app.status(e);
    }
    ui.separator();
    let view = app.ui.symbols_view;
    for (v, label) in View::ALL {
        if menu_item(ui, tl!(label), true, view == v)
            && let Err(e) = app.run("ui.panelView", json!({ "panel": "symbols", "view": v }))
        {
            app.status(e);
        }
    }
    ui.separator();
    ui.menu_button(tl!("Open Symbol Library"), |ui| library_panel::library_menu::<SymbolLibraries>(app, ui));
    let has_symbols = app.session.active().is_some_and(|d| !d.doc.symbols.is_empty());
    if menu_item(ui, tl!("Save Symbol Library…"), has_symbols, false) {
        save_library(app, sel.into_iter().collect());
    }
}

/// The symbol of the active document (`uid`, whose symbols are `names`) dragged from the panel and
/// released on `button` this frame, if any; `button` is outlined while one is held over it, also
/// when it is disabled for clicks. A symbol dragged from another document is ignored. Only the
/// release of the primary button drops the symbol; the release of another button ends the drag.
fn symbol_dropped(ui: &Ui, button: &egui::Response, names: &[String], uid: Option<u64>) -> Option<String> {
    let held = button.dnd_hover_payload::<PanelDrag>()?;
    let PanelDrag::Symbol { document, name } = &*held else { return None };
    if !names.contains(name) || Some(*document) != uid {
        return None;
    }
    widgets::drop_target_outline(ui, button.rect);
    if !ui.input(|i| i.pointer.button_released(egui::PointerButton::Primary)) {
        return None;
    }
    button.dnd_release_payload::<PanelDrag>().map(|_| name.clone())
}

/// The ids of the art dragged off the canvas of the active document `doc` (`uid`) and released
/// with Alt (Option) held on the symbol tile or row `item` this frame, if any. While such art is
/// held over `item` with Alt, `over` is set and `item` is outlined. Art of another document, or
/// whose objects `doc` no longer has, is not taken. Only the release of the primary button drops
/// the art; the release of another button ends the drag.
fn art_redefine_drop(ui: &Ui, item: &egui::Response, doc: &Document, uid: Option<u64>, over: &mut bool) -> Option<Vec<u64>> {
    if !alt_at_release(ui) {
        return None;
    }
    let held = item.dnd_hover_payload::<PanelDrag>()?;
    let PanelDrag::Art(ids) = &*held else { return None };
    *over = true;
    let ours = uid.is_some() && widgets::art_drag_doc(ui.ctx()) == uid;
    if !ours || ids.is_empty() || !ids.iter().all(|id| doc.node(*id).is_some_and(|n| !n.is_layer())) {
        return None;
    }
    widgets::drop_target_outline(ui, item.rect);
    if !ui.input(|i| i.pointer.button_released(egui::PointerButton::Primary)) {
        return None;
    }
    item.dnd_release_payload::<PanelDrag>().map(|_| ids.iter().map(|id| id.0).collect())
}

/// Whether Alt (Option) is held for a drop: as [`modifiers_at_release`] gives it.
fn alt_at_release(ui: &Ui) -> bool {
    modifiers_at_release(ui).alt
}

/// The modifiers held for a click on the widget whose interact rectangle is `rect`: when the
/// primary button last went up in this frame as a click (the pointer stayed within egui's click
/// distance of where the button went down) after going down over `rect` or within egui's interact
/// radius of it (egui gives a press that close to the widget), as it went up (egui reports the
/// click of the last such release in the frame); else, when no such click happened in this frame,
/// as the first release of the frame went up, which ends a press of an earlier frame that egui
/// counted as the click; else now. A release that ends a drag is not a click and is skipped.
/// `pointer` is whether egui reports the widget clicked by the primary button; when it is false,
/// the click came from the keyboard or a screen reader and the modifiers held now are used.
fn modifiers_released_in(ui: &Ui, rect: egui::Rect, pointer: bool) -> egui::Modifiers {
    if !pointer {
        return ui.input(|i| i.modifiers);
    }
    let radius = ui.ctx().global_style().interaction.interact_radius;
    let max_click_dist = ui.ctx().options(|o| o.input_options.max_click_dist);
    ui.input(|i| {
        // Where the button went down in this frame, and whether the pointer has since moved too far
        // for a click.
        let mut origin: Option<egui::Pos2> = None;
        let mut moved = false;
        // Whether a button went down earlier in this frame; until one does, a release ends a press
        // of an earlier frame.
        let mut pressed_here = false;
        // The modifiers of the release that ends a press of an earlier frame.
        let mut early = None;
        let mut found = None;
        for e in &i.events {
            match e {
                egui::Event::PointerMoved(pos) => {
                    moved |= origin.is_some_and(|o| o.distance(*pos) > max_click_dist);
                }
                egui::Event::PointerButton { pos, pressed: true, .. } => {
                    origin = Some(*pos);
                    moved = false;
                    pressed_here = true;
                }
                egui::Event::PointerButton { pos, button, pressed: false, modifiers } => {
                    moved |= origin.is_some_and(|o| o.distance(*pos) > max_click_dist);
                    let over = origin.is_some_and(|o| rect.distance_sq_to_pos(o) <= radius * radius);
                    if *button == egui::PointerButton::Primary {
                        if !pressed_here {
                            early = early.or(Some(*modifiers));
                        } else if !moved && over {
                            found = Some(*modifiers);
                        }
                    }
                    origin = None;
                    moved = false;
                }
                _ => {}
            }
        }
        // egui tracks a press of an earlier frame. A widget it reports clicked by the pointer, with
        // no click on it after a press of this frame, was clicked by the early release.
        found.or(early).unwrap_or(i.modifiers)
    })
}

/// The modifiers held for a drop: when the primary button went up in this frame, as it first went
/// up, which is the release that ends a drag under way when the frame starts; else now.
fn modifiers_at_release(ui: &Ui) -> egui::Modifiers {
    ui.input(|i| {
        i.events
            .iter()
            .find_map(|e| match e {
                egui::Event::PointerButton { button: egui::PointerButton::Primary, pressed: false, modifiers, .. } => Some(*modifiers),
                _ => None,
            })
            .unwrap_or(i.modifiers)
    })
}

/// The panel state key of the symbols Select All Unused picked.
const MARKED: &str = "sym-unused";

/// The panel's views: Thumbnail View (the default) and the Small and Large List Views, whose rows
/// show each symbol's thumbnail and name (`ui.panelView {panel: "symbols", view}`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum View {
    #[default]
    Thumbnail,
    SmallList,
    LargeList,
}

impl View {
    const ALL: [(View, &'static str); 3] =
        [(View::Thumbnail, "Thumbnail View"), (View::SmallList, "Small List View"), (View::LargeList, "Large List View")];
    /// Row height, thumbnail side and name size of the list views; None for the thumbnail grid.
    fn row(self) -> Option<(f32, f32, f32)> {
        match self {
            View::Thumbnail => None,
            View::SmallList => Some((18.0, 14.0, 12.0)),
            View::LargeList => Some((26.0, 22.0, 12.5)),
        }
    }
}

/// The symbols Select All Unused picked in the active document (`uid`) that it still has (`names`);
/// none once another document is active.
fn marked(ctx: &egui::Context, uid: Option<u64>, names: &[String]) -> Vec<String> {
    let (of, mut marked): (Option<u64>, Vec<String>) = pstate(ctx, MARKED);
    if of != uid {
        return vec![];
    }
    marked.retain(|m| names.contains(m));
    marked
}

/// Delete Symbol: the symbols Select All Unused picked (`marked`) when there are any, asked about
/// once unless `now` (Alt), else the current symbol `sel`.
fn delete(app: &mut VectorcraftApp, marked: &[String], sel: Option<&str>, now: bool) {
    match (marked, sel) {
        ([], Some(n)) => crate::dialogs::symbol_delete::ask(app, n, now),
        ([], None) => {}
        // Picked by Select All Unused, even one: deleted only while still unused.
        (names, _) => crate::dialogs::symbol_delete::ask_many(app, names, now),
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
}

impl LibSymbol {
    fn name(&self) -> &str {
        self.lib.doc.symbols.get(self.index).map_or("", |s| s.name.as_str())
    }
}

/// Symbol libraries in library panels (engine: `symbol.libraries`, `symbol.addFromLibrary`…).
pub(crate) struct SymbolLibraries;

impl LibraryKind for SymbolLibraries {
    const KIND: &str = "symbols";
    const OPEN: &str = "window.symbolLibrary";
    const LIST: &str = "symbol.libraries";
    const LOAD: &str = "symbol.loadLibrary";
    const MENU: &str = "Symbol Libraries Menu";
    const ADD: &str = "Add to Symbols";
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
        let shown = (0..lib.len()).map(|index| LibSymbol { lib: lib.clone(), index }).collect();
        Some((info.name, Arc::new(shown)))
    }
    fn find(app: &VectorcraftApp, key: &str) -> library_panel::Found {
        symbollib::library(&app.session, key).map(|(info, lib)| (info.id, info.name, lib.len()))
    }
    fn path(app: &VectorcraftApp, id: &str) -> Option<String> {
        app.session.symbol_libraries.path(id).map(str::to_string)
    }
    /// Find matches a symbol's name as shown and as stored.
    fn rows<'a>(lib: &'a Self::Lib, id: &str, query: &str) -> Vec<Row<'a, LibSymbol>> {
        let found = |n: &str| query.is_empty() || n.to_lowercase().contains(query) || Self::label(id, n).to_lowercase().contains(query);
        lib.iter().filter(|s| found(s.name())).map(|s| Row { name: s.name(), item: Some(s) }).collect()
    }
    /// A built-in library's symbol in the UI language; a User Defined or loaded library's symbol as
    /// it is named.
    fn label<'a>(id: &str, name: &'a str) -> Cow<'a, str> {
        super::label_or_name(name, symbollib::BUILTIN.iter().any(|b| b.id == id)).into()
    }
    /// The symbol's art on white, as the Symbols panel shows it.
    fn draw(_: &VectorcraftApp, ui: &Ui, r: egui::Rect, s: &LibSymbol, selected: bool, hovered: bool) {
        chip(ui, &s.lib.doc, &library_symbols(&s.lib), r, s.name());
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
mod tests;
