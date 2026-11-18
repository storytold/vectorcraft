//! Symbols panel: the document's symbols as rendered thumbnails or in a small or large list. Click
//! selects a symbol (it becomes the Symbol Sprayer's current symbol); double-click opens it in symbol editing mode. Symbol
//! libraries open in library panels ([`SymbolLibraries`]).

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

/// A thumbnail of symbol `name` (an instance at its natural size), cached by its key in `keys`
/// ([`Thumbs`]).
fn thumb(ui: &Ui, doc: &Document, keys: &Thumbs, name: &str, size: f32) -> Option<egui::TextureHandle> {
    thread_local! {
        static RENDERER: RefCell<vectorcraft_render::Renderer> = RefCell::new(vectorcraft_render::Renderer::new());
        static CACHE: crate::graphics::TexCache<HashMap<(u64, u32), egui::TextureHandle>> = crate::graphics::TexCache::default();
    }
    let px = (size * ui.ctx().pixels_per_point()).round().max(8.0) as u32;
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
        // Shift held at the release keeps the dropped art as ordinary art.
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
            // Both are read as the primary button went up, when it did in this frame.
            let m = modifiers_at_release(ui);
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
            delete(app, &marked, sel.as_deref(), alt_at_release(ui));
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
    // Shift keeps the selected art as ordinary art instead of making it an instance. It is read as
    // the primary button went up, when it did in this frame.
    let shift = modifiers_at_release(ui).shift;
    let redefine = if shift { json!({"name": sel, "keepArt": true}) } else { name.clone() };
    let new = if shift { json!({"keepArt": true}) } else { json!({}) };
    let items: [(&str, bool, &str, Value); 3] = [
        (tl!("New Symbol…"), has_sel, "ui.symbolOptions", new),
        (tl!("Redefine Symbol"), has_sel && sel.is_some(), "symbol.update", redefine),
        (tl!("Duplicate Symbol"), sel.is_some(), "symbol.duplicate", name.clone()),
    ];
    for (label, enabled, cmd, params) in items {
        if menu_item(ui, label, enabled, false) {
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

/// Whether Alt (Option) is held for a drop or a click: when the primary button went up in this frame, as it
/// went up, else now.
fn alt_at_release(ui: &Ui) -> bool {
    modifiers_at_release(ui).alt
}

/// The modifiers held for a drop or a click: when the primary button went up in this frame, as it went up,
/// else now.
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
    fn rows<'a>(lib: &'a Self::Lib, _: &str, query: &str) -> Vec<Row<'a, LibSymbol>> {
        let found = |n: &str| query.is_empty() || n.to_lowercase().contains(query);
        lib.iter().filter(|s| found(s.name())).map(|s| Row { name: s.name(), item: Some(s) }).collect()
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

    /// The primary button going up at `at` with Alt (Option) held.
    fn alt_release(at: Pos2) -> egui::Event {
        egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::ALT }
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
        // Dragging the symbol's tile out of the panel carries the symbol and its document.
        frame(&ctx, &mut app, vec![]);
        let tile = pos2(26.0, 26.0);
        frame(&ctx, &mut app, vec![egui::Event::PointerMoved(tile), button(tile, true)]);
        frame(&ctx, &mut app, vec![egui::Event::PointerMoved(tile + vec2(40.0, 40.0))]);
        frame(&ctx, &mut app, vec![egui::Event::PointerMoved(tile + vec2(80.0, 80.0))]);
        let document = app.session.active().unwrap().uid;
        assert_eq!(egui::DragAndDrop::payload::<PanelDrag>(&ctx).as_deref(), Some(&PanelDrag::Symbol { document, name: names[0].clone() }));
    }

    /// One headless frame of the panel menu; returns its painted labels.
    fn menu_frame(ctx: &egui::Context, app: &mut VectorcraftApp, events: Vec<egui::Event>) -> Vec<(String, Rect)> {
        let raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(236.0, 400.0))), events, ..Default::default() };
        let mut out = ctx.run_ui(raw, |ui| menu(app, ui));
        out.textures_delta.clear();
        crate::tests_removeanchors::shapes_text(&out.shapes.iter().map(|c| c.shape.clone()).collect::<Vec<_>>())
    }

    /// Clicks the menu item labelled `label`.
    fn click_menu(ctx: &egui::Context, app: &mut VectorcraftApp, label: &str) {
        let labels = menu_frame(ctx, app, vec![]);
        let item = labels.iter().find(|(t, _)| t.trim() == label).map(|(_, r)| r.center()).unwrap_or_else(|| panic!("{label} in the menu"));
        menu_frame(ctx, app, vec![egui::Event::PointerMoved(item), button(item, true)]);
        menu_frame(ctx, app, vec![button(item, false)]);
    }

    /// Clicks the menu item labeled `label` with `modifiers` held.
    fn click_menu_held(ctx: &egui::Context, app: &mut VectorcraftApp, label: &str, modifiers: egui::Modifiers) {
        click_menu_held_then(ctx, app, label, modifiers, vec![]);
    }

    /// Clicks the menu item labeled `label` with `modifiers` held, `after` following the release in
    /// its frame.
    fn click_menu_held_then(ctx: &egui::Context, app: &mut VectorcraftApp, label: &str, modifiers: egui::Modifiers, after: Vec<egui::Event>) {
        let held = egui::Event::ModifiersChanged(modifiers);
        let labels = menu_frame(ctx, app, vec![held.clone()]);
        let item = labels.iter().find(|(t, _)| t.trim() == label).map(|(_, r)| r.center()).unwrap_or_else(|| panic!("{label} in the menu"));
        let press = |pressed| egui::Event::PointerButton { pos: item, button: egui::PointerButton::Primary, pressed, modifiers };
        menu_frame(ctx, app, vec![held.clone(), egui::Event::PointerMoved(item), press(true)]);
        let mut last = vec![held, press(false)];
        last.extend(after);
        menu_frame(ctx, app, last);
    }

    /// Redefine Symbol in the panel menu with Shift held updates the symbol and leaves the
    /// selected art as ordinary art, still selected; without Shift the art becomes an instance.
    #[test]
    fn shift_redefine_symbol_in_the_menu_keeps_the_art_as_ordinary_art() {
        let mut app = leaf_and_dot();
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let rect = NodeId(app.run("shape.rectangle", json!({"x": 100, "y": 100, "width": 60, "height": 20})).unwrap()["id"].as_u64().unwrap());
        app.run("select.set", json!({"ids": [rect.0]})).unwrap();
        let dot_art = |app: &VectorcraftApp| app.session.active().unwrap().doc.symbols.iter().find(|s| s.name == "Dot").map(|s| s.art.clone());
        let before = dot_art(&app);
        click_menu_held(&ctx, &mut app, "Redefine Symbol", egui::Modifiers::SHIFT);
        assert_ne!(dot_art(&app), before, "Dot is redefined");
        assert_eq!(art(&app, "Dot"), (0, 1), "the rectangle stays ordinary art");
        assert_eq!(app.session.active().unwrap().selection.objects, vec![rect], "and stays selected");
        click_menu_held(&ctx, &mut app, "Redefine Symbol", egui::Modifiers::NONE);
        assert_eq!(art(&app, "Dot"), (1, 0), "without Shift the rectangle becomes an instance");
    }

    /// `leaf_and_dot` with a selected 60 × 20 rectangle; returns its id.
    fn leaf_dot_and_rect() -> (VectorcraftApp, NodeId) {
        let mut app = leaf_and_dot();
        let rect = NodeId(app.run("shape.rectangle", json!({"x": 100, "y": 100, "width": 60, "height": 20})).unwrap()["id"].as_u64().unwrap());
        app.run("select.set", json!({"ids": [rect.0]})).unwrap();
        (app, rect)
    }

    /// New Symbol… in the panel menu with Shift held opens Symbol Options, and its OK makes the
    /// symbol and leaves the selected art as ordinary art, still selected, in one undo step, though
    /// Shift is let go before OK; Cancel makes no symbol. Without Shift, OK makes the art an
    /// instance.
    #[test]
    fn shift_new_symbol_in_the_menu_keeps_the_art_as_ordinary_art() {
        let (mut app, rect) = leaf_dot_and_rect();
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        click_menu_held(&ctx, &mut app, "New Symbol…", egui::Modifiers::SHIFT);
        assert_eq!(dialog_kind(&app), Some(crate::dialogs::symbol_options::KIND));
        crate::dialogs::cancel(&mut app);
        assert_eq!(symbols(&mut app), ["Leaf", "Dot"], "Cancel makes no symbol");
        click_menu_held(&ctx, &mut app, "New Symbol…", egui::Modifiers::SHIFT);
        menu_frame(&ctx, &mut app, vec![egui::Event::ModifiersChanged(egui::Modifiers::NONE)]);
        let undo = undo_len(&app);
        crate::dialogs::confirm(&mut app).unwrap();
        assert_eq!(symbols(&mut app), ["Leaf", "Dot", "New Symbol"]);
        assert_eq!(art(&app, "New Symbol"), (0, 1), "the rectangle stays ordinary art");
        assert_eq!(app.session.active().unwrap().selection.objects, vec![rect], "and stays selected");
        assert_eq!(undo_len(&app), undo + 1, "one undo step");
        click_menu_held(&ctx, &mut app, "New Symbol…", egui::Modifiers::NONE);
        crate::dialogs::confirm(&mut app).unwrap();
        assert_eq!(art(&app, "New Symbol 2"), (1, 0), "without Shift the rectangle becomes an instance");
    }

    /// Clicks the panel's New Symbol button with `modifiers` held.
    fn click_new(app: &mut VectorcraftApp, modifiers: egui::Modifiers) {
        click_button_then(app, "New Symbol", modifiers, vec![]);
    }

    /// Clicks the panel's button labeled `label` with `modifiers` held, `after` following the
    /// release in its frame.
    fn click_button_then(app: &mut VectorcraftApp, label: &str, modifiers: egui::Modifiers, after: Vec<egui::Event>) {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let out = frame_held(&ctx, app, vec![], modifiers);
        let at = clickable(&out).into_iter().find(|(l, _)| l == label).unwrap_or_else(|| panic!("a {label} button")).1;
        let press = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers };
        let mut events = vec![egui::Event::PointerMoved(at), press(true), press(false)];
        events.extend(after);
        frame_held(&ctx, app, events, modifiers);
    }

    /// Shift held as the primary button goes up on New Symbol, on the panel's button or in its
    /// menu, or on Redefine Symbol in the menu, keeps the selected art as ordinary art, though Shift
    /// is let go later in the same frame. Alt held as the button goes up on Delete Symbol deletes
    /// an unused symbol without asking, in the same case.
    #[test]
    fn shift_or_alt_let_go_after_the_release_in_the_same_frame_still_counts() {
        let let_go = || vec![egui::Event::ModifiersChanged(egui::Modifiers::NONE)];
        let menu_ctx = || {
            let ctx = egui::Context::default();
            crate::theme::install_fonts(&ctx);
            ctx
        };
        let (mut app, _) = leaf_dot_and_rect();
        click_button_then(&mut app, "New Symbol", egui::Modifiers::SHIFT, let_go());
        crate::dialogs::confirm(&mut app).unwrap();
        assert_eq!(art(&app, "New Symbol"), (0, 1), "New Symbol button with Shift: the rectangle stays ordinary art");
        let (mut app, _) = leaf_dot_and_rect();
        click_button_then(&mut app, "New Symbol", egui::Modifiers::SHIFT | egui::Modifiers::ALT, let_go());
        assert_eq!(dialog_kind(&app), None, "Alt skips Symbol Options");
        assert_eq!(art(&app, "New Symbol"), (0, 1), "New Symbol button with Shift and Alt: the rectangle stays ordinary art");
        let (mut app, _) = leaf_dot_and_rect();
        click_menu_held_then(&menu_ctx(), &mut app, "New Symbol…", egui::Modifiers::SHIFT, let_go());
        crate::dialogs::confirm(&mut app).unwrap();
        assert_eq!(art(&app, "New Symbol"), (0, 1), "New Symbol… with Shift: the rectangle stays ordinary art");
        let (mut app, _) = leaf_dot_and_rect();
        click_menu_held_then(&menu_ctx(), &mut app, "Redefine Symbol", egui::Modifiers::SHIFT, let_go());
        assert_eq!(art(&app, "Dot"), (0, 1), "Redefine Symbol with Shift: the rectangle stays ordinary art");
        let mut app = leaf_and_dot();
        click_button_then(&mut app, "Delete Symbol", egui::Modifiers::ALT, let_go());
        assert!(app.ui.dialog.is_none(), "Alt deletes an unused symbol without asking");
        assert_eq!(symbols(&mut app), ["Leaf"]);
    }

    /// The New Symbol button with Shift held opens Symbol Options, whose OK keeps the selected art
    /// as ordinary art; with Shift and Alt it makes the symbol at once under the default name and
    /// keeps the art. Alt alone still makes the art an instance at once.
    #[test]
    fn shift_new_symbol_button_keeps_the_art_with_or_without_alt() {
        let (mut app, rect) = leaf_dot_and_rect();
        click_new(&mut app, egui::Modifiers::SHIFT);
        assert_eq!(dialog_kind(&app), Some(crate::dialogs::symbol_options::KIND));
        assert_eq!(symbols(&mut app).len(), 2, "no symbol before OK");
        crate::dialogs::confirm(&mut app).unwrap();
        assert_eq!(art(&app, "New Symbol"), (0, 1), "Shift: the rectangle stays ordinary art");
        assert_eq!(app.session.active().unwrap().selection.objects, vec![rect]);
        let undo = undo_len(&app);
        click_new(&mut app, egui::Modifiers::SHIFT | egui::Modifiers::ALT);
        assert_eq!(dialog_kind(&app), None, "Alt skips Symbol Options");
        assert_eq!(symbols(&mut app), ["Leaf", "Dot", "New Symbol", "New Symbol 2"]);
        assert_eq!(art(&app, "New Symbol 2"), (0, 1), "Shift and Alt: the rectangle stays ordinary art");
        assert_eq!(app.session.active().unwrap().selection.objects, vec![rect]);
        assert_eq!(undo_len(&app), undo + 1, "one undo step");
        click_new(&mut app, egui::Modifiers::ALT);
        assert_eq!(art(&app, "New Symbol 3"), (1, 0), "Alt alone: the rectangle becomes an instance");
    }

    /// The center of the panel's Symbol Options button, and of Break Link to Symbol and New Symbol
    /// beside it, in a frame of `app`.
    fn options_button(app: &mut VectorcraftApp) -> (Pos2, Pos2, Pos2) {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let found = clickable(&frame_held(&ctx, app, vec![], egui::Modifiers::NONE));
        let at = |label: &str| found.iter().find(|(l, _)| l == label).unwrap_or_else(|| panic!("a {label} button")).1;
        (at("Break Link to Symbol"), at("Symbol Options"), at("New Symbol"))
    }

    /// Clicks the panel at `at` with `modifiers` held.
    fn click_at(app: &mut VectorcraftApp, at: Pos2, modifiers: egui::Modifiers) {
        let ctx = egui::Context::default();
        frame_held(&ctx, app, vec![], modifiers);
        let press = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers };
        frame_held(&ctx, app, vec![egui::Event::PointerMoved(at), press(true), press(false)], modifiers);
    }

    /// The bottom bar's Symbol Options button sits between Break Link to Symbol and New Symbol and
    /// opens Symbol Options on the current symbol, as the panel menu's item does, with or without
    /// Alt, and adds no undo step; in a document without symbols it is disabled and opens nothing.
    #[test]
    fn symbol_options_button_edits_the_current_symbol() {
        let mut app = leaf_and_dot();
        app.run("select.all", json!({})).unwrap();
        let (break_link, options, new) = options_button(&mut app);
        assert!(break_link.x < options.x && options.x < new.x, "Break Link, Symbol Options, New Symbol: {break_link:?} {options:?} {new:?}");
        let undo = undo_len(&app);
        for (current, modifiers) in [("Dot", egui::Modifiers::NONE), ("Leaf", egui::Modifiers::ALT)] {
            app.run("symbol.setCurrent", json!({ "name": current })).unwrap();
            click_at(&mut app, options, modifiers);
            let d = app.ui.dialog.as_ref().expect("Symbol Options opens");
            assert_eq!(d.kind, crate::dialogs::symbol_options::KIND);
            assert_eq!((d.fields.get("name"), d.fields.get("__edit")), (Some(&json!(current)), Some(&json!(true))), "{modifiers:?}");
            crate::dialogs::cancel(&mut app);
        }
        assert_eq!(undo_len(&app), undo, "opening the dialog adds no undo step");
        assert_eq!(symbols(&mut app), ["Leaf", "Dot"]);
        let mut empty = VectorcraftApp::new(Session::new(), Default::default());
        empty.run("file.new", json!({"width": 300, "height": 200})).unwrap();
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        for (app, disabled) in [(&mut app, false), (&mut empty, true)] {
            let out = frame_held(&ctx, app, vec![], egui::Modifiers::NONE);
            let nodes = &out.platform_output.accesskit_update.as_ref().expect("an accessibility tree").nodes;
            let button = &nodes.iter().find(|(_, n)| n.label().is_some_and(|l| l.trim() == "Symbol Options")).expect("a Symbol Options button").1;
            assert_eq!(button.is_disabled(), disabled, "disabled without a current symbol");
        }
        click_at(&mut empty, options, egui::Modifiers::NONE);
        assert_eq!(dialog_kind(&empty), None, "no symbol, no dialog");
    }

    /// Art dropped on the panel with Shift held as the primary button goes up becomes a new
    /// symbol and stays ordinary art, still selected, though Shift is let go later in the same
    /// frame; dropped without Shift, it becomes an instance.
    #[test]
    fn shift_art_drop_on_the_panel_keeps_the_art_as_ordinary_art() {
        let (mut app, rect) = leaf_dot_and_rect();
        let ctx = egui::Context::default();
        let at = pos2(180.0, 60.0);
        let release = |modifiers| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers };
        widgets::drag_art(&ctx, uid(&app).unwrap(), vec![rect]);
        frame_held(&ctx, &mut app, vec![egui::Event::PointerMoved(at)], egui::Modifiers::SHIFT);
        let undo = undo_len(&app);
        let let_go = egui::Event::ModifiersChanged(egui::Modifiers::NONE);
        frame_held(&ctx, &mut app, vec![release(egui::Modifiers::SHIFT), let_go], egui::Modifiers::SHIFT);
        assert_eq!(symbols(&mut app).len(), 3, "a new symbol");
        assert_eq!(art(&app, "New Symbol"), (0, 1), "the rectangle stays ordinary art");
        assert_eq!(app.session.active().unwrap().selection.objects, vec![rect], "and stays selected");
        assert_eq!(undo_len(&app), undo + 1, "one undo step");
        widgets::drag_art(&ctx, uid(&app).unwrap(), vec![rect]);
        frame_held(&ctx, &mut app, vec![egui::Event::PointerMoved(at)], egui::Modifiers::NONE);
        frame_held(&ctx, &mut app, vec![release(egui::Modifiers::NONE)], egui::Modifiers::NONE);
        assert_eq!(art(&app, "New Symbol 2"), (1, 0), "without Shift the rectangle becomes an instance");
    }

    /// `ui.symbolOptions` takes `keepArt` only as true or false, and only for a new symbol.
    #[test]
    fn symbol_options_checks_keep_art() {
        let (mut app, _) = leaf_dot_and_rect();
        for bad in [json!("yes"), json!(1)] {
            let e = app.run("ui.symbolOptions", json!({ "keepArt": bad })).unwrap_err();
            assert!(e.contains("`keepArt` must be true or false"), "{bad}: {e}");
        }
        let e = app.run("ui.symbolOptions", json!({"edit": true, "name": "Dot", "keepArt": true})).unwrap_err();
        assert!(e.contains("only to a new symbol"), "{e}");
        assert_eq!(dialog_kind(&app), None);
        app.run("ui.symbolOptions", json!({"edit": true, "name": "Dot", "keepArt": false})).unwrap();
        assert_eq!(dialog_kind(&app), Some(crate::dialogs::symbol_options::KIND));
    }

    /// #4918: Select All Unused picks the symbols nothing places, Delete Symbol asks once and
    /// deletes them all in one undo step, and a click on a symbol picks one again.
    #[test]
    fn select_all_unused_picks_the_unused_symbols_and_delete_deletes_them() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
        for (i, name) in ["A", "B", "C"].into_iter().enumerate() {
            app.run("shape.rectangle", json!({"x": 10.0 + 40.0 * i as f64, "y": 10, "width": 30, "height": 20})).unwrap();
            let id = app.run("symbol.new", json!({"name": name})).unwrap()["id"].as_u64().unwrap();
            // A and C lose their instances; B keeps its own.
            if name != "B" {
                app.run("edit.clear", json!({"ids": [id]})).unwrap();
            }
        }
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        click_menu(&ctx, &mut app, "Select All Unused");
        let names = symbols(&mut app);
        assert_eq!(marked(&ctx, uid(&app), &names), ["A", "C"]);
        let undo = app.session.active().unwrap().history.undo.len();
        click_menu(&ctx, &mut app, "Delete Symbol");
        assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some(crate::dialogs::symbol_delete::KIND));
        assert_eq!(app.ui.dialog.as_ref().unwrap().fields["names"], json!(["A", "C"]), "asked once, about both");
        crate::dialogs::confirm(&mut app).unwrap();
        assert_eq!(symbols(&mut app), ["B"]);
        assert_eq!(app.session.active().unwrap().history.undo.len(), undo + 1, "one undo step");
        assert!(marked(&ctx, uid(&app), &symbols(&mut app)).is_empty(), "the deleted ones are no longer picked");
        // Undone, they are back; Select All Unused again, then a click on a symbol picks just it.
        app.run("edit.undo", json!({})).unwrap();
        click_menu(&ctx, &mut app, "Select All Unused");
        assert_eq!(marked(&ctx, uid(&app), &symbols(&mut app)).len(), 2);
        frame(&ctx, &mut app, vec![]);
        let tile = pos2(26.0, 26.0);
        frame(&ctx, &mut app, vec![egui::Event::PointerMoved(tile), button(tile, true)]);
        frame(&ctx, &mut app, vec![button(tile, false)]);
        assert!(marked(&ctx, uid(&app), &symbols(&mut app)).is_empty());
        // With nothing unused, the item reports it and picks nothing.
        app.run("symbol.delete", json!({"names": ["A", "C"]})).unwrap();
        click_menu(&ctx, &mut app, "Select All Unused");
        assert!(marked(&ctx, uid(&app), &symbols(&mut app)).is_empty());
    }

    fn uid(app: &VectorcraftApp) -> Option<u64> {
        app.session.active().map(|d| d.uid)
    }

    /// Picks made in one document don't carry to another with symbols of the same names, and a
    /// picked symbol that gained a use since (here in a brush's art) isn't deleted (#4918 review).
    #[test]
    fn picked_symbols_stay_with_their_document_and_are_rechecked_before_deleting() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        let make = |app: &mut VectorcraftApp| {
            app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
            app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 30, "height": 20})).unwrap();
            let id = app.run("symbol.new", json!({"name": "A"})).unwrap()["id"].as_u64().unwrap();
            app.run("edit.clear", json!({"ids": [id]})).unwrap();
        };
        make(&mut app);
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        click_menu(&ctx, &mut app, "Select All Unused");
        assert_eq!(marked(&ctx, uid(&app), &symbols(&mut app)), ["A"]);
        make(&mut app);
        assert!(marked(&ctx, uid(&app), &symbols(&mut app)).is_empty(), "not in the other document");
        // Back in the first one: A gains a use in a brush's art, then Delete (Alt: no question).
        click_menu(&ctx, &mut app, "Select All Unused");
        let id = app.run("symbol.place", json!({"name": "A"})).unwrap()["id"].as_u64().unwrap();
        app.run("brush.new", json!({"type": "art", "name": "B", "ids": [id]})).unwrap();
        app.run("edit.clear", json!({"ids": [id]})).unwrap();
        let names = symbols(&mut app);
        let picked = marked(&ctx, uid(&app), &names);
        assert_eq!(picked, ["A"]);
        delete(&mut app, &picked, None, true);
        assert_eq!(symbols(&mut app), ["A"], "still used by the brush, so kept");
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
        assert_eq!(app.ui.dialog.as_ref().unwrap().str("type"), "dynamic", "Symbol Type: a new symbol is dynamic");
        crate::dialogs::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
        let list = app.run("symbol.list", json!({})).unwrap();
        assert_eq!(list["symbols"][0]["exportType"], "movieClip");
        assert_eq!(list["symbols"][0]["type"], "dynamic");
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
        // Symbol Options on the current symbol renames, keeps Export Type and sets Symbol Type.
        app.run("ui.symbolOptions", json!({"edit": true, "name": "Badge"})).unwrap();
        assert_eq!(app.ui.dialog.as_ref().unwrap().str("exportType"), "graphic");
        app.ui.dialog.as_mut().unwrap().fields.insert("name".into(), json!("Mark"));
        app.ui.dialog.as_mut().unwrap().fields.insert("type".into(), json!("static"));
        crate::dialogs::confirm(&mut app).unwrap();
        assert_eq!(symbols(&mut app), ["New Symbol", "Mark"]);
        assert_eq!(app.run("symbol.list", json!({})).unwrap()["symbols"][1]["exportType"], "graphic");
        assert_eq!(app.run("symbol.list", json!({})).unwrap()["symbols"][1]["type"], "static");
        app.run("ui.symbolOptions", json!({"edit": true, "name": "Mark"})).unwrap();
        assert_eq!(app.ui.dialog.as_ref().unwrap().str("type"), "static");
        app.ui.dialog = None;
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
        app.ui.dialog = None;
        // Guides outside the art, and ones that are not whole percents, open as they are and come
        // back unchanged from OK; a field changed to a value past 100 % puts that guide outside.
        let guides = [-0.123456789, 1.5, 0.013, -2.75];
        app.run("symbol.options", json!({"name": "Mark", "nineSliceGuides": guides})).unwrap();
        app.run("ui.symbolOptions", json!({"edit": true, "name": "Mark"})).unwrap();
        let dialog = app.ui.dialog.as_ref().unwrap();
        for (key, want) in [("sliceLeft", -12.3456789), ("sliceTop", 150.0), ("sliceRight", 1.3), ("sliceBottom", -275.0)] {
            assert!((dialog.f64(key, 0.0) - want).abs() < 1e-9, "{key}: {}", dialog.f64(key, 0.0));
        }
        crate::dialogs::confirm(&mut app).unwrap();
        assert_eq!(app.run("symbol.list", json!({})).unwrap()["symbols"][1]["nineSlice"], json!(guides));
        app.run("ui.symbolOptions", json!({"edit": true, "name": "Mark"})).unwrap();
        app.ui.dialog.as_mut().unwrap().fields.insert("sliceRight".into(), json!(-125.5));
        crate::dialogs::confirm(&mut app).unwrap();
        let nine = app.run("symbol.list", json!({})).unwrap()["symbols"][1]["nineSlice"].clone();
        assert_eq!(nine, json!([-0.123456789, 1.5, -1.255, -2.75]));
    }

    /// Symbol Options › Align to Pixel Grid is off for a new symbol, OK sends it, and the dialog
    /// reopens with the symbol's setting.
    #[test]
    fn symbol_options_sets_align_to_pixel_grid() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
        app.run("shape.rectangle", json!({"x": 10.3, "y": 10.6, "width": 30, "height": 20})).unwrap();
        app.run("ui.symbolOptions", json!({})).unwrap();
        assert!(!app.ui.dialog.as_ref().unwrap().bool("alignToPixelGrid"));
        app.ui.dialog.as_mut().unwrap().fields.insert("alignToPixelGrid".into(), json!(true));
        crate::dialogs::confirm(&mut app).unwrap();
        let name = symbols(&mut app)[0].clone();
        assert_eq!(app.run("symbol.list", json!({})).unwrap()["symbols"][0]["alignToPixelGrid"], true);
        app.run("ui.symbolOptions", json!({"edit": true, "name": name})).unwrap();
        assert!(app.ui.dialog.as_ref().unwrap().bool("alignToPixelGrid"));
        app.ui.dialog.as_mut().unwrap().fields.insert("alignToPixelGrid".into(), json!(false));
        crate::dialogs::confirm(&mut app).unwrap();
        assert_eq!(app.run("symbol.list", json!({})).unwrap()["symbols"][0]["alignToPixelGrid"], false);
    }

    /// Does the selection hold a symbol instance that `wanted` takes, found by walking every
    /// selected subtree on each call: what the cached flags must match.
    fn walked(app: &VectorcraftApp, wanted: impl Fn(&Node) -> bool) -> bool {
        app.session.active().is_some_and(|st| {
            st.selection.objects.iter().any(|id| {
                let mut any = false;
                if let Some(n) = st.doc.node(*id) {
                    n.walk(&mut |c| any |= matches!(c.kind, NodeKind::SymbolInstance { .. }) && wanted(c));
                }
                any
            })
        })
    }

    /// Any instance, one with appearance changes, one Reset Transformation changes: walked again.
    fn walked_flags(app: &VectorcraftApp) -> [bool; 3] {
        let Some(doc) = app.session.active().map(|st| st.doc.clone()) else { return [false; 3] };
        [
            walked(app, |_| true),
            walked(app, |c| !c.extras().symbol_overrides.is_empty()),
            walked(app, |c| vectorcraft_engine::cmd::reset_transformation_changes(&doc, c)),
        ]
    }

    fn cached_flags(app: &mut VectorcraftApp) -> [bool; 3] {
        [has_instances_selected(app), changed_instances_selected(app), transformed_instances_selected(app)]
    }

    fn scans() -> usize {
        INSTANCE_SCANS.with(std::cell::Cell::get)
    }

    /// The Symbols and Properties panels' instance flags (Break Link, Reset Symbol Instance and the
    /// Properties panel's Reset, Reset Transformation) are those a walk of the selection gives, for
    /// plain, changed, scaled and grouped instances, several selected objects, nothing selected,
    /// Undo and another document; the selection is walked once per document revision, however
    /// often they are read.
    #[test]
    fn instance_flags_are_found_once_per_revision_and_match_a_walk_of_the_selection() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 400, "height": 200})).unwrap();
        let id = |v: Value| v["id"].as_u64().unwrap();
        let rect = id(app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 40, "height": 40})).unwrap());
        let second = id(app.run("shape.rectangle", json!({"x": 60, "y": 10, "width": 40, "height": 40})).unwrap());
        let other = id(app.run("shape.rectangle", json!({"x": 300, "y": 10, "width": 40, "height": 40})).unwrap());
        // Reads every flag several times; the walk runs only when the revision changed.
        let check = |app: &mut VectorcraftApp, want: [bool; 3], what: &str| {
            let before = scans();
            for _ in 0..3 {
                assert_eq!(cached_flags(app), want, "{what}");
            }
            assert_eq!(walked_flags(app), want, "{what}: a walk of the selection agrees");
            assert!(scans() - before <= 1, "{what}: walked {} times", scans() - before);
        };
        app.run("select.set", json!({"ids": [rect]})).unwrap();
        check(&mut app, [false; 3], "no instance");
        // Two objects, so the instance's art is a group whose first object can be painted.
        let inst = id(app.run("symbol.new", json!({"name": "S", "ids": [rect, second], "type": "dynamic"})).unwrap());
        app.run("select.set", json!({"ids": [inst]})).unwrap();
        check(&mut app, [true, false, false], "a plain instance");
        app.run("symbol.instance.setAppearance", json!({"id": inst, "path": [0], "fill": "#ff0000"})).unwrap();
        app.run("select.set", json!({"ids": [inst]})).unwrap();
        check(&mut app, [true, true, false], "an instance with appearance changes");
        let scaled = id(app.run("symbol.place", json!({"name": "S", "x": 200, "y": 100})).unwrap());
        app.run("select.set", json!({"ids": [scaled]})).unwrap();
        app.run("object.scale", json!({"sx": 150, "sy": 50})).unwrap();
        check(&mut app, [true, false, true], "a scaled instance");
        app.run("select.set", json!({"ids": [scaled, other]})).unwrap();
        let group = id(app.run("object.group", json!({})).unwrap());
        app.run("select.set", json!({"ids": [group]})).unwrap();
        check(&mut app, [true, false, true], "an instance in a selected group");
        app.run("select.set", json!({"ids": [inst, group]})).unwrap();
        check(&mut app, [true, true, true], "several selected objects");
        app.run("select.set", json!({"ids": [other]})).unwrap();
        check(&mut app, [false; 3], "select.set to a path");
        app.run("select.none", json!({})).unwrap();
        check(&mut app, [false; 3], "nothing selected");
        // Undo takes the group apart again.
        app.run("edit.undo", json!({})).unwrap();
        let undone = walked_flags(&app);
        check(&mut app, undone, "after Undo");
        // At one revision the walk runs once; a new revision walks again.
        app.run("select.set", json!({"ids": [inst, scaled]})).unwrap();
        let before = scans();
        cached_flags(&mut app);
        cached_flags(&mut app);
        assert_eq!(scans(), before + 1);
        app.run("select.set", json!({"ids": [inst]})).unwrap();
        cached_flags(&mut app);
        assert_eq!(scans(), before + 2);
        // Another document has its own uid, so its flags are found again.
        app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
        let plain = id(app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 40, "height": 40})).unwrap());
        app.run("select.set", json!({"ids": [plain]})).unwrap();
        check(&mut app, [false; 3], "another document");
    }

    /// The Properties panel (its Reset quick action) and the Symbols panel (Break Link) share one
    /// walk of the selection while the document does not change.
    #[test]
    fn properties_and_symbols_panels_walk_the_selection_once_per_revision() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
        let rect = |app: &mut VectorcraftApp, x: f64| {
            app.run("shape.rectangle", json!({"x": x, "y": 10, "width": 40, "height": 40})).unwrap()["id"].as_u64().unwrap()
        };
        let ids = [rect(&mut app, 10.0), rect(&mut app, 60.0)];
        let inst = app.run("symbol.new", json!({"name": "S", "ids": ids, "type": "dynamic"})).unwrap()["id"].as_u64().unwrap();
        app.run("symbol.instance.setAppearance", json!({"id": inst, "path": [0], "fill": "#ff0000"})).unwrap();
        app.run("select.set", json!({"ids": [inst]})).unwrap();
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let before = scans();
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| super::super::properties::show(&mut app, ui));
        out.textures_delta.clear();
        assert_eq!(scans(), before + 1, "the Properties panel reads the flags");
        for _ in 0..3 {
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                super::super::properties::show(&mut app, ui);
                show(&mut app, ui);
            });
            out.textures_delta.clear();
        }
        assert_eq!(scans(), before + 1, "one walk for both panels over four frames");
        assert_eq!(cached_flags(&mut app), [true, true, false]);
        assert_eq!(scans(), before + 1);
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

    /// A document with symbol "Leaf", whose instance stands where its rectangle was, and symbol
    /// "Dot", which no instance uses; "Dot" is current.
    fn leaf_and_dot() -> VectorcraftApp {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
        for (name, x) in [("Leaf", 10), ("Dot", 150)] {
            app.run("shape.rectangle", json!({"x": x, "y": 10, "width": 30, "height": 30})).unwrap();
            app.run("symbol.new", json!({"name": name})).unwrap();
        }
        // Dot's instance goes: no art uses Dot.
        app.run("edit.clear", json!({})).unwrap();
        app.run("symbol.setCurrent", json!({"name": "Dot"})).unwrap();
        app
    }

    /// The instances of symbol `name` in the active document, and the ordinary paths in it.
    fn art(app: &VectorcraftApp, name: &str) -> (usize, usize) {
        let (mut instances, mut paths) = (0, 0);
        for l in &app.session.active().unwrap().doc.layers {
            l.walk(&mut |n| match &n.kind {
                NodeKind::SymbolInstance { symbol, .. } if symbol == name => instances += 1,
                NodeKind::Path { .. } => paths += 1,
                _ => {}
            });
        }
        (instances, paths)
    }

    /// The bounds of the art on the symbol editing layer, rounded to 0.001 pt.
    fn editing_art(app: &VectorcraftApp) -> Option<[f64; 4]> {
        let d = &app.session.active()?.doc;
        let layer = d.node(d.symbol_edit.as_ref()?.layer)?;
        let ids: Vec<NodeId> = layer.children()?.iter().map(|n| n.id).collect();
        let r = d.bounds_of(&ids, false)?;
        Some([r.x0, r.y0, r.x1, r.y1].map(|v| (v * 1000.0).round() / 1000.0))
    }

    /// Edit Symbol without an instance centers the symbol's art on the active artboard, and in a
    /// document without artboards on the view's center, in a batch too.
    #[test]
    fn edit_symbol_from_the_panel_centers_the_art_on_the_active_artboard_or_the_view() {
        let mut app = leaf_and_dot();
        app.run("artboard.new", json!({"x": 500, "y": 0, "width": 200, "height": 100})).unwrap();
        app.run("select.none", json!({})).unwrap();
        crate::panels::artboards::select(&mut app, 1);
        app.run("symbol.edit", json!({"name": "Dot"})).unwrap();
        assert_eq!(editing_art(&app), Some([585.0, 35.0, 615.0, 65.0]));
        app.run("symbol.exitEditing", json!({})).unwrap();
        std::sync::Arc::make_mut(&mut app.session.active_mut().unwrap().doc).artboards.clear();
        // A view with no artboard to fit to is never fitted.
        if let Some(v) = app.view_mut() {
            (v.fitted, v.center) = (false, vectorcraft_geom::Point::new(1000.0, 1000.0));
        }
        app.run("symbol.edit", json!({"name": "Dot"})).unwrap();
        assert_eq!(editing_art(&app), Some([985.0, 985.0, 1015.0, 1015.0]));
        // A coordinate the call gives stays; the other is the view's.
        app.run("symbol.exitEditing", json!({})).unwrap();
        app.run("symbol.edit", json!({"name": "Dot", "x": 2000})).unwrap();
        assert_eq!(editing_art(&app), Some([1985.0, 985.0, 2015.0, 1015.0]));
        // A batch's symbol.edit step gets the view's center as well.
        app.run("symbol.exitEditing", json!({})).unwrap();
        app.run("command.batch", json!({"commands": [{"command": "symbol.edit", "params": {"name": "Dot"}}]})).unwrap();
        assert_eq!(editing_art(&app), Some([985.0, 985.0, 1015.0, 1015.0]));
        // An artboard an earlier step makes is there when the symbol.edit step runs.
        app.run("symbol.exitEditing", json!({})).unwrap();
        let steps = json!([
            {"command": "artboard.new", "params": {"x": 500, "y": 0, "width": 200, "height": 100}},
            {"command": "symbol.edit", "params": {"name": "Dot"}},
        ]);
        app.run("command.batch", json!({ "commands": steps })).unwrap();
        assert_eq!(editing_art(&app), Some([585.0, 35.0, 615.0, 65.0]));
    }

    /// The symbol in symbol editing mode, if any.
    fn editing(app: &VectorcraftApp) -> Option<String> {
        app.session.active()?.doc.symbol_edit.as_ref().map(|e| e.symbol.clone())
    }

    /// The first instance of symbol `name` in the active document.
    fn instance_of(app: &VectorcraftApp, name: &str) -> Option<NodeId> {
        let mut found = None;
        for l in &app.session.active()?.doc.layers {
            l.walk(&mut |n| {
                if let NodeKind::SymbolInstance { symbol, .. } = &n.kind
                    && symbol == name
                    && found.is_none()
                {
                    found = Some(n.id);
                }
            });
        }
        found
    }

    fn double_click(at: Pos2) -> Vec<egui::Event> {
        vec![egui::Event::PointerMoved(at), button(at, true), button(at, false), button(at, true), button(at, false)]
    }

    /// A double-click on a symbol in the panel, in Thumbnail View and in Large List View, opens it in
    /// symbol editing mode and places no instance; a single click only makes it current.
    #[test]
    fn double_clicking_a_symbol_in_the_panel_edits_it_in_either_view() {
        let mut app = leaf_and_dot();
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        // Thumbnail View: the first tile is Leaf's.
        let tile = pos2(26.0, 26.0);
        frame(&ctx, &mut app, vec![]);
        frame(&ctx, &mut app, vec![egui::Event::PointerMoved(tile), button(tile, true), button(tile, false)]);
        assert_eq!((editing(&app), current(&app)), (None, Some("Leaf".into())), "a click makes it current");
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        frame(&ctx, &mut app, vec![]);
        frame(&ctx, &mut app, double_click(tile));
        assert_eq!(editing(&app).as_deref(), Some("Leaf"));
        assert_eq!(art(&app, "Leaf").0, 1, "no instance is placed");
        app.run("symbol.exitEditing", json!({})).unwrap();
        // Large List View: the row named Dot.
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        app.ui.symbols_view = View::LargeList;
        let raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(236.0, 400.0))), ..Default::default() };
        let mut out = ctx.run_ui(raw, |ui| show(&mut app, ui));
        out.textures_delta.clear();
        let labels = crate::tests_removeanchors::shapes_text(&out.shapes.iter().map(|c| c.shape.clone()).collect::<Vec<_>>());
        let row = labels.iter().find(|(t, _)| t.trim() == "Dot").map(|(_, r)| r.center()).expect("a row named Dot");
        frame(&ctx, &mut app, double_click(row));
        assert_eq!(editing(&app).as_deref(), Some("Dot"));
        assert_eq!(art(&app, "Dot").0, 0, "no instance is placed");
    }

    /// The names the panel draws (none in Thumbnail View) and where, in one headless frame.
    fn panel_labels(ctx: &egui::Context, app: &mut VectorcraftApp, names: &[&str]) -> Vec<(String, Pos2)> {
        let raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(236.0, 400.0))), ..Default::default() };
        let mut out = ctx.run_ui(raw, |ui| show(app, ui));
        out.textures_delta.clear();
        let labels = crate::tests_removeanchors::shapes_text(&out.shapes.iter().map(|c| c.shape.clone()).collect::<Vec<_>>());
        labels.into_iter().filter(|(t, _)| names.contains(&t.trim())).map(|(t, r)| (t.trim().to_string(), r.center())).collect()
    }

    /// The view items of the panel menu, each with whether it is checked.
    fn view_items(ctx: &egui::Context, app: &mut VectorcraftApp) -> Vec<(String, bool)> {
        let labels = menu_frame(ctx, app, vec![]);
        let items = labels.into_iter().map(|(t, _)| (t.trim_start_matches('✓').trim().to_string(), t.starts_with('✓')));
        items.filter(|(t, _)| t.ends_with("View")).collect()
    }

    /// The panel menu's Sort by Name puts the symbols in name order, ignoring case, as one undo
    /// step, and adds none when they are already in order. The menu offers Thumbnail View (the
    /// default), Small List View and Large List View, one checked at a time. Both list views draw a
    /// row with a thumbnail and the name for each symbol, the small one in shorter rows, and in each
    /// view a click makes a symbol current and a double-click edits it.
    #[test]
    fn sort_by_name_and_the_small_and_large_list_views() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 300, "height": 100})).unwrap();
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        for (i, name) in ["bee", "Cat", "ant"].into_iter().enumerate() {
            app.run("shape.rectangle", json!({"x": 10.0 + 40.0 * i as f64, "y": 10, "width": 30, "height": 20})).unwrap();
            app.run("symbol.new", json!({"name": name})).unwrap();
        }
        let undo = app.session.active().unwrap().history.undo.len();
        click_menu(&ctx, &mut app, "Sort by Name");
        assert_eq!(symbols(&mut app), ["ant", "bee", "Cat"]);
        assert_eq!(app.session.active().unwrap().history.undo.len(), undo + 1, "one undo step");
        let views = |checked: &str| ["Thumbnail View", "Small List View", "Large List View"].map(|v| (v.to_string(), v == checked)).to_vec();
        assert_eq!(view_items(&ctx, &mut app), views("Thumbnail View"), "Thumbnail View by default");
        assert!(panel_labels(&ctx, &mut app, &["ant", "bee", "Cat"]).is_empty(), "the thumbnails draw no names");
        // Each list view: a row per symbol in panel order, the small rows 18 points high and the
        // large ones 26, with the list's 3-point spacing between them.
        let mut gaps = vec![];
        for (view, gap) in [("Small List View", 21.0), ("Large List View", 29.0)] {
            click_menu(&ctx, &mut app, view);
            assert_eq!(view_items(&ctx, &mut app), views(view));
            let rows = panel_labels(&ctx, &mut app, &["ant", "bee", "Cat"]);
            let order: Vec<&str> = rows.iter().map(|(t, _)| t.as_str()).collect();
            assert_eq!(order, ["ant", "bee", "Cat"], "{view}");
            let step = rows[1].1.y - rows[0].1.y;
            assert!((step - gap).abs() < 0.5, "{view}: rows {step} apart");
            gaps.push(step);
            // A click on the last row makes Cat current, a double-click edits it.
            let row = rows[2].1;
            app.run("symbol.setCurrent", json!({"name": "ant"})).unwrap();
            frame(&ctx, &mut app, vec![egui::Event::PointerMoved(row), button(row, true), button(row, false)]);
            assert_eq!((editing(&app), current(&app)), (None, Some("Cat".into())), "{view}");
            let ctx2 = egui::Context::default();
            crate::theme::install_fonts(&ctx2);
            frame(&ctx2, &mut app, vec![]);
            frame(&ctx2, &mut app, double_click(row));
            assert_eq!(editing(&app).as_deref(), Some("Cat"), "{view}");
            app.run("symbol.exitEditing", json!({})).unwrap();
        }
        assert!(gaps[0] < gaps[1], "the small rows are shorter");
        click_menu(&ctx, &mut app, "Thumbnail View");
        assert_eq!(view_items(&ctx, &mut app), views("Thumbnail View"));
        // Already in order, Sort by Name adds no undo step.
        let undo = app.session.active().unwrap().history.undo.len();
        click_menu(&ctx, &mut app, "Sort by Name");
        assert_eq!(app.session.active().unwrap().history.undo.len(), undo);
    }

    /// `ui.panelView` sets the Symbols, Graphic Styles and Swatches panels' views as the view items
    /// of their panel menus do, and answers the current view without `view`. An unknown panel or
    /// view, or a view of another panel, is an error and changes nothing.
    #[test]
    fn panel_view_sets_the_view_of_each_panel() {
        use super::super::{graphic_styles, swatches};
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 300, "height": 100})).unwrap();
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        for (i, name) in ["ant", "bee"].into_iter().enumerate() {
            app.run("shape.rectangle", json!({"x": 10.0 + 40.0 * i as f64, "y": 10, "width": 30, "height": 20})).unwrap();
            app.run("symbol.new", json!({"name": name})).unwrap();
        }
        let views = |checked: &str| ["Thumbnail View", "Small List View", "Large List View"].map(|v| (v.to_string(), v == checked)).to_vec();
        assert_eq!(app.run("ui.panelView", json!({"panel": "symbols"})).unwrap(), json!({"panel": "symbols", "view": "thumbnail"}));
        assert!(panel_labels(&ctx, &mut app, &["ant", "bee"]).is_empty(), "the thumbnails draw no names");
        let r = app.run("ui.panelView", json!({"panel": "symbols", "view": "largeList"})).unwrap();
        assert_eq!(r, json!({"panel": "symbols", "view": "largeList"}));
        assert_eq!(view_items(&ctx, &mut app), views("Large List View"), "the panel menu checks it");
        let names: Vec<String> = panel_labels(&ctx, &mut app, &["ant", "bee"]).into_iter().map(|(t, _)| t).collect();
        assert_eq!(names, ["ant", "bee"], "the list names the symbols");
        click_menu(&ctx, &mut app, "Small List View");
        assert_eq!(app.run("ui.panelView", json!({"panel": "symbols"})).unwrap()["view"], "smallList", "the menu item sets it too");
        for bad in [
            json!({"panel": "symbols", "view": "mediumThumb"}),
            json!({"panel": "symbols", "view": 3}),
            json!({"panel": "brushes", "view": "smallList"}),
            json!({"view": "largeList"}),
        ] {
            assert!(app.run("ui.panelView", bad.clone()).is_err(), "{bad}");
        }
        assert_eq!(app.ui.symbols_view, View::SmallList, "unchanged");
        let r = app.run("ui.panelView", json!({"panel": "graphicStyles", "view": "largeList"})).unwrap();
        assert_eq!(r, json!({"panel": "graphicStyles", "view": "largeList"}));
        assert_eq!(app.ui.graphic_styles_view, graphic_styles::View::LargeList);
        assert_eq!(app.run("ui.panelView", json!({"panel": "swatches"})).unwrap()["view"], "mediumThumb");
        app.run("ui.panelView", json!({"panel": "swatches", "view": "largeThumb"})).unwrap();
        assert!(app.run("ui.panelView", json!({"panel": "swatches", "view": "thumbnail"})).is_err());
        assert_eq!(app.ui.swatches_view, swatches::View::LargeThumb, "unchanged by the error");
        assert_eq!(app.ui.symbols_view, View::SmallList, "each panel keeps its own view");
    }

    /// The panel menu's Edit Symbol edits the selected instance's symbol, and with no instance
    /// selected the symbol current in the panel.
    #[test]
    fn the_panel_menus_edit_symbol_edits_the_instances_symbol_or_the_panels() {
        let mut app = leaf_and_dot();
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        click_menu(&ctx, &mut app, "Edit Symbol");
        assert_eq!(editing(&app).as_deref(), Some("Dot"), "no instance selected: the panel's symbol");
        app.run("symbol.exitEditing", json!({})).unwrap();
        let leaf = instance_of(&app, "Leaf").expect("a Leaf instance");
        app.run("select.set", json!({"ids": [leaf.0]})).unwrap();
        click_menu(&ctx, &mut app, "Edit Symbol");
        assert_eq!(editing(&app).as_deref(), Some("Leaf"), "the selected instance's symbol");
        assert_eq!(instance_of(&app, "Leaf"), Some(leaf), "the instance keeps its link");
    }

    /// With instances of one symbol selected, the Control bar's Edit Symbol opens that symbol in
    /// symbol editing mode and keeps the instance linked.
    #[test]
    fn the_control_bars_edit_symbol_edits_the_selected_instances_symbol() {
        let mut app = leaf_and_dot();
        let leaf = instance_of(&app, "Leaf").expect("a Leaf instance");
        app.run("select.set", json!({"ids": [leaf.0]})).unwrap();
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        ctx.enable_accesskit();
        let bar = |app: &mut VectorcraftApp, events: Vec<egui::Event>| {
            let raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(800.0, 400.0))), events, ..Default::default() };
            let mut out = ctx.run_ui(raw, |ui| {
                ui.horizontal(|ui| control_bar(app, ui, 0.0));
            });
            out.textures_delta.clear();
            out
        };
        let out = bar(&mut app, vec![]);
        let at = clickable(&out).into_iter().find(|(l, _)| l == "Edit Symbol").expect("an Edit Symbol button").1;
        bar(&mut app, vec![egui::Event::PointerMoved(at), button(at, true), button(at, false)]);
        assert_eq!(editing(&app).as_deref(), Some("Leaf"));
        assert_eq!(instance_of(&app, "Leaf"), Some(leaf), "the instance keeps its link");
    }

    fn dialog_kind(app: &VectorcraftApp) -> Option<&str> {
        app.ui.dialog.as_ref().map(|d| d.kind.as_str())
    }

    /// One frame of the panel with `modifiers` held, its output kept.
    fn frame_held(ctx: &egui::Context, app: &mut VectorcraftApp, events: Vec<egui::Event>, modifiers: egui::Modifiers) -> egui::FullOutput {
        let screen_rect = Some(Rect::from_min_size(Pos2::ZERO, vec2(236.0, 400.0)));
        let events = std::iter::once(egui::Event::ModifiersChanged(modifiers)).chain(events).collect();
        let mut out = ctx.run_ui(egui::RawInput { screen_rect, events, ..Default::default() }, |ui| show(app, ui));
        out.textures_delta.clear();
        out
    }

    /// The labels of the clickable accessibility nodes of `out`, with their centers.
    fn clickable(out: &egui::FullOutput) -> Vec<(String, Pos2)> {
        let nodes = &out.platform_output.accesskit_update.as_ref().expect("an accessibility tree").nodes;
        nodes
            .iter()
            .filter(|(_, n)| n.supports_action(egui::accesskit::Action::Click))
            .filter_map(|(_, n)| {
                let b = n.bounds()?;
                Some((n.label()?.trim().to_string(), pos2(((b.x0 + b.x1) / 2.0) as f32, ((b.y0 + b.y1) / 2.0) as f32)))
            })
            .collect()
    }

    /// Clicks the panel's Delete Symbol button with `modifiers` held.
    fn click_delete(app: &mut VectorcraftApp, modifiers: egui::Modifiers) {
        click_button_then(app, "Delete Symbol", modifiers, vec![]);
    }

    /// Clicks the panel menu's Delete Symbol item.
    fn menu_delete(app: &mut VectorcraftApp) {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let labels = menu_frame(&ctx, app, vec![]);
        let item = labels.iter().find(|(t, _)| t.trim() == "Delete Symbol").map(|(_, r)| r.center()).expect("Delete Symbol in the menu");
        menu_frame(&ctx, app, vec![egui::Event::PointerMoved(item), button(item, true)]);
        menu_frame(&ctx, app, vec![button(item, false)]);
    }

    /// Delete Symbol, from the panel's button or its menu, asks "Delete the selected symbol?" for a
    /// symbol no instance uses, and the in-use question for one with instances, before changing
    /// the document; No leaves it as it was and Yes deletes the symbol. Alt-click on the button
    /// deletes an unused symbol without asking, and still asks for one with instances.
    #[test]
    fn delete_symbol_asks_first_and_alt_click_deletes_an_unused_symbol_at_once() {
        let mut app = leaf_and_dot();
        let before = app.session.active().unwrap().doc.clone();
        let unchanged = |app: &VectorcraftApp| Arc::ptr_eq(&before, &app.session.active().unwrap().doc);
        click_delete(&mut app, egui::Modifiers::NONE);
        assert_eq!(dialog_kind(&app), Some(crate::dialogs::symbol_delete::KIND));
        assert!(unchanged(&app), "nothing changes while it asks");
        crate::dialogs::cancel(&mut app);
        assert!(app.ui.dialog.is_none());
        assert!(unchanged(&app), "No changes nothing");
        menu_delete(&mut app);
        assert_eq!(dialog_kind(&app), Some(crate::dialogs::symbol_delete::KIND), "the menu item asks too");
        assert!(unchanged(&app));
        crate::dialogs::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
        assert_eq!(symbols(&mut app), ["Leaf"], "Yes deletes it");
        // Alt-click on the button: an unused symbol goes without a question.
        app.run("edit.undo", json!({})).unwrap();
        app.run("symbol.setCurrent", json!({"name": "Dot"})).unwrap();
        click_delete(&mut app, egui::Modifiers::ALT);
        assert!(app.ui.dialog.is_none());
        assert_eq!(symbols(&mut app), ["Leaf"]);
        // A symbol with an instance asks, with Alt held too, from the button and from the menu.
        app.run("symbol.setCurrent", json!({"name": "Leaf"})).unwrap();
        let before = app.session.active().unwrap().doc.clone();
        for alt in [egui::Modifiers::NONE, egui::Modifiers::ALT] {
            click_delete(&mut app, alt);
            assert_eq!(dialog_kind(&app), Some(crate::dialogs::symbol_delete::USED), "{alt:?}");
            crate::dialogs::cancel(&mut app);
        }
        menu_delete(&mut app);
        assert_eq!(dialog_kind(&app), Some(crate::dialogs::symbol_delete::USED));
        crate::dialogs::cancel(&mut app);
        assert!(Arc::ptr_eq(&before, &app.session.active().unwrap().doc), "Cancel changes nothing");
        assert_eq!(art(&app, "Leaf"), (1, 0));
    }

    /// The in-use question: Expand Instances deletes the symbol and turns its instance into
    /// ordinary art, Delete Instances deletes the symbol and its instance, each in one undo step.
    #[test]
    fn the_in_use_question_expands_or_deletes_the_instances() {
        for (discard, left) in [(false, (0, 1)), (true, (0, 0))] {
            let mut app = leaf_and_dot();
            crate::dialogs::symbol_delete::ask(&mut app, "Leaf", false);
            assert_eq!(dialog_kind(&app), Some(crate::dialogs::symbol_delete::USED));
            if discard {
                app.ui.dialog.as_mut().unwrap().fields.insert("discard".into(), json!(true));
            }
            crate::dialogs::confirm(&mut app).unwrap();
            assert!(app.ui.dialog.is_none());
            assert_eq!(symbols(&mut app), ["Dot"], "discard {discard}");
            assert_eq!(art(&app, "Leaf"), left, "discard {discard}");
            app.run("edit.undo", json!({})).unwrap();
            assert_eq!((symbols(&mut app), art(&app, "Leaf")), (vec!["Leaf".to_string(), "Dot".to_string()], (1, 0)), "one undo step");
        }
    }

    /// A question asked in one document deletes nothing once another is active, and Yes for a
    /// symbol that has gained an instance since asks the in-use question instead.
    #[test]
    fn a_stale_delete_symbol_question_deletes_nothing() {
        let mut app = leaf_and_dot();
        crate::dialogs::symbol_delete::ask(&mut app, "Dot", false);
        let asked = app.ui.dialog.clone();
        app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
        app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 30, "height": 30})).unwrap();
        app.run("symbol.new", json!({"name": "Dot"})).unwrap();
        app.run("edit.clear", json!({})).unwrap();
        app.ui.dialog = asked;
        crate::dialogs::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
        assert_eq!(symbols(&mut app), ["Dot"], "the other document keeps its symbol");
        // Back in the first document: Dot gains an instance while the question is open.
        app.run("document.activate", json!({"index": 0})).unwrap();
        crate::dialogs::symbol_delete::ask(&mut app, "Dot", false);
        assert_eq!(dialog_kind(&app), Some(crate::dialogs::symbol_delete::KIND));
        app.run("symbol.place", json!({"name": "Dot"})).unwrap();
        crate::dialogs::confirm(&mut app).unwrap();
        assert_eq!(dialog_kind(&app), Some(crate::dialogs::symbol_delete::USED));
        assert_eq!(symbols(&mut app), ["Leaf", "Dot"]);
        assert_eq!(art(&app, "Dot"), (1, 0));
    }

    /// The questions' buttons: Yes and No for an unused symbol; Expand Instances, Delete Instances
    /// and Cancel for one with instances.
    #[test]
    fn the_delete_symbol_questions_show_their_buttons() {
        let mut app = leaf_and_dot();
        for (name, want) in [("Dot", &["Yes", "No"][..]), ("Leaf", &["Expand Instances", "Delete Instances", "Cancel"][..])] {
            crate::dialogs::symbol_delete::ask(&mut app, name, false);
            let ctx = egui::Context::default();
            ctx.enable_accesskit();
            // The window measures itself in its first frame and draws its buttons in the next.
            let mut out = None;
            for _ in 0..2 {
                let raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0))), ..Default::default() };
                let mut o = ctx.run_ui(raw, |ui| crate::dialogs::show(&mut app, ui.ctx()));
                o.textures_delta.clear();
                out = Some(o);
            }
            let out = out.expect("a frame");
            let labels: Vec<String> = clickable(&out).into_iter().map(|(l, _)| l).collect();
            for w in want {
                assert!(labels.iter().any(|l| l == w), "{name}: {w} in {labels:?}");
            }
            assert!(!labels.iter().any(|l| l == "OK"), "{name}: {labels:?}");
            crate::dialogs::cancel(&mut app);
        }
    }

    /// #4121: with symbol instances selected, the Control bar's Replace menu lists the document's
    /// symbols and shows the instance's own; picking another replaces it.
    #[test]
    fn the_control_bar_replace_menu_swaps_the_selected_instances_symbol() {
        fn bar(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<egui::Event>) -> Vec<egui::Shape> {
            let raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(800.0, 400.0))), events, ..Default::default() };
            let mut out = ctx.run_ui(raw, |ui| {
                ui.horizontal(|ui| control_bar(app, ui, 0.0));
            });
            out.textures_delta.clear();
            out.shapes.into_iter().map(|c| c.shape).collect()
        }
        fn text_at(shapes: &[egui::Shape], s: &str) -> Option<Pos2> {
            shapes.iter().find_map(|sh| match sh {
                egui::Shape::Text(t) if t.galley.text().trim() == s => Some(t.visual_bounding_rect().center()),
                egui::Shape::Vec(v) => text_at(v, s),
                _ => None,
            })
        }
        fn click(app: &mut VectorcraftApp, ctx: &egui::Context, at: Pos2) {
            bar(app, ctx, vec![egui::Event::PointerMoved(at), button(at, true)]);
            bar(app, ctx, vec![button(at, false)]);
        }
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
        for (name, x) in [("Leaf", 10), ("Star", 150)] {
            app.run("shape.rectangle", json!({"x": x, "y": 10, "width": 30, "height": 30})).unwrap();
            app.run("symbol.new", json!({"name": name})).unwrap();
        }
        let instance_of = |app: &VectorcraftApp, name: &str| {
            let mut found = None;
            for l in &app.session.active().unwrap().doc.layers {
                l.walk(&mut |n| {
                    if let NodeKind::SymbolInstance { symbol, .. } = &n.kind
                        && symbol == name
                    {
                        found = Some(n.id);
                    }
                });
            }
            found
        };
        let leaf = instance_of(&app, "Leaf").expect("a Leaf instance");
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        // Art, not an instance: no Replace menu.
        let rect = app.run("shape.rectangle", json!({"x": 10, "y": 100, "width": 30, "height": 30})).unwrap()["id"].as_u64().unwrap();
        app.run("select.set", json!({"ids": [rect]})).unwrap();
        assert!(text_at(&bar(&mut app, &ctx, vec![]), "Replace:").is_none());
        app.run("select.set", json!({"ids": [leaf.0]})).unwrap();
        bar(&mut app, &ctx, vec![]);
        let shapes = bar(&mut app, &ctx, vec![]);
        assert!(text_at(&shapes, "Replace:").is_some());
        let shown = text_at(&shapes, "Leaf").expect("the menu shows the instance's symbol");
        click(&mut app, &ctx, shown);
        let star = text_at(&bar(&mut app, &ctx, vec![]), "Star").expect("the menu lists the other symbol");
        click(&mut app, &ctx, star);
        let doc = &app.session.active().unwrap().doc;
        assert!(matches!(&doc.node(leaf).unwrap().kind, NodeKind::SymbolInstance { symbol, .. } if symbol == "Star"), "the instance is replaced");
    }

    /// The panel menu's Reset Transformation and the Control bar's Reset are enabled only while a
    /// selected instance's scale, rotation or shear differs from its symbol's, and give the
    /// instance its symbol's again.
    #[test]
    fn reset_transformation_is_enabled_for_a_transformed_instance_and_resets_it() {
        let mut app = leaf_and_dot();
        let leaf = instance_of(&app, "Leaf").expect("a Leaf instance");
        app.run("select.set", json!({"ids": [leaf.0]})).unwrap();
        let xf = |app: &VectorcraftApp| match app.session.active().unwrap().doc.node(leaf).map(|n| &n.kind) {
            Some(NodeKind::SymbolInstance { xf, .. }) => *xf,
            _ => panic!("the Leaf instance"),
        };
        let master = xf(&app);
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        ctx.enable_accesskit();
        let item_disabled = |app: &mut VectorcraftApp| {
            let raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(236.0, 400.0))), ..Default::default() };
            let mut out = ctx.run_ui(raw, |ui| menu(app, ui));
            out.textures_delta.clear();
            let nodes = &out.platform_output.accesskit_update.as_ref().expect("an accessibility tree").nodes;
            nodes
                .iter()
                .find(|(_, n)| n.label().is_some_and(|l| l.trim() == "Reset Transformation"))
                .map(|(_, n)| n.is_disabled())
                .expect("the menu item")
        };
        // At its symbol's scale the menu item is disabled and a click changes nothing.
        assert!(item_disabled(&mut app), "the item is disabled at the symbol's scale");
        let before = app.session.active().unwrap().doc.clone();
        click_menu(&ctx, &mut app, "Reset Transformation");
        assert!(Arc::ptr_eq(&before, &app.session.active().unwrap().doc), "the item is disabled");
        app.run("object.rotate", json!({"angle": 30})).unwrap();
        assert!(!item_disabled(&mut app), "a turned instance enables it");
        click_menu(&ctx, &mut app, "Reset Transformation");
        let close =
            |a: vectorcraft_geom::Affine, b: vectorcraft_geom::Affine| a.as_coeffs().iter().zip(b.as_coeffs()).all(|(x, y)| (x - y).abs() < 1e-9);
        assert!(close(xf(&app), master), "the menu item resets the instance: {:?} vs {master:?}", xf(&app));
        // The Control bar's Reset.
        let bar = |app: &mut VectorcraftApp, events: Vec<egui::Event>| {
            let raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(800.0, 400.0))), events, ..Default::default() };
            let mut out = ctx.run_ui(raw, |ui| {
                ui.horizontal(|ui| control_bar(app, ui, 0.0));
            });
            out.textures_delta.clear();
            out
        };
        let reset_disabled = |out: &egui::FullOutput| {
            let nodes = &out.platform_output.accesskit_update.as_ref().expect("an accessibility tree").nodes;
            nodes.iter().find(|(_, n)| n.label().is_some_and(|l| l.trim() == "Reset")).map(|(_, n)| n.is_disabled()).expect("a Reset button")
        };
        assert!(reset_disabled(&bar(&mut app, vec![])), "Reset is disabled at the symbol's scale");
        app.run("object.scale", json!({"sx": 150, "sy": 50})).unwrap();
        let out = bar(&mut app, vec![]);
        assert!(!reset_disabled(&out));
        let at = clickable(&out).into_iter().find(|(l, _)| l == "Reset").expect("a Reset button").1;
        bar(&mut app, vec![egui::Event::PointerMoved(at), button(at, true), button(at, false)]);
        assert!(close(xf(&app), master), "the Control bar's Reset resets the instance: {:?} vs {master:?}", xf(&app));
    }

    /// Right-clicking a symbol in the Symbols panel selects it (if not already selected),
    /// displays the context menu, and clicking an item dispatches the corresponding command.
    #[test]
    fn right_click_context_menu_in_symbols_panel_runs_actions() {
        use egui_kittest::kittest::{NodeT, Queryable};

        let has_instance = |doc: &Document, name: &str| {
            let mut found = false;
            for l in &doc.layers {
                l.walk(&mut |c| found |= matches!(&c.kind, NodeKind::SymbolInstance { symbol, .. } if symbol == name));
            }
            found
        };

        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        // Create two symbols "A" and "B" (clearing the instances symbol.new leaves)
        app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 30, "height": 20})).unwrap();
        let id_a = app.run("symbol.new", json!({"name": "A"})).unwrap()["id"].as_u64().unwrap();
        app.run("edit.clear", json!({"ids": [id_a]})).unwrap();
        app.run("shape.rectangle", json!({"x": 50, "y": 10, "width": 30, "height": 20})).unwrap();
        let id_b = app.run("symbol.new", json!({"name": "B"})).unwrap()["id"].as_u64().unwrap();
        app.run("edit.clear", json!({"ids": [id_b]})).unwrap();

        // Currently "B" is the current symbol
        assert_eq!(current(&app), Some("B".to_string()));

        let mut h = egui_kittest::Harness::builder().with_size(vec2(260.0, 400.0)).with_accessibility_check(false).build_ui_state(
            |ui, app: &mut VectorcraftApp| {
                show(app, ui);
            },
            app,
        );
        h.run_steps(2);

        // In thumbnail view, get symbol A
        let node_a = h.get_by_label("A");
        let pos_a = node_a.rect().center();

        // Right-click symbol A
        h.event(egui::Event::PointerMoved(pos_a));
        h.event(egui::Event::PointerButton { pos: pos_a, button: egui::PointerButton::Secondary, pressed: true, modifiers: Default::default() });
        h.event(egui::Event::PointerButton { pos: pos_a, button: egui::PointerButton::Secondary, pressed: false, modifiers: Default::default() });
        h.run_steps(2);

        // Right-clicking unselected "A" selected it
        assert_eq!(current(h.state_mut()), Some("A".to_string()));

        // Context menu items are displayed
        let place = h.get_by_label("   Place Symbol Instance");
        let _ = h.get_by_label("   Edit Symbol");
        let _ = h.get_by_label("   Duplicate Symbol");
        let _ = h.get_by_label("   Delete Symbol");
        let break_link = h.get_by_label("   Break Link to Symbol");
        let _ = h.get_by_label("   Select All Instances");

        // Break Link to Symbol is disabled because no instances of A are selected
        assert!(break_link.accesskit_node().is_disabled());

        // 1. Click "Place Symbol Instance"
        place.click();
        h.run_steps(2);

        // An instance of A is placed on the canvas and selected
        let doc = &h.state().session.active().unwrap().doc;
        assert!(has_instance(doc, "A"));

        // 2. Right-click symbol A again now that its instance is selected
        // Place an instance of B as well and select both to verify Break Link is scoped to A
        let inst_b = h.state_mut().run("symbol.place", json!({ "name": "B" })).unwrap()["id"].as_u64().unwrap();
        let inst_a = h.state().session.active().unwrap().doc.layers[0].children().unwrap()[0].id.0;
        h.state_mut().run("select.set", json!({ "ids": [inst_a, inst_b] })).unwrap();
        h.run_steps(2);

        h.event(egui::Event::PointerMoved(pos_a));
        h.event(egui::Event::PointerButton { pos: pos_a, button: egui::PointerButton::Secondary, pressed: true, modifiers: Default::default() });
        h.event(egui::Event::PointerButton { pos: pos_a, button: egui::PointerButton::Secondary, pressed: false, modifiers: Default::default() });
        h.run_steps(2);

        // Now "Break Link to Symbol" is enabled
        let break_link = h.get_by_label("   Break Link to Symbol");
        assert!(!break_link.accesskit_node().is_disabled());
        break_link.click();
        h.run_steps(2);

        // Link broken for A (no longer a SymbolInstance of A), while B remains an instance
        let doc = &h.state().session.active().unwrap().doc;
        assert!(!has_instance(doc, "A"));
        assert!(has_instance(doc, "B"));

        // 3. Right-click A and click "Duplicate Symbol"
        let before_syms = symbols(h.state_mut());
        h.event(egui::Event::PointerMoved(pos_a));
        h.event(egui::Event::PointerButton { pos: pos_a, button: egui::PointerButton::Secondary, pressed: true, modifiers: Default::default() });
        h.event(egui::Event::PointerButton { pos: pos_a, button: egui::PointerButton::Secondary, pressed: false, modifiers: Default::default() });
        h.run_steps(2);

        h.get_by_label("   Duplicate Symbol").click();
        h.run_steps(2);

        let after_syms = symbols(h.state_mut());
        assert_eq!(after_syms.len(), before_syms.len() + 1);
        assert!(before_syms.iter().all(|s| after_syms.contains(s)));
        assert!(after_syms.contains(&"A copy".to_string()));

        // 4. Right-click B and click "Edit Symbol"
        let node_b = h.get_by_label("B");
        let pos_b = node_b.rect().center();
        h.event(egui::Event::PointerMoved(pos_b));
        h.event(egui::Event::PointerButton { pos: pos_b, button: egui::PointerButton::Secondary, pressed: true, modifiers: Default::default() });
        h.event(egui::Event::PointerButton { pos: pos_b, button: egui::PointerButton::Secondary, pressed: false, modifiers: Default::default() });
        h.run_steps(2);

        assert_eq!(current(h.state_mut()), Some("B".to_string()));
        h.get_by_label("   Edit Symbol").click();
        h.run_steps(2);
        assert!(h.state().session.active().unwrap().doc.symbol_edit.is_some());

        // Exit symbol editing mode
        h.state_mut().run("symbol.exitEditing", json!({})).unwrap();
        h.run_steps(2);

        // 5. Right-click B and click "Delete Symbol"
        h.event(egui::Event::PointerMoved(pos_b));
        h.event(egui::Event::PointerButton { pos: pos_b, button: egui::PointerButton::Secondary, pressed: true, modifiers: Default::default() });
        h.event(egui::Event::PointerButton { pos: pos_b, button: egui::PointerButton::Secondary, pressed: false, modifiers: Default::default() });
        h.run_steps(2);

        h.get_by_label("   Delete Symbol").click();
        h.run_steps(2);

        // Dialog opened; cancelling preserves B
        assert!(h.state().ui.dialog.is_some());
        h.state_mut().ui.dialog = None;
        assert!(symbols(h.state_mut()).contains(&"B".to_string()));

        // Right-click B again and confirm deletion
        h.event(egui::Event::PointerMoved(pos_b));
        h.event(egui::Event::PointerButton { pos: pos_b, button: egui::PointerButton::Secondary, pressed: true, modifiers: Default::default() });
        h.event(egui::Event::PointerButton { pos: pos_b, button: egui::PointerButton::Secondary, pressed: false, modifiers: Default::default() });
        h.run_steps(2);
        h.get_by_label("   Delete Symbol").click();
        h.run_steps(2);
        crate::dialogs::confirm(h.state_mut()).unwrap();
        assert!(!symbols(h.state_mut()).contains(&"B".to_string()));

        // Also test list view
        h.state_mut().ui.symbols_view = View::SmallList;
        h.run_steps(2);
        let node_a = h.get_by_label("A");
        let pos_a = node_a.rect().center();
        h.event(egui::Event::PointerMoved(pos_a));
        h.event(egui::Event::PointerButton { pos: pos_a, button: egui::PointerButton::Secondary, pressed: true, modifiers: Default::default() });
        h.event(egui::Event::PointerButton { pos: pos_a, button: egui::PointerButton::Secondary, pressed: false, modifiers: Default::default() });
        h.run_steps(2);

        assert!(h.query_by_label("   Place Symbol Instance").is_some());
        h.get_by_label("   Select All Instances").click();
        h.run_steps(2);
    }

    /// The Control bar's symbol instance controls leave the room `reserve` asks for after them, the
    /// gap after the controls included, when the row has no room for any of them.
    #[test]
    fn control_bar_symbol_controls_leave_the_reserve_when_none_fits() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
        let art = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 30, "height": 20})).unwrap()["id"].clone();
        let instance = app.run("symbol.new", json!({"name": "Mark", "ids": [art]})).unwrap()["id"].clone();
        app.run("select.set", json!({ "ids": [instance] })).unwrap();
        let ctx = egui::Context::default();
        let raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(600.0, 100.0))), ..Default::default() };
        let mut room = None;
        let mut out = ctx.run_ui(raw, |ui| {
            ui.horizontal(|ui| {
                // Two points more than the reserve: too little for Edit Symbol or Replace.
                let reserve = ui.available_width() - 2.0;
                control_bar(&mut app, ui, reserve);
                room = Some((ui.available_width(), reserve));
            });
        });
        out.textures_delta.clear();
        let (left, reserve) = room.expect("the row was drawn");
        assert!(left >= reserve, "{left} pt left after the symbol controls, {reserve} pt reserved");
    }

    /// The current symbol as `symbol.list` answers it, read by running the command.
    fn listed_current(app: &mut VectorcraftApp) -> Option<String> {
        app.run("symbol.list", json!({})).ok().and_then(|v| v["current"].as_str().map(str::to_string))
    }

    fn store_current(app: &mut VectorcraftApp, v: Value) {
        Arc::make_mut(&mut app.session.doc_mut().unwrap().doc).unknown.insert("currentSymbol".into(), v);
    }

    /// The panel's current symbol is the one `symbol.list` answers: with no document, with no
    /// symbols, with several symbols, with an instance of another symbol selected, and with a
    /// stored name no symbol has.
    #[test]
    fn current_is_the_current_symbol_list_answers() {
        fn check(app: &mut VectorcraftApp, want: Option<&str>, case: &str) {
            let read = current(app);
            assert_eq!(read.as_deref(), want, "{case}");
            assert_eq!(read, listed_current(app), "{case}: symbol.list");
        }
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        check(&mut app, None, "no document");
        app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
        check(&mut app, None, "a document without symbols");
        for (name, x) in [("Leaf", 10), ("Dot", 80), ("Star", 150)] {
            app.run("shape.rectangle", json!({"x": x, "y": 10, "width": 30, "height": 30})).unwrap();
            app.run("symbol.new", json!({"name": name})).unwrap();
        }
        check(&mut app, Some("Star"), "several symbols, the newest current");
        app.run("symbol.setCurrent", json!({"name": "Leaf"})).unwrap();
        check(&mut app, Some("Leaf"), "the first symbol made current");
        let dot = instance_of(&app, "Dot").expect("a Dot instance");
        app.run("select.set", json!({"ids": [dot.0]})).unwrap();
        check(&mut app, Some("Leaf"), "an instance of Dot selected");
        app.run("symbol.setCurrent", json!({"name": "Dot"})).unwrap();
        check(&mut app, Some("Dot"), "a symbol that is not first");
        store_current(&mut app, json!("Gone"));
        check(&mut app, Some("Leaf"), "a stored name no symbol has");
        store_current(&mut app, json!(7));
        check(&mut app, Some("Leaf"), "a stored value that is not a name");
    }

    /// Idle frames of the panel and of its open menu run no command (`symbol.list` included), so
    /// they leave `run_count`, which the native menus watch, as it was.
    #[test]
    fn idle_frames_of_the_panel_and_its_menu_run_no_command() {
        let mut app = leaf_and_dot();
        let ctx = egui::Context::default();
        frame(&ctx, &mut app, vec![]);
        let commands = app.run_count;
        frame(&ctx, &mut app, vec![]);
        frame(&ctx, &mut app, vec![]);
        assert_eq!(app.run_count, commands, "the panel");
        let menu_ctx = egui::Context::default();
        menu_frame(&menu_ctx, &mut app, vec![]);
        menu_frame(&menu_ctx, &mut app, vec![]);
        assert_eq!(app.run_count, commands, "the panel menu");
        assert_eq!(current(&app).as_deref(), Some("Dot"));
    }

    /// The rectangle of the panel's button labeled `label`, enabled or not.
    fn button_rect(ctx: &egui::Context, app: &mut VectorcraftApp, label: &str) -> Rect {
        ctx.enable_accesskit();
        let out = frame_held(ctx, app, vec![], egui::Modifiers::NONE);
        let nodes = &out.platform_output.accesskit_update.as_ref().expect("an accessibility tree").nodes;
        let b = nodes.iter().filter(|(_, n)| n.label().is_some_and(|l| l.trim() == label)).find_map(|(_, n)| n.bounds());
        let b = b.unwrap_or_else(|| panic!("a {label} button"));
        Rect::from_min_max(pos2(b.x0 as f32, b.y0 as f32), pos2(b.x1 as f32, b.y1 as f32))
    }

    /// Whether `out` draws the drop outline around `rect`.
    fn outlined(out: &egui::FullOutput, rect: Rect) -> bool {
        out.shapes.iter().any(|c| match &c.shape {
            egui::Shape::Rect(r) => r.rect.center().distance(rect.center()) < 0.5 && (r.stroke.width - 1.5).abs() < 0.01,
            _ => false,
        })
    }

    /// A symbol dragged from the panel, in Thumbnail View or a list view, and dropped on New Symbol
    /// is duplicated as Duplicate Symbol does it: a copy of the dragged symbol, not the current one,
    /// is added under its default name in one undo step, with no dialog and no placed instance, and
    /// the selection and the current symbol stay as they were, with art selected or none (when the
    /// button is disabled for clicks). The button is outlined while the symbol is held over it.
    #[test]
    fn symbol_dropped_on_new_button_duplicates_the_dragged_symbol_in_both_views() {
        for (view, select) in [("thumbnail", false), ("smallList", true)] {
            let mut app = leaf_and_dot();
            app.run("ui.panelView", json!({"panel": "symbols", "view": view})).unwrap();
            if select {
                let leaf = instance_of(&app, "Leaf").expect("a Leaf instance");
                app.run("select.set", json!({"ids": [leaf.0]})).unwrap();
            }
            let selection = app.session.active().unwrap().selection.clone();
            let undo = app.session.active().unwrap().history.undo.len();
            let ctx = egui::Context::default();
            crate::theme::install_fonts(&ctx);
            let new = button_rect(&ctx, &mut app, "New Symbol");
            let from = if select { panel_labels(&ctx, &mut app, &["Leaf"])[0].1 } else { pos2(26.0, 26.0) };
            frame(&ctx, &mut app, vec![egui::Event::PointerMoved(from), button(from, true)]);
            frame(&ctx, &mut app, vec![egui::Event::PointerMoved(from + vec2(40.0, 40.0))]);
            frame(&ctx, &mut app, vec![egui::Event::PointerMoved(from + vec2(80.0, 80.0))]);
            let document = uid(&app).expect("an active document");
            assert_eq!(
                egui::DragAndDrop::payload::<PanelDrag>(&ctx).as_deref(),
                Some(&PanelDrag::Symbol { document, name: "Leaf".into() }),
                "{view}"
            );
            let out = frame_held(&ctx, &mut app, vec![egui::Event::PointerMoved(new.center())], egui::Modifiers::NONE);
            assert!(outlined(&out, new), "{view}: outlined while Leaf is held over it");
            frame(&ctx, &mut app, vec![button(new.center(), false)]);
            assert_eq!(symbols(&mut app), ["Leaf", "Dot", "Leaf copy"], "{view}");
            let d = app.session.active().unwrap();
            assert_eq!(d.history.undo.len(), undo + 1, "{view}: one undo step");
            assert_eq!(d.selection, selection, "{view}: the selection is unchanged");
            let (leaf, copy) = (&d.doc.symbols[0], &d.doc.symbols[2]);
            assert!(!Arc::ptr_eq(&leaf.art, &copy.art) && leaf.dynamic == copy.dynamic, "{view}: an independent copy of Leaf");
            assert_eq!(d.doc.symbol_registration("Leaf copy"), d.doc.symbol_registration("Leaf"), "{view}");
            assert_eq!(dialog_kind(&app), None, "{view}: no Symbol Options");
            assert_eq!(art(&app, "Leaf copy").0, 0, "{view}: no instance placed");
            assert_eq!(current(&app).as_deref(), Some("Dot"), "{view}: Dot stays current");
            app.run("edit.undo", json!({})).unwrap();
            assert_eq!(symbols(&mut app), ["Leaf", "Dot"], "{view}: undo removes only the copy");
        }
    }

    /// New Symbol takes only a symbol of the document's: a library symbol, art, a symbol name the
    /// document does not have and a symbol released elsewhere make no symbol and no undo step, and
    /// the button is not outlined for them.
    #[test]
    fn new_symbol_button_ignores_other_drops_and_releases_outside() {
        let mut app = leaf_and_dot();
        let leaf = instance_of(&app, "Leaf").expect("a Leaf instance");
        let undo = app.session.active().unwrap().history.undo.len();
        let ctx = egui::Context::default();
        // The payloads are set directly, as if dragged from this document's panel.
        let document = uid(&app).expect("an active document");
        let new = button_rect(&ctx, &mut app, "New Symbol");
        let elsewhere = new.center() - vec2(0.0, 60.0);
        let cases = [
            ("a library symbol", PanelDrag::LibrarySymbol { library: "basic-shapes".into(), name: "Leaf".into() }, new.center()),
            ("art", PanelDrag::Art(vec![leaf]), new.center()),
            ("a stale name", PanelDrag::Symbol { document, name: "Gone".into() }, new.center()),
            ("a symbol released elsewhere", PanelDrag::Symbol { document, name: "Leaf".into() }, elsewhere),
        ];
        for (case, drag, at) in cases {
            egui::DragAndDrop::set_payload(&ctx, drag);
            let out = frame_held(&ctx, &mut app, vec![egui::Event::PointerMoved(at)], egui::Modifiers::NONE);
            assert!(!outlined(&out, new), "{case}: not outlined");
            frame(&ctx, &mut app, vec![button(at, false)]);
            assert_eq!(symbols(&mut app), ["Leaf", "Dot"], "{case}");
            assert_eq!(app.session.active().unwrap().history.undo.len(), undo, "{case}: no undo step");
            assert_eq!(dialog_kind(&app), None, "{case}");
            egui::DragAndDrop::clear_payload(&ctx);
        }
    }

    /// A symbol dragged from one document's panel and dropped on New Symbol after another document
    /// with a symbol of the same name became active is duplicated in neither document, and the
    /// button is not outlined for it.
    #[test]
    fn a_symbol_dragged_from_another_document_is_not_duplicated() {
        let mut app = leaf_and_dot();
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let tile = pos2(26.0, 26.0);
        frame(&ctx, &mut app, vec![]);
        frame(&ctx, &mut app, vec![egui::Event::PointerMoved(tile), button(tile, true)]);
        frame(&ctx, &mut app, vec![egui::Event::PointerMoved(tile + vec2(40.0, 40.0))]);
        frame(&ctx, &mut app, vec![egui::Event::PointerMoved(tile + vec2(80.0, 80.0))]);
        let document = uid(&app).expect("an active document");
        assert_eq!(egui::DragAndDrop::payload::<PanelDrag>(&ctx).as_deref(), Some(&PanelDrag::Symbol { document, name: "Leaf".into() }));
        // While Leaf is held, a new document with a Leaf of its own becomes active.
        app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
        app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 30, "height": 30})).unwrap();
        app.run("symbol.new", json!({"name": "Leaf"})).unwrap();
        let undo = app.session.active().unwrap().history.undo.len();
        let new = button_rect(&ctx, &mut app, "New Symbol");
        let out = frame_held(&ctx, &mut app, vec![egui::Event::PointerMoved(new.center())], egui::Modifiers::NONE);
        assert!(!outlined(&out, new), "not outlined");
        frame(&ctx, &mut app, vec![button(new.center(), false)]);
        assert_eq!(symbols(&mut app), ["Leaf"], "no copy in the active document");
        assert_eq!(app.session.active().unwrap().history.undo.len(), undo, "no undo step");
        assert_eq!(dialog_kind(&app), None);
        assert!(app.session.set_active(0));
        assert_eq!(symbols(&mut app), ["Leaf", "Dot"], "no copy in the first document");
    }

    /// Drags the symbol at `from` in the panel onto `to` and releases it there with `modifiers`
    /// held; returns the frame in which it is held over `to`.
    fn drag_onto(ctx: &egui::Context, app: &mut VectorcraftApp, from: Pos2, to: Pos2, modifiers: egui::Modifiers) -> egui::FullOutput {
        // egui finds what the press hits among the widgets of the frame before, so the panel is
        // drawn once first (the frame before may have drawn the panel menu).
        frame(ctx, app, vec![]);
        frame(ctx, app, vec![egui::Event::PointerMoved(from), button(from, true)]);
        frame(ctx, app, vec![egui::Event::PointerMoved(from + vec2(40.0, 40.0))]);
        frame(ctx, app, vec![egui::Event::PointerMoved(from + vec2(80.0, 80.0))]);
        let held = frame_held(ctx, app, vec![egui::Event::PointerMoved(to)], egui::Modifiers::NONE);
        let release = egui::Event::PointerButton { pos: to, button: egui::PointerButton::Primary, pressed: false, modifiers };
        frame_held(ctx, app, vec![release], modifiers);
        held
    }

    /// A symbol dragged from the panel, in Thumbnail View or a list view, and dropped on Delete
    /// Symbol is deleted as a click on the button deletes it, though another symbol is current. The
    /// button is outlined while the symbol is held over it. A symbol with instances asks the in-use
    /// question, with Alt held too, and Cancel changes nothing; an unused one asks "Delete the
    /// selected symbol?", or with Alt held is deleted at once; each deletion is one undo step.
    #[test]
    fn symbol_dropped_on_delete_button_deletes_the_dragged_symbol_in_both_views() {
        for view in ["thumbnail", "smallList"] {
            let mut app = leaf_and_dot();
            app.run("ui.panelView", json!({"panel": "symbols", "view": view})).unwrap();
            let ctx = egui::Context::default();
            crate::theme::install_fonts(&ctx);
            let trash = button_rect(&ctx, &mut app, "Delete Symbol");
            let first = |ctx: &egui::Context, app: &mut VectorcraftApp, name: &str| {
                if view == "thumbnail" { pos2(26.0, 26.0) } else { panel_labels(ctx, app, &[name])[0].1 }
            };
            let before = app.session.active().unwrap().doc.clone();
            let undo = app.session.active().unwrap().history.undo.len();
            // Leaf, which an instance uses, while Dot is current.
            let leaf = first(&ctx, &mut app, "Leaf");
            for alt in [egui::Modifiers::NONE, egui::Modifiers::ALT] {
                let held = drag_onto(&ctx, &mut app, leaf, trash.center(), alt);
                assert!(outlined(&held, trash), "{view}: outlined while Leaf is held over it");
                assert_eq!(dialog_kind(&app), Some(crate::dialogs::symbol_delete::USED), "{view} {alt:?}");
                assert_eq!(app.ui.dialog.as_ref().unwrap().fields["name"], json!("Leaf"), "{view}: the dragged symbol");
                crate::dialogs::cancel(&mut app);
                assert!(Arc::ptr_eq(&before, &app.session.active().unwrap().doc), "{view} {alt:?}: Cancel changes nothing");
            }
            assert_eq!(current(&app).as_deref(), Some("Dot"), "{view}");
            drag_onto(&ctx, &mut app, leaf, trash.center(), egui::Modifiers::NONE);
            app.ui.dialog.as_mut().unwrap().fields.insert("discard".into(), json!(true));
            crate::dialogs::confirm(&mut app).unwrap();
            assert_eq!((symbols(&mut app), art(&app, "Leaf").0), (vec!["Dot".to_string()], 0), "{view}: Delete Instances");
            assert_eq!(app.session.active().unwrap().history.undo.len(), undo + 1, "{view}: one undo step");
            // Dot, which nothing uses, now comes first: Yes deletes it, and so does a drop with Alt.
            let dot = first(&ctx, &mut app, "Dot");
            drag_onto(&ctx, &mut app, dot, trash.center(), egui::Modifiers::NONE);
            assert_eq!(dialog_kind(&app), Some(crate::dialogs::symbol_delete::KIND), "{view}");
            assert_eq!(app.ui.dialog.as_ref().unwrap().fields["name"], json!("Dot"), "{view}");
            crate::dialogs::confirm(&mut app).unwrap();
            assert!(symbols(&mut app).is_empty(), "{view}: Yes deletes Dot");
            assert_eq!(app.session.active().unwrap().history.undo.len(), undo + 2, "{view}: one undo step");
            app.run("edit.undo", json!({})).unwrap();
            drag_onto(&ctx, &mut app, dot, trash.center(), egui::Modifiers::ALT);
            assert_eq!(dialog_kind(&app), None, "{view}: Alt asks nothing for an unused symbol");
            assert!(symbols(&mut app).is_empty(), "{view}: Alt deletes Dot");
            assert_eq!(app.session.active().unwrap().history.undo.len(), undo + 2, "{view}: one undo step");
            app.run("edit.undo", json!({})).unwrap();
            app.run("edit.undo", json!({})).unwrap();
            assert_eq!((symbols(&mut app), art(&app, "Leaf").0), (vec!["Leaf".to_string(), "Dot".to_string()], 1), "{view}");
        }
    }

    /// A symbol Select All Unused picked, dropped on Delete Symbol, deletes every picked symbol
    /// still unused, after one question or with Alt held at once, in one undo step; a symbol the
    /// pick left out is asked about and deleted on its own.
    #[test]
    fn a_picked_symbol_dropped_on_delete_deletes_the_picked_symbols() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
        for (i, name) in ["A", "B", "C"].into_iter().enumerate() {
            app.run("shape.rectangle", json!({"x": 10.0 + 40.0 * i as f64, "y": 10, "width": 30, "height": 20})).unwrap();
            let id = app.run("symbol.new", json!({"name": name})).unwrap()["id"].as_u64().unwrap();
            // A and C lose their instances; B keeps its own.
            if name != "B" {
                app.run("edit.clear", json!({"ids": [id]})).unwrap();
            }
        }
        app.run("ui.panelView", json!({"panel": "symbols", "view": "smallList"})).unwrap();
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let trash = button_rect(&ctx, &mut app, "Delete Symbol");
        let rows: HashMap<String, Pos2> = panel_labels(&ctx, &mut app, &["A", "B", "C"]).into_iter().collect();
        let undo = app.session.active().unwrap().history.undo.len();
        click_menu(&ctx, &mut app, "Select All Unused");
        let held = drag_onto(&ctx, &mut app, rows["A"], trash.center(), egui::Modifiers::NONE);
        assert!(outlined(&held, trash));
        assert_eq!(dialog_kind(&app), Some(crate::dialogs::symbol_delete::KIND));
        assert_eq!(app.ui.dialog.as_ref().unwrap().fields["names"], json!(["A", "C"]), "asked once, about both");
        crate::dialogs::confirm(&mut app).unwrap();
        assert_eq!(symbols(&mut app), ["B"]);
        assert_eq!(app.session.active().unwrap().history.undo.len(), undo + 1, "one undo step");
        app.run("edit.undo", json!({})).unwrap();
        // With Alt held, the picked symbols go without a question.
        click_menu(&ctx, &mut app, "Select All Unused");
        drag_onto(&ctx, &mut app, rows["C"], trash.center(), egui::Modifiers::ALT);
        assert_eq!(dialog_kind(&app), None);
        assert_eq!(symbols(&mut app), ["B"]);
        assert_eq!(app.session.active().unwrap().history.undo.len(), undo + 1, "one undo step");
        app.run("edit.undo", json!({})).unwrap();
        // B, which the pick left out, is asked about on its own.
        click_menu(&ctx, &mut app, "Select All Unused");
        drag_onto(&ctx, &mut app, rows["B"], trash.center(), egui::Modifiers::ALT);
        assert_eq!(dialog_kind(&app), Some(crate::dialogs::symbol_delete::USED));
        let fields = &app.ui.dialog.as_ref().unwrap().fields;
        assert_eq!((fields.get("name"), fields.get("names")), (Some(&json!("B")), None));
        crate::dialogs::confirm(&mut app).unwrap();
        assert_eq!(symbols(&mut app), ["A", "C"]);
        assert_eq!(app.session.active().unwrap().history.undo.len(), undo + 1, "one undo step");
    }

    /// Only the release of the primary button drops a symbol: a secondary button pressed and
    /// released while the symbol is held over New Symbol or Delete Symbol, Alt held, makes no copy,
    /// deletes nothing, asks nothing and adds no undo step, and the later primary release does
    /// nothing either.
    #[test]
    fn a_secondary_release_over_new_or_delete_symbol_drops_nothing() {
        for label in ["New Symbol", "Delete Symbol"] {
            let mut app = leaf_and_dot();
            let ctx = egui::Context::default();
            crate::theme::install_fonts(&ctx);
            let target = button_rect(&ctx, &mut app, label);
            let undo = app.session.active().unwrap().history.undo.len();
            let tile = pos2(26.0, 26.0);
            frame(&ctx, &mut app, vec![]);
            frame(&ctx, &mut app, vec![egui::Event::PointerMoved(tile), button(tile, true)]);
            frame(&ctx, &mut app, vec![egui::Event::PointerMoved(tile + vec2(40.0, 40.0))]);
            frame(&ctx, &mut app, vec![egui::Event::PointerMoved(tile + vec2(80.0, 80.0))]);
            let held = frame_held(&ctx, &mut app, vec![egui::Event::PointerMoved(target.center())], egui::Modifiers::NONE);
            assert!(outlined(&held, target), "{label}: outlined while Leaf is held over it");
            let secondary = |pressed| egui::Event::PointerButton {
                pos: target.center(),
                button: egui::PointerButton::Secondary,
                pressed,
                modifiers: egui::Modifiers::ALT,
            };
            frame_held(&ctx, &mut app, vec![secondary(true)], egui::Modifiers::ALT);
            frame_held(&ctx, &mut app, vec![secondary(false)], egui::Modifiers::ALT);
            assert_eq!(symbols(&mut app), ["Leaf", "Dot"], "{label}: the secondary release");
            assert_eq!(dialog_kind(&app), None, "{label}: the secondary release asks nothing");
            frame_held(&ctx, &mut app, vec![button(target.center(), false)], egui::Modifiers::ALT);
            assert_eq!(symbols(&mut app), ["Leaf", "Dot"], "{label}: the primary release");
            assert_eq!(app.session.active().unwrap().history.undo.len(), undo, "{label}: no undo step");
            assert_eq!(dialog_kind(&app), None, "{label}");
        }
    }

    /// Delete Symbol takes only a symbol of the active document's that it still has: a library
    /// symbol, art, a name the document does not have, a symbol released elsewhere, a symbol deleted
    /// while it is dragged and a symbol dragged from another document with a symbol of the same name
    /// delete nothing, ask nothing and add no undo step, and the button is not outlined for them.
    #[test]
    fn delete_symbol_button_ignores_other_drops_and_releases_outside() {
        let mut app = leaf_and_dot();
        let leaf = instance_of(&app, "Leaf").expect("a Leaf instance");
        let undo = app.session.active().unwrap().history.undo.len();
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        // The payloads are set directly, as if dragged from this document's panel.
        let document = uid(&app).expect("an active document");
        let trash = button_rect(&ctx, &mut app, "Delete Symbol");
        let elsewhere = trash.center() - vec2(0.0, 60.0);
        let cases = [
            ("a library symbol", PanelDrag::LibrarySymbol { library: "basic-shapes".into(), name: "Leaf".into() }, trash.center()),
            ("art", PanelDrag::Art(vec![leaf]), trash.center()),
            ("a stale name", PanelDrag::Symbol { document, name: "Gone".into() }, trash.center()),
            ("a symbol released elsewhere", PanelDrag::Symbol { document, name: "Dot".into() }, elsewhere),
        ];
        for (case, drag, at) in cases {
            egui::DragAndDrop::set_payload(&ctx, drag);
            let out = frame_held(&ctx, &mut app, vec![egui::Event::PointerMoved(at)], egui::Modifiers::NONE);
            assert!(!outlined(&out, trash), "{case}: not outlined");
            frame_held(&ctx, &mut app, vec![button(at, false)], egui::Modifiers::ALT);
            assert_eq!(symbols(&mut app), ["Leaf", "Dot"], "{case}");
            assert_eq!(app.session.active().unwrap().history.undo.len(), undo, "{case}: no undo step");
            assert_eq!(dialog_kind(&app), None, "{case}");
            egui::DragAndDrop::clear_payload(&ctx);
        }
        // Leaf is deleted by command while it is dragged, then released on the button.
        let tile = pos2(26.0, 26.0);
        frame(&ctx, &mut app, vec![egui::Event::PointerMoved(tile), button(tile, true)]);
        frame(&ctx, &mut app, vec![egui::Event::PointerMoved(tile + vec2(40.0, 40.0))]);
        frame(&ctx, &mut app, vec![egui::Event::PointerMoved(tile + vec2(80.0, 80.0))]);
        assert_eq!(egui::DragAndDrop::payload::<PanelDrag>(&ctx).as_deref(), Some(&PanelDrag::Symbol { document, name: "Leaf".into() }));
        app.run("symbol.delete", json!({"name": "Leaf"})).unwrap();
        let undo = app.session.active().unwrap().history.undo.len();
        let out = frame_held(&ctx, &mut app, vec![egui::Event::PointerMoved(trash.center())], egui::Modifiers::NONE);
        assert!(!outlined(&out, trash), "a deleted symbol: not outlined");
        frame_held(&ctx, &mut app, vec![button(trash.center(), false)], egui::Modifiers::ALT);
        assert_eq!((symbols(&mut app), dialog_kind(&app)), (vec!["Dot".to_string()], None), "a deleted symbol");
        assert_eq!(app.session.active().unwrap().history.undo.len(), undo, "a deleted symbol: no undo step");
        // Dot is dragged, and a new document with a Dot of its own becomes active before the release.
        frame(&ctx, &mut app, vec![egui::Event::PointerMoved(tile), button(tile, true)]);
        frame(&ctx, &mut app, vec![egui::Event::PointerMoved(tile + vec2(40.0, 40.0))]);
        frame(&ctx, &mut app, vec![egui::Event::PointerMoved(tile + vec2(80.0, 80.0))]);
        assert_eq!(egui::DragAndDrop::payload::<PanelDrag>(&ctx).as_deref(), Some(&PanelDrag::Symbol { document, name: "Dot".into() }));
        app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
        app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 30, "height": 30})).unwrap();
        app.run("symbol.new", json!({"name": "Dot"})).unwrap();
        app.run("edit.clear", json!({})).unwrap();
        let other = app.session.active().unwrap().history.undo.len();
        let trash = button_rect(&ctx, &mut app, "Delete Symbol");
        let out = frame_held(&ctx, &mut app, vec![egui::Event::PointerMoved(trash.center())], egui::Modifiers::NONE);
        assert!(!outlined(&out, trash), "another document: not outlined");
        frame_held(&ctx, &mut app, vec![button(trash.center(), false)], egui::Modifiers::ALT);
        assert_eq!((symbols(&mut app), dialog_kind(&app)), (vec!["Dot".to_string()], None), "the active document keeps its Dot");
        assert_eq!(app.session.active().unwrap().history.undo.len(), other, "another document: no undo step");
        assert!(app.session.set_active(0));
        assert_eq!(symbols(&mut app), ["Dot"], "the first document keeps its Dot");
    }

    /// The rects `out` outlines as drop targets (1.5 pt strokes).
    fn drop_outlines(out: &egui::FullOutput) -> Vec<Rect> {
        out.shapes
            .iter()
            .filter_map(|c| match &c.shape {
                egui::Shape::Rect(r) if (r.stroke.width - 1.5).abs() < 0.01 => Some(r.rect),
                _ => None,
            })
            .collect()
    }

    /// Where symbol `name`'s tile or row is in the panel: its name in the list views, its tile (the
    /// 44 pt squares, in panel order) in Thumbnail View.
    fn item_at(ctx: &egui::Context, app: &mut VectorcraftApp, name: &str) -> Pos2 {
        if app.ui.symbols_view != View::Thumbnail {
            return panel_labels(ctx, app, &[name]).first().map(|l| l.1).unwrap_or_else(|| panic!("a {name} row"));
        }
        let out = frame_held(ctx, app, vec![], egui::Modifiers::NONE);
        let tiles: Vec<Rect> = out
            .shapes
            .iter()
            .filter_map(|c| match &c.shape {
                egui::Shape::Rect(r) if (r.rect.width() - 44.0).abs() < 0.01 && (r.rect.height() - 44.0).abs() < 0.01 => Some(r.rect),
                _ => None,
            })
            .collect();
        let i = symbols(app).iter().position(|n| n == name).unwrap_or_else(|| panic!("a {name} symbol"));
        tiles.get(i).map(|r| r.center()).unwrap_or_else(|| panic!("a {name} tile in {tiles:?}"))
    }

    /// The art of symbol `name` in the active document.
    fn symbol_art(app: &VectorcraftApp, name: &str) -> Arc<Node> {
        let doc = &app.session.active().unwrap().doc;
        doc.symbols.iter().find(|s| s.name == name).map(|s| s.art.clone()).unwrap_or_else(|| panic!("a {name} symbol"))
    }

    fn undo_len(app: &VectorcraftApp) -> usize {
        app.session.active().unwrap().history.undo.len()
    }

    /// Art dragged off the canvas and released with Alt (Option on macOS) held on a symbol's tile
    /// or row, in each view, redefines that symbol and not the current one: its instances take the
    /// art, the art becomes an instance of it, and no symbol is added, in one undo step. Only that
    /// tile or row is outlined while the art is held over it with Alt.
    #[test]
    fn alt_art_drop_on_a_symbol_redefines_that_symbol_in_one_step() {
        for view in ["thumbnail", "smallList", "largeList"] {
            let mut app = leaf_and_dot();
            app.run("ui.panelView", json!({"panel": "symbols", "view": view})).unwrap();
            let shape = app.run("shape.rectangle", json!({"x": 200, "y": 120, "width": 40, "height": 20})).unwrap()["id"].as_u64().unwrap();
            let (leaf, dot) = (symbol_art(&app, "Leaf"), symbol_art(&app, "Dot"));
            let placed = instance_of(&app, "Leaf").expect("a Leaf instance");
            let undo = undo_len(&app);
            let ctx = egui::Context::default();
            crate::theme::install_fonts(&ctx);
            let (at, other) = (item_at(&ctx, &mut app, "Leaf"), item_at(&ctx, &mut app, "Dot"));
            widgets::drag_art(&ctx, uid(&app).unwrap(), vec![NodeId(shape)]);
            let out = frame_held(&ctx, &mut app, vec![egui::Event::PointerMoved(at)], egui::Modifiers::ALT);
            let outlines = drop_outlines(&out);
            assert_eq!(outlines.len(), 1, "{view}: only the Leaf item is outlined: {outlines:?}");
            assert!(outlines[0].contains(at) && !outlines[0].contains(other), "{view}: {outlines:?}");
            frame_held(&ctx, &mut app, vec![alt_release(at)], egui::Modifiers::ALT);
            assert_eq!(symbols(&mut app), ["Leaf", "Dot"], "{view}: no symbol added");
            assert_eq!(undo_len(&app), undo + 1, "{view}: one undo step");
            // The Leaf instance there was takes the 40 × 20 rectangle's shape, where the old art was square.
            let doc = app.session.active().unwrap().doc.clone();
            let shape = doc.node(placed).and_then(|n| n.geometric_bounds()).map(|b| b.width() / b.height());
            assert!(shape.is_some_and(|r| (r - 2.0).abs() < 1e-6), "{view}: Leaf is the dropped art: {shape:?}");
            assert!(Arc::ptr_eq(&dot, &symbol_art(&app, "Dot")), "{view}: the current symbol Dot is unchanged");
            assert_eq!(art(&app, "Leaf"), (2, 0), "{view}: the art became a Leaf instance");
            app.run("edit.undo", json!({})).unwrap();
            assert!(Arc::ptr_eq(&leaf, &symbol_art(&app, "Leaf")), "{view}: undo restores Leaf");
            assert_eq!(art(&app, "Leaf"), (1, 1), "{view}: undo restores the art");
            app.run("edit.redo", json!({})).unwrap();
            assert_eq!(art(&app, "Leaf"), (2, 0), "{view}: redo");
        }
    }

    /// Alt held as the primary button goes up redefines the symbol, though Alt is let go later in
    /// the same frame.
    #[test]
    fn alt_let_go_after_the_release_in_the_same_frame_still_redefines() {
        let mut app = leaf_and_dot();
        let shape = app.run("shape.rectangle", json!({"x": 200, "y": 120, "width": 40, "height": 20})).unwrap()["id"].as_u64().unwrap();
        let leaf = symbol_art(&app, "Leaf");
        let undo = undo_len(&app);
        let ctx = egui::Context::default();
        let at = item_at(&ctx, &mut app, "Leaf");
        widgets::drag_art(&ctx, uid(&app).unwrap(), vec![NodeId(shape)]);
        frame_held(&ctx, &mut app, vec![egui::Event::PointerMoved(at)], egui::Modifiers::ALT);
        frame_held(&ctx, &mut app, vec![alt_release(at), egui::Event::ModifiersChanged(egui::Modifiers::NONE)], egui::Modifiers::ALT);
        assert_eq!(symbols(&mut app), ["Leaf", "Dot"], "no symbol added");
        assert!(!Arc::ptr_eq(&leaf, &symbol_art(&app, "Leaf")), "Leaf is redefined");
        assert_eq!(undo_len(&app), undo + 1, "one undo step");
    }

    /// Art released on a symbol without Alt makes a new symbol, as a drop elsewhere on the list
    /// does, and leaves the symbol under the pointer as it was; nothing is outlined but the list.
    #[test]
    fn a_plain_art_drop_on_a_symbol_makes_a_new_symbol() {
        let mut app = leaf_and_dot();
        let shape = app.run("shape.rectangle", json!({"x": 200, "y": 120, "width": 40, "height": 20})).unwrap()["id"].as_u64().unwrap();
        let leaf = symbol_art(&app, "Leaf");
        let ctx = egui::Context::default();
        let at = item_at(&ctx, &mut app, "Leaf");
        widgets::drag_art(&ctx, uid(&app).unwrap(), vec![NodeId(shape)]);
        let out = frame_held(&ctx, &mut app, vec![egui::Event::PointerMoved(at)], egui::Modifiers::NONE);
        let outlines = drop_outlines(&out);
        assert!(outlines.len() == 1 && outlines[0].width() > 100.0, "the list is outlined, not the tile: {outlines:?}");
        frame(&ctx, &mut app, vec![button(at, false)]);
        assert_eq!(symbols(&mut app).len(), 3, "a new symbol");
        assert!(Arc::ptr_eq(&leaf, &symbol_art(&app, "Leaf")), "Leaf is unchanged");
    }

    /// An Alt drop that cannot redefine the symbol under the pointer changes nothing, makes no new
    /// symbol and adds no undo step: art of another document, objects the document no longer has,
    /// no objects, and a release away from the panel are not outlined, and art holding an instance
    /// of the symbol is refused when it is released.
    #[test]
    fn an_alt_art_drop_that_cannot_redefine_changes_nothing() {
        let mut app = leaf_and_dot();
        let shape = app.run("shape.rectangle", json!({"x": 200, "y": 120, "width": 40, "height": 20})).unwrap()["id"].as_u64().unwrap();
        let instance = instance_of(&app, "Leaf").expect("a Leaf instance");
        let leaf = symbol_art(&app, "Leaf");
        let undo = undo_len(&app);
        let ctx = egui::Context::default();
        let at = item_at(&ctx, &mut app, "Leaf");
        let doc = uid(&app).unwrap();
        let away = pos2(230.0, 395.0);
        let cases = [
            ("another document's art", doc + 1, vec![NodeId(shape)], at, false),
            ("objects the document lacks", doc, vec![NodeId(u64::MAX)], at, false),
            ("no objects", doc, vec![], at, false),
            ("a release away from the panel", doc, vec![NodeId(shape)], away, false),
            ("a Leaf instance", doc, vec![instance], at, true),
        ];
        for (case, from, ids, to, outline) in cases {
            widgets::drag_art(&ctx, from, ids);
            let out = frame_held(&ctx, &mut app, vec![egui::Event::PointerMoved(to)], egui::Modifiers::ALT);
            assert_eq!(drop_outlines(&out).len(), usize::from(outline), "{case}: {:?}", drop_outlines(&out));
            frame_held(&ctx, &mut app, vec![alt_release(to)], egui::Modifiers::ALT);
            egui::DragAndDrop::clear_payload(&ctx);
            assert_eq!(symbols(&mut app), ["Leaf", "Dot"], "{case}: no symbol added");
            assert!(Arc::ptr_eq(&leaf, &symbol_art(&app, "Leaf")), "{case}: Leaf is unchanged");
            assert_eq!(undo_len(&app), undo, "{case}: no undo step");
        }
    }

    /// Art held over a symbol, with Alt or without, and released with the secondary button is not
    /// dropped: no symbol is redefined or added and no undo step is added.
    #[test]
    fn a_secondary_button_release_drops_no_art() {
        let mut app = leaf_and_dot();
        let shape = app.run("shape.rectangle", json!({"x": 200, "y": 120, "width": 40, "height": 20})).unwrap()["id"].as_u64().unwrap();
        let leaf = symbol_art(&app, "Leaf");
        let undo = undo_len(&app);
        let ctx = egui::Context::default();
        let at = item_at(&ctx, &mut app, "Leaf");
        let secondary = egui::Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed: false, modifiers: Default::default() };
        for (case, modifiers) in [("with Alt", egui::Modifiers::ALT), ("without Alt", egui::Modifiers::NONE)] {
            widgets::drag_art(&ctx, uid(&app).unwrap(), vec![NodeId(shape)]);
            let out = frame_held(&ctx, &mut app, vec![egui::Event::PointerMoved(at)], modifiers);
            assert_eq!(drop_outlines(&out).len(), 1, "{case}: the art is held over a target");
            frame_held(&ctx, &mut app, vec![secondary.clone()], modifiers);
            egui::DragAndDrop::clear_payload(&ctx);
            assert_eq!(symbols(&mut app), ["Leaf", "Dot"], "{case}: no symbol added");
            assert!(Arc::ptr_eq(&leaf, &symbol_art(&app, "Leaf")), "{case}: Leaf is unchanged");
            assert_eq!(undo_len(&app), undo, "{case}: no undo step");
        }
    }

    /// CPU time of this thread in nanoseconds where the system reports it (Linux), else `None`.
    fn thread_cpu_ns() -> Option<u128> {
        std::fs::read_to_string("/proc/thread-self/schedstat").ok()?.split_whitespace().next()?.parse().ok()
    }

    /// Time reading the current symbol of a document with 200 symbols and 50,000 objects (5,000 of
    /// them instances): by running `symbol.list` and by [`current`]. Per run,
    /// `symbol.list` reads 20 times and `current` 200,000 times (so the thread's CPU clock, which
    /// advances in steps of a few milliseconds, sees it); the two sides alternate, and the median
    /// time per read of 7 runs is printed.
    /// Run with `cargo test --release -p vectorcraft-ui-egui --lib current_symbol_read_timing -- --ignored --nocapture`.
    #[test]
    #[ignore = "manual timing probe; performance depends on machine load"]
    fn current_symbol_read_timing() {
        const SYMBOLS: usize = 200;
        const OBJECTS: usize = 50_000;
        const INSTANCES: usize = 5_000;
        const LIST_READS: u32 = 20;
        const READS: u32 = 200_000;
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 2000, "height": 2000})).unwrap();
        for i in 0..SYMBOLS {
            app.run("shape.rectangle", json!({"x": i % 40 * 40, "y": i / 40 * 40, "width": 30, "height": 30})).unwrap();
            app.run("symbol.new", json!({"name": format!("S{i}")})).unwrap();
        }
        let rect = app.run("shape.rectangle", json!({"x": 0, "y": 400, "width": 10, "height": 10})).unwrap()["id"].as_u64().unwrap();
        let rect = app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(rect)).cloned().unwrap();
        app.session
            .edit("Fill", |d, _| {
                let layer = d.layers[0].id;
                for i in 0..OBJECTS - SYMBOLS - 1 {
                    let id = d.alloc_id();
                    let node = if i < INSTANCES - SYMBOLS {
                        let at = vectorcraft_geom::Vec2::new((i % 100) as f64 * 20.0, 600.0 + (i / 100) as f64 * 20.0);
                        Node::new(id, NodeKind::SymbolInstance { symbol: format!("S{}", i % SYMBOLS), xf: vectorcraft_geom::Affine::translate(at) })
                    } else {
                        let mut n = rect.clone();
                        n.id = id;
                        n
                    };
                    d.insert(Some(layer), usize::MAX, node)?;
                }
                Ok(())
            })
            .unwrap();
        app.run("symbol.setCurrent", json!({"name": "S7"})).unwrap();
        let mut nodes = 0;
        app.session.active().unwrap().doc.walk(|_| nodes += 1);
        assert_eq!(listed_current(&mut app), current(&app));
        let mut time = |reads: u32, f: &mut dyn FnMut(&mut VectorcraftApp) -> Option<String>| {
            let (cpu, wall) = (thread_cpu_ns(), std::time::Instant::now());
            for _ in 0..reads {
                assert_eq!(f(&mut app).as_deref(), Some("S7"));
            }
            let ns = thread_cpu_ns().zip(cpu).map_or_else(|| wall.elapsed().as_nanos(), |(b, a)| b - a);
            ns as f64 / f64::from(reads) / 1000.0
        };
        let (mut base, mut new) = (vec![], vec![]);
        for _ in 0..7 {
            base.push(time(LIST_READS, &mut listed_current));
            new.push(time(READS, &mut |app| current(app)));
        }
        let median = |v: &mut Vec<f64>| {
            v.sort_by(f64::total_cmp);
            v[v.len() / 2]
        };
        let clock = if thread_cpu_ns().is_some() { "thread CPU time" } else { "wall-clock time" };
        eprintln!("{SYMBOLS} symbols, {nodes} nodes; {clock} per read, median of 7 runs");
        eprintln!("symbol.list ({LIST_READS} reads a run): {:>12.1} us  {base:.1?}", median(&mut base));
        eprintln!("current ({READS} reads a run):  {:>12.4} us  {new:.4?}", median(&mut new));
    }
}
