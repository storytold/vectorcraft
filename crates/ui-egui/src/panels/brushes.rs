//! Brushes panel: the document's brush library with rendered stroke previews. Clicking a brush
//! applies it to the selected paths (and makes it the Paintbrush's current brush); double-clicking
//! one opens its Brush Options. Shift-click selects a range of brushes, Cmd/Ctrl-click adds or
//! removes one, and the panel menu's Select All Unused selects the brushes no stroke paints with,
//! for Delete Brush and Duplicate Brush. Brush libraries open in library panels
//! ([`BrushLibraries`]).

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use egui::{Sense, Ui, vec2};
use serde_json::{Value, json};
use vectorcraft_brush::Brush;
use vectorcraft_doc::{Appearance, Document, Node, PressureProfile, color::Color, color::Paint};
use vectorcraft_engine::DocState;
use vectorcraft_engine::cmd::brushlib::{self, BrushLibrary};
use vectorcraft_engine::cmd::clipboard::{ArtResources, art_resources};
use vectorcraft_engine::cmd::swatchlib::LibraryInfo;

use super::library_panel::{self, LibraryKind, LibraryRef, Row};
use super::memo::Memo;
use super::{first_selected, pstate, set_pstate};
use crate::VectorcraftApp;
use crate::dialogs::brush_options;
use crate::menus::Item;
use crate::theme::Tokens;
use crate::widgets::{self, PanelDrag, menu_item};

const KINDS: [(&str, &str); 5] =
    [("calligraphic", "Calligraphic"), ("scatter", "Scatter"), ("art", "Art"), ("bristle", "Bristle"), ("pattern", "Pattern")];

/// (name, type) of every brush in the active document's library, and the current brush.
pub fn brushes(app: &mut VectorcraftApp) -> (Vec<(String, String)>, Option<String>) {
    let Some(st) = app.session.active() else { return (vec![], None) };
    let list = document_brushes(st).iter().map(|b| (b.name.clone(), b.ty.to_string())).collect();
    (list, vectorcraft_brush::current(&st.doc))
}

/// A brush as the panels show it: its definition (as `brush.get` gives it, which a drag carries)
/// and the key its stroke preview is cached by ([`preview_key`]), worked out once per document
/// version or library ([`document_brushes`], [`BrushLibraries`]).
pub(crate) struct Shown {
    pub(crate) name: String,
    pub(crate) ty: &'static str,
    pub(crate) def: Value,
    key: u64,
    /// Keeps the art and images `key` counts by address from being reused.
    _deps: ArtResources,
}

impl Shown {
    fn of(doc: &Document, b: &Brush) -> Self {
        let def = serde_json::to_value(b).unwrap_or_default();
        let (key, deps) = preview_key(doc, &def, b.art());
        Self { name: b.name.clone(), ty: b.kind.type_id(), def, key, _deps: deps }
    }
}

/// The brushes of the document version `st` shows, each worked out once while that version is
/// active: a frame of the Brushes panel serialises and hashes none.
pub(crate) fn document_brushes(st: &DocState) -> Rc<Vec<Shown>> {
    thread_local! {
        static MEMO: RefCell<Memo<Document, Vec<Shown>>> = RefCell::new(Memo::default());
    }
    MEMO.with(|m| m.borrow_mut().get(&st.doc, st.revision, || vectorcraft_brush::library(&st.doc).iter().map(|b| Shown::of(&st.doc, b)).collect()))
}

/// The cache key of the stroke preview of brush definition `def` in `doc` (`art`: its art): the
/// definition and every resource of `doc` the art uses ([`art_resources`]: patterns with their
/// options, symbols, swatches and colour groups, gradients, images, styles…), so the preview
/// renders again when one of those changes and stays cached when anything else does. With what
/// keeps the key's addresses from being reused.
fn preview_key(doc: &Document, def: &Value, art: Vec<&Node>) -> (u64, ArtResources) {
    use std::hash::{Hash, Hasher};
    #[cfg(test)]
    counters::KEYED.with(|k| k.set(k.get() + 1));
    let deps = art_resources(doc, art);
    let mut h = std::collections::hash_map::DefaultHasher::new();
    def.to_string().hash(&mut h);
    deps.key.hash(&mut h);
    (h.finish(), deps)
}

/// A stroke preview for brush definition `def`, rendered at `size` with the resources of `doc`
/// ([`preview_doc`]) and cached by `key` ([`preview_key`]).
fn preview(ui: &Ui, doc: &Document, def: &Value, key: u64, size: egui::Vec2) -> Option<egui::TextureHandle> {
    widgets::doc_preview(ui, &format!("brush:{key:016x}"), size, |w, h| {
        #[cfg(test)]
        tests::BUILDS.with(|b| b.set(b.get() + 1));
        preview_doc(doc, def, w, h)
    })
}

/// What a stroke preview of brush definition `def`, `w` × `h` pt, renders: a copy of `resources`
/// (a document whose patterns, symbols, swatches and images the brush art may use) without its art,
/// and a stroke painted with the brush across it. The stroke goes from light to heavy pen pressure
/// and back, so pressure-sensitive brushes show how they vary.
fn preview_doc(resources: &Document, def: &Value, w: f64, h: f64) -> Option<Document> {
    let mut doc = resources.clone();
    let layer = doc.alloc_id();
    doc.layers = vec![Arc::new(Node::layer(layer, "Preview", vectorcraft_doc::LayerColor::Preset(0)))];
    doc.unknown.insert(vectorcraft_brush::DOC_KEY.into(), json!([def]));
    let name = def["name"].as_str()?.to_string();
    let mut ap = Appearance::basic(Paint::None, Paint::solid(Color::BLACK), 1.0);
    let st = ap.stroke_mut()?;
    st.brush = Some(name);
    st.pressure = Some(PressureProfile::sample());
    // A gentle S-curve across the swatch (square tiles keep a quarter of it either side).
    let mut bp = vectorcraft_geom::BezPath::new();
    let margin = (h * 0.5).min(w * 0.25);
    let (x0, x1) = (margin, w - margin);
    bp.move_to((x0, h * 0.5));
    bp.curve_to((x0 + (x1 - x0) * 0.35, h * 0.1), (x0 + (x1 - x0) * 0.65, h * 0.9), (x1, h * 0.5));
    let id = doc.alloc_id();
    doc.insert(Some(layer), 0, Node::path(id, vectorcraft_geom::PathData::from_bezpath(&bp), ap)).ok()?;
    Some(doc)
}

/// The stroke preview of brush definition `def` (with the resources of `doc`), cached by `key`
/// ([`preview_key`]), on white in `r`: the panels' brushes, Brush Options and the chip a dragged
/// brush shows at the pointer.
pub(crate) fn chip(ui: &Ui, r: egui::Rect, doc: &Document, def: &Value, key: u64) {
    ui.painter().rect_filled(r, 0.0, egui::Color32::WHITE);
    if let Some(tex) = preview(ui, doc, def, key, r.size()) {
        #[cfg(test)]
        tests::DRAWN.with(|d| d.set(d.get() + 1));
        ui.painter().image(tex.id(), r, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
    }
}

/// [`chip`] of a brush the fields of Brush Options describe (`def`), with the resources of the
/// document version `st` (none without a document): its key is worked out again only when the
/// definition or the document changes, not every frame.
pub(crate) fn definition_chip(ui: &Ui, r: egui::Rect, st: Option<&DocState>, def: &Value) {
    type Last = (std::sync::Weak<Document>, u64, Value, u64, ArtResources);
    thread_local! {
        static LAST: RefCell<Option<Last>> = const { RefCell::new(None) };
        static BLANK: Arc<Document> = Arc::new(Document::new(1.0, 1.0));
    }
    let doc = st.map_or_else(|| BLANK.with(Arc::clone), |st| st.doc.clone());
    let stamp = st.map_or(0, |st| st.revision);
    let known = |l: &Last| l.0.as_ptr() == Arc::as_ptr(&doc) && l.1 == stamp && l.2 == *def;
    let key = LAST.with(|l| l.borrow().as_ref().filter(|l| known(l)).map(|l| l.3)).unwrap_or_else(|| {
        let brush = Brush::from_json(def.clone()).ok();
        let (key, deps) = preview_key(&doc, def, brush.as_ref().map(Brush::art).unwrap_or_default());
        LAST.with(|l| *l.borrow_mut() = Some((Arc::downgrade(&doc), stamp, def.clone(), key, deps)));
        key
    });
    chip(ui, r, &doc, def, key);
}

fn selected_brush(app: &VectorcraftApp) -> Option<String> {
    first_selected(app).and_then(|n| n.appearance.stroke().and_then(|s| s.brush.clone()))
}

/// What the panel's last click was made with: the document, its selected objects and the current
/// brush.
type ClickKey = (u64, Vec<u64>, Option<String>);

fn click_key(app: &VectorcraftApp, current: Option<&str>) -> Option<ClickKey> {
    app.session.active().map(|st| (st.uid, st.selection.objects.iter().map(|id| id.0).collect(), current.map(str::to_string)))
}

/// Whether the panel's last click was made with the document, objects and current brush active
/// now. Panel state `br-selected` holds the multi-selection and the key of that click.
fn same_click(app: &VectorcraftApp, ctx: &egui::Context, current: Option<&str>) -> bool {
    let (_, key): (Option<Vec<String>>, Option<ClickKey>) = pstate(ctx, "br-selected");
    key.is_some() && key == click_key(app, current)
}

/// The brushes that Shift-clicks, Cmd/Ctrl-clicks and Select All Unused selected, of those in
/// `list`. It is `None` after a plain click, once none of its brushes is left, and once another
/// document, other objects or another current brush is active, which also clears it and the key
/// of the last click; `Some` of an empty list after Cmd/Ctrl-clicks deselected every brush or
/// Select All Unused found none.
fn multi(app: &VectorcraftApp, ctx: &egui::Context, list: &[(String, String)], current: Option<&str>) -> Option<Vec<String>> {
    if !same_click(app, ctx, current) {
        set_pstate(ctx, "br-selected", (None::<Vec<String>>, None::<ClickKey>));
        return None;
    }
    let (names, _): (Option<Vec<String>>, Option<ClickKey>) = pstate(ctx, "br-selected");
    let names = names?;
    if names.is_empty() {
        return Some(names);
    }
    let kept: Vec<String> = names.into_iter().filter(|n| list.iter().any(|b| &b.0 == n)).collect();
    (!kept.is_empty()).then_some(kept)
}

/// The brushes Delete Brush and Duplicate Brush act on: the multi-selection, else the selected
/// path's brush, else the current one.
fn targets(app: &VectorcraftApp, ctx: &egui::Context, list: &[(String, String)], current: Option<&str>) -> Vec<String> {
    multi(app, ctx, list, current).unwrap_or_else(|| selected_brush(app).or(current.map(str::to_string)).into_iter().collect())
}

/// Runs `brush.delete` or `brush.duplicate` on `names`.
fn run_on(app: &mut VectorcraftApp, id: &str, names: &[String]) {
    let p = match names {
        [name] => json!({ "name": name }),
        _ => json!({ "names": names }),
    };
    app.run(id, p).ok();
}

/// The names in `list` in the order the panel shows them: by type in [`KINDS`] order, without the
/// hidden types.
fn shown<'a>(list: &'a [(String, String)], hidden: &[String]) -> Vec<&'a str> {
    let kinds = KINDS.iter().filter(|(ty, _)| !hidden.iter().any(|h| h == ty));
    kinds.flat_map(|(ty, _)| list.iter().filter(move |b| b.1 == *ty).map(|b| b.0.as_str())).collect()
}

/// A brush dragged over the tile or row `resp` of brush `name`, the `i`th brush of type `ty`
/// (`list`: rows stack downwards), lands before or after it by the pointer's half; a bar marks the
/// slot when the dragged brush is of type `ty` too (`types`: the (name, type) of every brush).
/// Released there, it sets `moved` to the `brush.move` params, which refuse a brush of another type.
fn reorder_drop(
    ui: &Ui,
    resp: &egui::Response,
    types: &[(String, String)],
    (name, ty, i): (&str, &str, usize),
    list: bool,
    moved: &mut Option<Value>,
) {
    let Some(drag) = resp.dnd_hover_payload::<PanelDrag>() else { return };
    let PanelDrag::Brush { name: moving, .. } = &*drag else { return };
    if moving == name {
        // Dropped on itself: it stays where it is.
        resp.dnd_release_payload::<PanelDrag>();
        return;
    }
    let r = resp.rect;
    let at = ui.input(|i| i.pointer.interact_pos()).unwrap_or(r.center());
    let after = if list { at.y > r.center().y } else { at.x > r.center().x };
    if types.iter().any(|b| &b.0 == moving && b.1 == ty) {
        let accent = egui::Stroke::new(2.0, Tokens::get(ui.ctx()).accent);
        if list {
            let y = if after { r.bottom() } else { r.top() };
            ui.painter().line_segment([egui::pos2(r.left(), y), egui::pos2(r.right(), y)], accent);
        } else {
            let x = if after { r.right() + 1.5 } else { r.left() - 1.5 };
            ui.painter().line_segment([egui::pos2(x, r.top()), egui::pos2(x, r.bottom())], accent);
        }
    }
    if resp.dnd_release_payload::<PanelDrag>().is_some() {
        *moved = Some(json!({ "name": moving, "to": i + usize::from(after), "type": ty }));
    }
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let (list, current) = brushes(app);
    let sel_brush = selected_brush(app);
    let mut multi = multi(app, ui.ctx(), &list, current.as_deref());
    let list_view: bool = pstate(ui.ctx(), "br-list");
    let hidden: Vec<String> = pstate(ui.ctx(), "br-hidden");
    // Previews show brush art with the document's patterns, symbols and other resources; the
    // definitions and preview keys are the document version's, worked out once.
    let doc = app.session.active().map(|d| d.doc.clone());
    let memo = app.session.active().map(document_brushes).unwrap_or_default();
    let defs: HashMap<&str, &Shown> = memo.iter().map(|b| (b.name.as_str(), b)).collect();
    let mut clicked: Option<(String, egui::Modifiers)> = None;
    let mut options: Option<String> = None;
    let mut moved: Option<Value> = None;
    let list_rect = widgets::list_box(ui, |ui| {
        ui.set_min_height(110.0);
        ui.set_width(ui.available_width());
        if list.is_empty() {
            super::empty_state(ui, "paintbrush", tl!("No brushes"), tl!("Select art and use New Brush to make one."));
            return ui.min_rect();
        }
        for (ty, _) in KINDS {
            if hidden.iter().any(|h| h == ty) {
                continue;
            }
            let group: Vec<&(String, String)> = list.iter().filter(|b| b.1 == ty).collect();
            if group.is_empty() {
                continue;
            }
            let row = |ui: &mut Ui, name: &str, size: egui::Vec2, label: bool| -> egui::Response {
                let (r, resp) = ui.allocate_exact_size(size, Sense::click_and_drag());
                // Dragged onto a path, the brush is applied to it.
                widgets::drag_source(ui, &resp, || PanelDrag::Brush {
                    name: name.to_string(),
                    def: defs.get(name).map(|b| b.def.clone()).unwrap_or_default(),
                });
                let on = match &multi {
                    Some(multi) => multi.iter().any(|m| m == name),
                    None => sel_brush.as_deref() == Some(name) || (sel_brush.is_none() && current.as_deref() == Some(name)),
                };
                if on {
                    ui.painter().rect_filled(r, 0.0, t.row_selected);
                } else if resp.hovered() {
                    ui.painter().rect_filled(r, 0.0, t.hover);
                }
                let pw = if label { 72.0 } else { size.x - 4.0 };
                let pr = egui::Rect::from_min_size(r.left_top() + vec2(2.0, 2.0), vec2(pw, size.y - 4.0));
                if let (Some(b), Some(doc)) = (defs.get(name), doc.as_deref()) {
                    chip(ui, pr, doc, &b.def, b.key);
                } else {
                    ui.painter().rect_filled(pr, 0.0, egui::Color32::WHITE);
                }
                if label {
                    ui.painter().text(
                        r.left_center() + vec2(pw + 10.0, 0.0),
                        egui::Align2::LEFT_CENTER,
                        name,
                        egui::FontId::proportional(12.5),
                        t.text,
                    );
                }
                resp.on_hover_text(name)
            };
            // A click applies or selects the brush; the second click of a double-click opens its
            // options.
            let mut pick = |resp: egui::Response, name: &String| {
                if resp.double_clicked() {
                    options = Some(name.clone());
                } else if resp.clicked() {
                    clicked = Some((name.clone(), resp.ctx.input(|i| i.modifiers)));
                }
            };
            if list_view {
                for (i, (name, _)) in group.into_iter().enumerate() {
                    let resp = row(ui, name, vec2(ui.available_width(), 26.0), true);
                    reorder_drop(ui, &resp, &list, (name, ty, i), true, &mut moved);
                    pick(resp, name);
                }
            } else {
                ui.horizontal_wrapped(|ui| {
                    for (i, (name, _)) in group.into_iter().enumerate() {
                        let resp = row(ui, name, vec2(64.0, 28.0), false);
                        reorder_drop(ui, &resp, &list, (name, ty, i), false, &mut moved);
                        pick(resp, name);
                    }
                });
            }
            ui.separator();
        }
        ui.min_rect()
    });
    // Art dragged off the canvas and dropped on the list opens New Brush for that art, as the New
    // Brush button opens it for the selection; a brush library's brush dropped there is added.
    let zone = ui.interact(list_rect, ui.scope_id().with("brushes-drop"), Sense::hover());
    if let Some(ids) = widgets::art_drop(ui, &zone)
        && let Err(e) = app.run("ui.newBrush", json!({ "ids": ids }))
    {
        app.status(e);
    }
    if let Some((library, name)) = library_brush_drop(ui, &zone) {
        add(app, json!({"library": library, "name": name}));
    }
    // A brush dragged within the list moves among the brushes of its type.
    if let Some(p) = moved
        && let Err(e) = app.run("brush.move", p)
    {
        app.status(e);
    }
    let highlighted = sel_brush.clone().or(current.clone());
    match clicked {
        // Shift and Cmd/Ctrl only select, as in the Swatches panel: they start from the
        // multi-selection, else from the highlighted brush.
        Some((name, m)) if m.shift || m.command => {
            let order = shown(&list, &hidden);
            // A range starts from the last brush clicked without Shift while the same document,
            // art and current brush are active, else from the highlighted brush.
            let anchor: String = pstate(ui.ctx(), "br-anchor");
            if !same_click(app, ui.ctx(), current.as_deref()) || !order.contains(&anchor.as_str()) {
                set_pstate(ui.ctx(), "br-anchor", highlighted.clone().unwrap_or_default());
            }
            let from = multi.unwrap_or_else(|| highlighted.iter().cloned().collect());
            let sel = super::swatches::click_selection(ui, "br-anchor", from, &order, &name, m);
            set_pstate(ui.ctx(), "br-selected", (Some(sel.clone()), click_key(app, current.as_deref())));
            multi = Some(sel);
        }
        Some((name, _)) => {
            set_pstate(ui.ctx(), "br-anchor", name.clone());
            multi = None;
            let has_sel = app.session.active().is_some_and(|d| !d.selection.is_empty());
            if has_sel {
                app.run("brush.apply", json!({"name": name})).ok();
            } else {
                app.run("brush.setCurrent", json!({"name": name})).ok();
            }
            let now = brushes(app).1;
            set_pstate(ui.ctx(), "br-selected", (None::<Vec<String>>, click_key(app, now.as_deref())));
        }
        None => {}
    }
    if let Some(name) = options
        && let Err(e) = app.run("ui.brushOptions", json!({ "name": name }))
    {
        app.status(e);
    }
    let has_brush = sel_brush.is_some();
    let targets = multi.unwrap_or_else(|| highlighted.into_iter().collect());
    let has_doc = app.session.active().is_some();
    widgets::bottom_bar(ui, |ui| {
        let open = library_panel::any_open::<BrushLibraries>(&app.ui);
        let lr = widgets::icon_button(ui, "library", BrushLibraries::MENU, open, 24.0);
        egui::Popup::menu(&lr).show(|ui| {
            ui.set_min_width(200.0);
            library_panel::library_menu::<BrushLibraries>(app, ui);
        });
        if widgets::icon_button_enabled(ui, "dc-remove-brush", tl!("Remove Brush Stroke"), false, has_brush, 24.0).clicked() {
            app.run("brush.remove", json!({})).ok();
        }
        if widgets::icon_button_enabled(ui, "dc-options", tl!("Expand Brush Strokes"), false, has_brush, 24.0).clicked() {
            app.run("object.expandBrush", json!({})).ok();
        }
        ui.add_space((ui.available_width() - 2.0 * 28.0).max(0.0));
        // New Brush asks for the type, then opens that type's Brush Options.
        if widgets::icon_button_enabled(ui, "dc-new-item", tl!("New Brush"), false, has_doc, 24.0).clicked()
            && let Err(e) = app.run("ui.newBrush", json!({}))
        {
            app.status(e);
        }
        if widgets::icon_button_enabled(ui, "trash-2", tl!("Delete Brush"), false, !targets.is_empty(), 24.0).clicked() {
            run_on(app, "brush.delete", &targets);
        }
    });
}

pub fn menu(app: &mut VectorcraftApp, ui: &mut Ui) {
    let sel = selected_brush(app);
    let (list, current) = brushes(app);
    let targets = targets(app, ui.ctx(), &list, current.as_deref());
    let has_sel = app.session.active().is_some_and(|d| !d.selection.is_empty());
    let has_brushes = !list.is_empty();
    if menu_item(ui, tl!("New Brush…"), app.session.active().is_some(), false)
        && let Err(e) = app.run("ui.newBrush", json!({}))
    {
        app.status(e);
    }
    // The types' own items make a brush at once, without its Brush Options.
    for (label, ty) in [(tl!("New Calligraphic Brush"), "calligraphic"), (tl!("New Bristle Brush"), "bristle")] {
        if menu_item(ui, label, true, false) {
            app.run("brush.new", json!({"type": ty})).ok();
        }
    }
    for (label, ty) in [
        (tl!("New Art Brush from Selection"), "art"),
        (tl!("New Scatter Brush from Selection"), "scatter"),
        (tl!("New Pattern Brush from Selection"), "pattern"),
    ] {
        if menu_item(ui, label, has_sel, false) {
            app.run("brush.new", json!({"type": ty})).ok();
        }
    }
    if menu_item(ui, tl!("Duplicate Brush"), !targets.is_empty(), false) {
        run_on(app, "brush.duplicate", &targets);
    }
    if menu_item(ui, tl!("Delete Brush"), !targets.is_empty(), false) {
        run_on(app, "brush.delete", &targets);
    }
    if menu_item(ui, tl!("Remove Brush Stroke"), sel.is_some(), false) {
        app.run("brush.remove", json!({})).ok();
    }
    if menu_item(ui, tl!("Expand Brush Strokes"), sel.is_some(), false) {
        app.run("object.expandBrush", json!({})).ok();
    }
    // On the selected path's brush, else the current one.
    if menu_item(ui, tl!("Brush Options…"), brush_options::available(app), false)
        && let Err(e) = app.run("ui.brushOptions", json!({}))
    {
        app.status(e);
    }
    ui.separator();
    // Selects the brushes no stroke paints with, for Delete Brush and Duplicate Brush.
    if menu_item(ui, tl!("Select All Unused"), app.session.active().is_some(), false)
        && let Ok(r) = app.run("brush.unused", json!({}))
    {
        let names: Vec<String> = serde_json::from_value(r["names"].clone()).unwrap_or_default();
        set_pstate(ui.ctx(), "br-selected", (Some(names), click_key(app, current.as_deref())));
    }
    ui.separator();
    let mut hidden: Vec<String> = pstate(ui.ctx(), "br-hidden");
    for (ty, label) in KINDS {
        let shown = !hidden.iter().any(|h| h == ty);
        if menu_item(ui, &crate::i18n::fmt(tl!("Show {kind} Brushes"), &[("kind", tl!(label))]), true, shown) {
            if shown {
                hidden.push(ty.to_string());
            } else {
                hidden.retain(|h| h != ty);
            }
            set_pstate(ui.ctx(), "br-hidden", hidden.clone());
        }
    }
    ui.separator();
    let list: bool = pstate(ui.ctx(), "br-list");
    if menu_item(ui, tl!("Thumbnail View"), true, !list) {
        set_pstate(ui.ctx(), "br-list", false);
    }
    if menu_item(ui, tl!("List View"), true, list) {
        set_pstate(ui.ctx(), "br-list", true);
    }
    ui.separator();
    ui.menu_button(tl!("Open Brush Library"), |ui| library_panel::library_menu::<BrushLibraries>(app, ui));
    if menu_item(ui, tl!("Save Brush Library…"), has_brushes, false) {
        save_library(app, targets);
    }
}

/// Save Brush Library… for the document's brushes (`names`: those selected in the panel).
fn save_library(app: &mut VectorcraftApp, names: Vec<String>) {
    if let Err(e) = app.run("ui.saveBrushLibrary", json!({ "names": names })) {
        app.status(e);
    }
}

/// A brush library's brush released over `zone` (outlined while one is held over it): → (its
/// library, its name).
fn library_brush_drop(ui: &Ui, zone: &egui::Response) -> Option<(String, String)> {
    let drag = zone.dnd_hover_payload::<PanelDrag>()?;
    let PanelDrag::LibraryBrush { library, name } = &*drag else { return None };
    ui.painter().rect_stroke(zone.rect, 0.0, egui::Stroke::new(1.5, Tokens::get(ui.ctx()).accent), egui::StrokeKind::Inside);
    zone.dnd_release_payload::<PanelDrag>().map(|_| (library.clone(), name.clone()))
}

// ---------- libraries ----------

/// A brush of a library as the library panel shows it.
pub(crate) struct LibBrush {
    lib: Arc<BrushLibrary>,
    index: usize,
    /// Its definition and preview key, with the library's resources.
    shown: Shown,
}

impl LibBrush {
    fn name(&self) -> &str {
        &self.shown.name
    }
}

/// The brushes of library `lib` as its panel shows them, worked out once per library: a frame of
/// the panel serialises and hashes none.
pub(crate) fn library_brushes(lib: &Arc<BrushLibrary>) -> Rc<Vec<LibBrush>> {
    thread_local! {
        static MEMO: RefCell<Memo<BrushLibrary, Vec<LibBrush>>> = RefCell::new(Memo::default());
    }
    MEMO.with(|m| {
        m.borrow_mut().get(lib, 0, || {
            lib.brushes.iter().enumerate().map(|(index, b)| LibBrush { lib: lib.clone(), index, shown: Shown::of(&lib.doc, b) }).collect()
        })
    })
}

/// The chip brush `name` of the document version `st` shows at the pointer while dragged.
pub(crate) fn document_chip(ui: &Ui, r: egui::Rect, st: &DocState, name: &str) {
    if let Some(b) = document_brushes(st).iter().find(|b| b.name == name) {
        chip(ui, r, &st.doc, &b.def, b.key);
    }
}

/// The chip brush `name` of library `lib` shows at the pointer while dragged.
pub(crate) fn library_chip(ui: &Ui, r: egui::Rect, lib: &Arc<BrushLibrary>, name: &str) {
    if let Some(b) = library_brushes(lib).iter().find(|b| b.name() == name) {
        chip(ui, r, &lib.doc, &b.shown.def, b.shown.key);
    }
}

/// Brush libraries in library panels (engine: `brush.libraries`, `brush.addFromLibrary`…).
pub(crate) struct BrushLibraries;

impl LibraryKind for BrushLibraries {
    const KIND: &'static str = "brushes";
    const OPEN: &'static str = "window.brushLibrary";
    const LIST: &'static str = "brush.libraries";
    const LOAD: &'static str = "brush.loadLibrary";
    const MENU: &'static str = "Brush Libraries Menu";
    const ADD: &'static str = "Add to Brushes";
    /// Stroke previews need room to show the brush.
    const VIEW: super::swatches::View = super::swatches::View::LargeThumb;
    type Lib = Rc<Vec<LibBrush>>;
    type Item = LibBrush;

    fn list(app: &VectorcraftApp) -> Vec<LibraryRef> {
        let libs = brushlib::libraries(&app.session).into_iter();
        libs.map(|l| LibraryRef { submenu: library_panel::submenu(l.category), id: l.id, name: l.name }).collect()
    }
    fn get(app: &VectorcraftApp, id: &str) -> Option<(String, Self::Lib)> {
        let (info, lib) = brushlib::library(&app.session, id)?;
        Some((info.name, library_brushes(&lib)))
    }
    fn find(app: &VectorcraftApp, key: &str) -> library_panel::Found {
        brushlib::library(&app.session, key).map(|(info, lib)| (info.id, info.name, lib.len()))
    }
    fn path(app: &VectorcraftApp, id: &str) -> Option<String> {
        app.session.brush_libraries.path(id).map(str::to_string)
    }
    fn rows<'a>(lib: &'a Self::Lib, query: &str) -> Vec<Row<'a, LibBrush>> {
        let found = |n: &str| query.is_empty() || n.to_lowercase().contains(query);
        lib.iter().filter(|b| found(b.name())).map(|b| Row { name: b.name(), item: Some(b) }).collect()
    }
    /// The brush's stroke preview, drawn with the library's resources.
    fn draw(_: &VectorcraftApp, ui: &Ui, r: egui::Rect, b: &LibBrush, selected: bool, hovered: bool) {
        chip(ui, r, &b.lib.doc, &b.shown.def, b.shown.key);
        super::graphic_styles::outline(ui, r, selected, hovered);
    }
    fn row_icons(_: &Ui, _: egui::Rect, _: &LibBrush) {}
    /// Its type.
    fn describe(b: &LibBrush) -> String {
        b.lib.brushes.get(b.index).map_or_else(String::new, |x| tl!(x.kind.label()).to_string())
    }
    /// Adds the brush to the document's Brushes as the current brush, and applies it to the
    /// selected paths, as clicking a brush of the Brushes panel does.
    fn click(app: &mut VectorcraftApp, _: &Ui, id: &str, name: &str) {
        let apply = app.session.active().is_some_and(|d| !d.selection.is_empty());
        add(app, json!({"library": id, "name": name, "apply": apply}));
    }
    fn add(app: &mut VectorcraftApp, id: &str, names: Vec<String>) {
        add(app, json!({"library": id, "names": names}));
    }
    /// Dragged onto a path, the brush is added and applied to it; onto the Brushes panel, added.
    fn drag(id: &str, name: &str) -> Option<PanelDrag> {
        Some(PanelDrag::LibraryBrush { library: id.to_string(), name: name.to_string() })
    }
    fn menu_tail(app: &mut VectorcraftApp, ui: &mut Ui) {
        ui.separator();
        // As a command, so its file dialog, shown off the UI thread, loads the library when it
        // answers.
        if menu_item(ui, tl!("Other Library…"), true, false)
            && let Err(e) = app.run("window.brushLibrary.other", json!({}))
        {
            app.status(e);
        }
        let has_brushes = has_brushes(app);
        if menu_item(ui, tl!("Save Brushes…"), has_brushes, false) {
            let (list, current) = brushes(app);
            let names = targets(app, ui.ctx(), &list, current.as_deref());
            save_library(app, names);
        }
    }
}

/// Run `brush.addFromLibrary` with `p`; a failure goes to the status bar.
fn add(app: &mut VectorcraftApp, p: Value) {
    if let Err(e) = app.run("brush.addFromLibrary", p) {
        app.status(e);
    }
}

/// Has the active document brushes to save? (Its stored library isn't parsed: a document without
/// one has the default brushes.)
pub(crate) fn has_brushes(app: &VectorcraftApp) -> bool {
    let stored = |d: &Document| d.unknown.get(vectorcraft_brush::DOC_KEY).map(|v| v.as_array().is_some_and(|a| !a.is_empty()));
    app.session.active().is_some_and(|st| stored(&st.doc).unwrap_or(true))
}

/// `window.brushLibrary {library}`: open a brush library in its own panel (`library` null closes
/// every brush library panel).
pub(crate) fn open_library(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    library_panel::open_command::<BrushLibraries>(app, p)
}

/// `brush.loadLibrary` params, then open the library in its panel.
pub(crate) fn load_library(app: &mut VectorcraftApp, params: Value) -> Result<Value, String> {
    let r = app.run("brush.loadLibrary", params)?;
    open_library(app, &json!({ "library": r["library"] }))
}

/// Other Library…: load the brushes of the document at `path`, else of one picked in an open
/// dialog.
pub(crate) fn other_library(app: &mut VectorcraftApp, path: Option<String>) -> Result<Value, String> {
    match library_panel::pick_library_file(app, path, library_panel::library_pick_filters(BrushLibraries::KIND))? {
        Some(path) => load_library(app, json!({ "path": path })),
        None => Ok(Value::Null),
    }
}

/// The id prefix of the Window → Brush Libraries → User Defined slots.
pub(crate) const USER_SLOT: &str = "window.userBrushLibrary";

/// The User Defined library slot `id` (`window.userBrushLibrary3`) stands for.
pub(crate) fn user_library(app: &VectorcraftApp, id: &str) -> Option<LibraryInfo> {
    library_panel::user_slot(id, USER_SLOT, brushlib::libraries(&app.session))
}

/// Window → Brush Libraries.
pub(crate) fn window_menu() -> Vec<Item> {
    const SLOTS: [&str; 10] = [
        "window.userBrushLibrary1",
        "window.userBrushLibrary2",
        "window.userBrushLibrary3",
        "window.userBrushLibrary4",
        "window.userBrushLibrary5",
        "window.userBrushLibrary6",
        "window.userBrushLibrary7",
        "window.userBrushLibrary8",
        "window.userBrushLibrary9",
        "window.userBrushLibrary10",
    ];
    let open = |b: &'static brushlib::BuiltinBrushLibrary| Item::Cmd(b.name, BrushLibraries::OPEN, json!({ "library": b.id }));
    let mut items: Vec<Item> = brushlib::BUILTIN.iter().map(open).collect();
    items.extend([
        Item::Sep,
        Item::Sub("User Defined", SLOTS.iter().map(|id| Item::Cmd("User Library", id, Value::Null)).collect()),
        Item::Sep,
        Item::Cmd("Other Library…", "window.brushLibrary.other", Value::Null),
        Item::Cmd("Save Brushes…", "ui.saveBrushLibrary", Value::Null),
    ]);
    items
}

/// Brush definitions serialised and preview keys hashed on this test thread.
#[cfg(test)]
pub(crate) mod counters {
    thread_local! {
        pub(crate) static KEYED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    }
}

#[cfg(test)]
mod tests;
