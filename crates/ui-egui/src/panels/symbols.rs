//! Symbols panel: the document's symbols as rendered thumbnails or in a small or large list. Click
//! selects a symbol (it becomes the Symbol Sprayer's current symbol); double-click opens it in symbol editing mode. Symbol
//! libraries open in library panels ([`SymbolLibraries`]).

use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{Arc, Weak};

use craft::behavior::{IconId, Selection, SelectionModel as _};
use craft::keymap::Mods;
use craft::ui::buttons::{Button, IconButton};
use craft::ui::contract::{
    Bind, DropAt, DropPlace, Entry, ListDrag, MemoProps, PopupKind, Props as _, SelectMode, Show as _, ShowContainer as _, Trigger,
};
use craft::ui::display::Divider;
use craft::ui::form::Field;
use craft::ui::menu::{Menu, MenuItem, PanelRow};
use craft::ui::patterns::presets::{
    PresetBrowser, PresetDrop, PresetEvent, PresetFns, PresetRow, PresetRowMenu, PresetSource, PresetState, PresetView,
};
use craft::ui::pickers::Select;
use craft::ui::primitives::textures::{Priority, Revision, ThumbPaint, ThumbRender, ThumbRequest, ThumbSource, ThumbnailOptions, Thumbnails};
use craft::ui::primitives::{ContentPainterCx, IconSpec};
use craft::ui::runtime::commands::{CommandRef, CommandState, CommandStates};
use craft::ui::sections::{DropAction, FooterAction, FooterBar, FooterOutput, Panel};
use craft::ui::style::DataColor;
use egui::{Color32, Rect, Ui};
use serde_json::{Value, json};
use vectorcraft_doc::{Document, Node, NodeId, NodeKind};
use vectorcraft_engine::DocState;
use vectorcraft_engine::cmd::clipboard::{ArtResources, art_resources};
use vectorcraft_engine::cmd::swatchlib::LibraryInfo;
use vectorcraft_engine::cmd::symbollib::{self, SymbolLibrary};

use super::library_panel::{self, LibraryKey, LibraryKind, LibraryRef, Row};
use super::memo::Memo;
use crate::VectorcraftApp;
use crate::caches::SymbolInstances;
use crate::menus::{Sep, c, cp, sub};
use crate::panel_drag::PanelDrag;

/// The largest side of a symbol thumbnail, in pixels. A larger chip draws its thumbnail scaled up.
const MAX_THUMB_PX: f32 = 1024.0;

/// The pixels on a side of the thumbnail for a chip `size` points wide at `pixels_per_point`: 8
/// to [`MAX_THUMB_PX`].
fn thumb_px(size: f32, pixels_per_point: f32) -> u32 {
    let px = (size * pixels_per_point).round();
    if px.is_nan() { 8 } else { px.clamp(8.0, MAX_THUMB_PX) as u32 }
}

/// A symbol in the Symbols panel: the uid of the document it belongs to and its name. It keys the
/// panel's rows and is what a symbol dragged out of the panel carries (`ListDrag<SymbolKey>`), so a
/// drop knows the document the symbol came from.
#[derive(Clone, Debug, PartialEq, Eq, Hash, MemoProps)]
pub(crate) struct SymbolKey {
    /// The uid of the document whose Symbols panel showed the symbol.
    pub doc: u64,
    /// The symbol's name.
    pub name: String,
}

/// Which picture of a symbol a thumbnail is, keyed by the symbol's [`Thumbs`] key, which changes
/// with anything the picture shows.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum SymbolThumb {
    /// An instance at its natural size ([`render_thumb`]).
    Symbol { key: u64 },
    /// The art's bounds (`art`, as `f64` bits) stretched to `w` × `h` pixels ([`render_art`]).
    Art { key: u64, w: u32, h: u32, art: [u64; 4] },
}

/// Makes the pictures of symbol `name` of `doc`.
struct SymbolPictures<'a> {
    doc: &'a Document,
    name: &'a str,
}

impl ThumbSource<SymbolThumb> for SymbolPictures<'_> {
    /// The key already names the symbol's content, so one key has one picture.
    fn revision(&self, key: &SymbolThumb, _: &ThumbRequest) -> Revision {
        Revision::of(key)
    }

    /// Rendered on the UI thread, which the store limits to a few renders a pass. An art picture is
    /// `w` × `h`, which never exceeds the request's side: both come from the same cell, the side
    /// rounded up ([`paint_symbol_art`]).
    fn render(&self, key: &SymbolThumb, request: &ThumbRequest) -> ThumbRender {
        #[cfg(test)]
        counters::RENDERED.with(|c| c.set(c.get() + 1));
        let mut renderer = vectorcraft_render::Renderer::new();
        let img = match key {
            SymbolThumb::Symbol { .. } => render_thumb(&mut renderer, self.doc, self.name, request.side),
            SymbolThumb::Art { w, h, art, .. } => {
                let [x0, y0, x1, y1] = art.map(f64::from_bits);
                render_art(&mut renderer, self.doc, self.name, vectorcraft_geom::Rect::new(x0, y0, x1, y1), (*w, *h))
            }
        };
        img.map_or(ThumbRender::Blank, |img| ThumbRender::Ready(crate::canvas::color_image_owned(img)))
    }
}

/// Paints the picture `key` of symbol `name` of `doc` into `rect`, on white paper, from the one
/// store of symbol thumbnails (the panel, library panels and dialogs share it).
fn paint_picture(cx: &ContentPainterCx<'_>, rect: Rect, doc: &Document, name: &str, key: &SymbolThumb) {
    let ctx = cx.painter.ctx();
    let thumbs = Thumbnails::<SymbolThumb>::in_context(ctx, "symbols", ThumbnailOptions::new().max_side(MAX_THUMB_PX as u32));
    let request = ThumbRequest::for_cell(rect.width().max(rect.height()), cx.pixels_per_point());
    let state = thumbs.state(ctx, &SymbolPictures { doc, name }, key, &request, Priority::VISIBLE);
    thumbs.paint(cx, rect, &state, &ThumbPaint::default().paper(DataColor::new(Color32::WHITE)));
}

/// Symbol `name`'s thumbnail (keyed by `keys`, [`document_symbols`] or [`library_symbols`] of
/// `doc`) on white in `rect`: the panel's tiles and rows, a library panel's, and the 3D dialogs'
/// graphics.
// PLAN: ap.AS10 gives `doc: &Arc<Document>`, but a symbol library's document is a plain
// `Document` (`SymbolLibrary::doc`); `&Document` takes both, as an `&Arc<Document>` coerces.
pub(crate) fn paint_symbol(cx: &ContentPainterCx<'_>, rect: Rect, doc: &Document, keys: &Thumbs, name: &str) {
    let Some(&key) = keys.keys.get(name) else { return };
    paint_picture(cx, rect, doc, name, &SymbolThumb::Symbol { key });
}

/// Symbol `name`'s art on white in `rect`, its bounds `art` (at its own size, as a mapped graphic
/// measures them) stretched to fill `rect`, as a box the art is scaled into shows it. The art is
/// rendered at most [`MAX_THUMB_PX`] pixels on a side, keyed by its key in `keys`.
pub(crate) fn paint_symbol_art(cx: &ContentPainterCx<'_>, rect: Rect, doc: &Document, keys: &Thumbs, name: &str, art: vectorcraft_geom::Rect) {
    let Some(&key) = keys.keys.get(name) else { return };
    let ppp = cx.pixels_per_point();
    let (w, h) = (thumb_px(rect.width(), ppp), thumb_px(rect.height(), ppp));
    let art = [art.x0, art.y0, art.x1, art.y1].map(f64::to_bits);
    paint_picture(cx, rect, doc, name, &SymbolThumb::Art { key, w, h, art });
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

/// [`Thumbs`] of document `doc` at `revision`.
pub(crate) fn document_symbols(doc: &Arc<Document>, revision: u64) -> Rc<Thumbs> {
    thread_local! {
        static MEMO: RefCell<Memo<Document, Thumbs>> = RefCell::new(Memo::default());
    }
    MEMO.with(|m| m.borrow_mut().get(doc, revision, || Thumbs::of(doc)))
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

/// The symbol instances in the selection of `st` and in its groups, in one walk of the selected
/// subtrees.
fn walk_instances(st: &DocState) -> SymbolInstances {
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
    found
}

/// The symbol instances in the selection and in its groups, found in one walk of the selected
/// subtrees per (document uid, revision) ([`crate::caches::CanvasCache::symbol_instances`]);
/// `None` without a document.
fn selected_instances(app: &mut VectorcraftApp) -> Option<&mut SymbolInstances> {
    let st = app.session.active()?;
    let key = (st.uid, st.revision);
    if app.canvas.symbol_instances.as_ref().is_none_or(|(k, _)| *k != key) {
        app.canvas.symbol_instances = Some((key, walk_instances(st)));
    }
    app.canvas.symbol_instances.as_mut().map(|(_, found)| found)
}

/// Walks the selection for its symbol instances unless this document version was walked already, for the readers
/// that only borrow the app ([`instance_flags`]).
pub(crate) fn cache_instances(app: &mut VectorcraftApp) {
    selected_instances(app);
}

fn has_instances_selected(app: &mut VectorcraftApp) -> bool {
    selected_instances(app).is_some_and(|found| !found.ids.is_empty())
}

/// Whether one of the instances `ids` of `doc` has a scale, rotation or shear that Reset
/// Transformation would change.
fn transformed(doc: &Document, ids: &[NodeId]) -> bool {
    ids.iter().filter_map(|id| doc.node(*id)).any(|n| vectorcraft_engine::cmd::reset_transformation_changes(doc, n))
}

/// What the selected symbol instances allow, for the panel menu's states.
#[derive(Clone, Copy, Debug, Default)]
struct InstanceFlags {
    /// The selection holds an instance.
    any: bool,
    /// One has appearance changes of its own ([`changed_instances_selected`]).
    changed: bool,
    /// One has a transform Reset Transformation changes ([`transformed_instances_selected`]).
    transformed: bool,
}

/// [`InstanceFlags`] read without writing the canvas cache: from the cache while it holds the
/// active document version, else from a walk of the selection.
/// The menus' rebuild fills the cache first ([`cache_instances`]), so the panel menu's states read it.
fn instance_flags(app: &VectorcraftApp) -> InstanceFlags {
    let Some(st) = app.session.active() else { return InstanceFlags::default() };
    let walked;
    let found = match &app.canvas.symbol_instances {
        Some((key, found)) if *key == (st.uid, st.revision) => found,
        _ => {
            walked = walk_instances(st);
            &walked
        }
    };
    InstanceFlags {
        any: !found.ids.is_empty(),
        changed: found.changed,
        transformed: found.transformed.unwrap_or_else(|| transformed(&st.doc, &found.ids)),
    }
}

/// The names of the symbols the selected instances are instances of, each once.
fn linked_symbols(app: &mut VectorcraftApp) -> Vec<String> {
    let Some(doc) = app.session.active().map(|st| st.doc.clone()) else { return vec![] };
    let Some(found) = selected_instances(app) else { return vec![] };
    let mut names: Vec<String> = found
        .ids
        .iter()
        .filter_map(|id| match doc.node(*id).map(|n| &n.kind) {
            Some(NodeKind::SymbolInstance { symbol, .. }) => Some(symbol.clone()),
            _ => None,
        })
        .collect();
    names.sort();
    names.dedup();
    names
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

/// The rows of a symbol's context menu ([`PresetRowMenu::Host`]): the command each row reports and
/// its label.
const ITEM_PLACE: &str = "place";
const ITEM_EDIT: &str = "edit";
const ITEM_DUPLICATE: &str = "duplicate";
const ITEM_DELETE: &str = "delete";
const ITEM_BREAK_LINK: &str = "breakLink";
const ITEM_SELECT_INSTANCES: &str = "selectInstances";
const ITEM_ROWS: [(&str, &str); 6] = [
    (ITEM_PLACE, "Place Symbol Instance"),
    (ITEM_EDIT, "Edit Symbol"),
    (ITEM_DUPLICATE, "Duplicate Symbol"),
    (ITEM_DELETE, "Delete Symbol"),
    (ITEM_BREAK_LINK, "Break Link to Symbol"),
    (ITEM_SELECT_INSTANCES, "Select All Instances"),
];

/// The context menu of symbol `name`; Break Link to Symbol is enabled while the selection holds an
/// instance of it (`linked`, [`linked_symbols`]).
fn item_rows(linked: &[String], name: &str) -> Vec<Entry<String>> {
    let has_instances = linked.iter().any(|n| n == name);
    ITEM_ROWS
        .iter()
        .map(|&(command, label)| Entry::new(command.to_string(), tl!(label)).enabled(command != ITEM_BREAK_LINK || has_instances))
        .collect()
}

/// Runs the context menu row `command` ([`item_rows`]) on symbol `name`.
fn item_command(app: &mut VectorcraftApp, name: &str, command: &str) {
    match command {
        ITEM_PLACE => {
            let center = app
                .view()
                .map(|v| v.center)
                .or_else(|| app.session.active().and_then(|st| st.doc.artboards.first().map(|a| a.rect.center())))
                .unwrap_or(vectorcraft_geom::Point::ZERO);
            run_reported(app, "symbol.place", json!({ "name": name, "x": center.x, "y": center.y }));
        }
        ITEM_EDIT => run_reported(app, "symbol.edit", json!({ "name": name })),
        ITEM_DUPLICATE => run_reported(app, "symbol.duplicate", json!({ "name": name })),
        ITEM_DELETE => delete(app, &[], Some(name), false),
        ITEM_BREAK_LINK => {
            let ids = selected_symbol_instances(app, name);
            if !ids.is_empty() {
                run_reported(app, "symbol.breakLink", json!({ "ids": ids }));
            }
        }
        ITEM_SELECT_INSTANCES => run_reported(app, "symbol.selectInstances", json!({ "name": name })),
        _ => {}
    }
}

/// Runs command `id` with `params`; a failure goes to the status bar.
fn run_reported(app: &mut VectorcraftApp, id: &str, params: Value) {
    if let Err(e) = app.run(id, params) {
        app.status(e);
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
    *found.transformed.get_or_insert_with(|| transformed(&doc, ids))
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
/// The controls are one ranked row of the bar: a bar too narrow for it leaves the whole group out
/// by its rank before the Transform link. Returns whether it drew them (symbol instances alone are
/// selected).
pub(crate) fn control_bar(app: &mut VectorcraftApp, ui: &mut Ui) -> bool {
    let Some(symbols) = selected_instance_symbols(app) else { return false };
    let Some(st) = app.session.active() else { return false };
    let entries: Vec<Entry<String>> = st.doc.symbols.iter().map(|s| Entry::new(s.name.clone(), s.name.clone())).collect();
    let shared = symbols.first().filter(|first| symbols.iter().all(|s| s == *first)).cloned().unwrap_or_default();
    // Reset: Reset Transformation, enabled while an instance's scale, rotation or shear differs
    // from its symbol's.
    let transformed = transformed_instances_selected(app);
    // PLAN: assumed `crate::chrome::rank::GROUP: u8` (ch.P9, chrome-w16).
    let mut shown = craft::ui::layout_components::Row::new().fit_rank(crate::chrome::rank::GROUP).show(ui, |ui, _| {
        Divider::vertical().show(ui);
        // Edit Symbol: the instances' symbol in symbol editing mode, when they share one.
        let edit = !shared.is_empty()
            && IconButton::new(IconSpec::svg(IconId::new("dc-place-symbol")))
                .label(tl!("Edit Symbol"))
                .tooltip(tl!("Edit Symbol"))
                .show(ui)
                .clicked();
        // The select reports a pick of another symbol only.
        let replace = Field::new(tl!("Replace:"))
            .show(ui, |ui, _| Select::new(tl!("Replace:"), Bind::given(shared.clone()), &entries).show(ui))
            .inner
            .and_then(|out| out.events.committed().cloned());
        let reset = Button::new(tl!("Reset")).tooltip(tl!("Reset Transformation")).enabled(transformed).show(ui).clicked();
        (edit, replace, reset)
    });
    let (edit, replace, reset) = shown.settle(|_, picked| std::mem::take(picked)).unwrap_or_default();
    if edit {
        run_reported(app, "symbol.edit", json!({}));
    }
    if let Some(name) = replace.filter(|name| *name != shared) {
        run_reported(app, "symbol.replace", json!({ "name": name }));
    }
    if reset {
        run_reported(app, "symbol.instance.resetTransform", json!({}));
    }
    true
}

/// What the panel's rows and thumbnails read: the symbols of one version of a document.
#[derive(MemoProps)]
struct SymbolSource<'a> {
    uid: u64,
    revision: u64,
    #[memo_props(covered_by = "revision", reason = "a document version's symbols are fixed")]
    doc: &'a Document,
    #[memo_props(covered_by = "revision", reason = "worked out once from the document version")]
    keys: &'a Thumbs,
}

/// The panel's presets: a row per symbol of the document, in its order, each with its thumbnail.
fn symbol_source(inputs: SymbolSource<'_>) -> PresetSource<'_, SymbolKey> {
    let fns = PresetFns::<SymbolSource<'_>, SymbolKey>::new(|s, _| {
        s.doc.symbols.iter().map(|sym| PresetRow::item(SymbolKey { doc: s.uid, name: sym.name.clone() }, sym.name.clone(), 0)).collect()
    })
    .thumb(|s, cx, rect, info| paint_symbol(cx, rect, s.doc, s.keys, &info.key.name));
    PresetSource::new(inputs, fns)
}

/// The footer's actions, by their index in the order [`show`] gives them: the first [`AT_START`]
/// sit at the start edge (Library, Place, Break Link, Options), then Delete at the end edge with
/// New before it.
const AT_LIBRARY: usize = 0;
const AT_PLACE: usize = 1;
const AT_BREAK_LINK: usize = 2;
const AT_OPTIONS: usize = 3;
const AT_DELETE: usize = 4;
const AT_NEW: usize = 5;
const AT_START: usize = 4;

/// The panel: the document's symbols in a preset browser, over the footer's actions.
///
/// A click makes a symbol current and a double click edits it; a symbol drags out to the canvas,
/// onto New Symbol (duplicated) or onto Delete Symbol. Art dragged off the canvas and dropped on the
/// list becomes a symbol (Shift at the release keeps the art as ordinary art); dropped on a symbol
/// with Alt it redefines that symbol in one undo step.
pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let Some((doc, revision, uid, reverts, has_sel)) =
        app.session.active().map(|st| (st.doc.clone(), st.revision, st.uid, st.reverts, !st.selection.is_empty()))
    else {
        return;
    };
    let keys = document_symbols(&doc, revision);
    let sel = current(app);
    let marked = marked(app);
    let inst = has_instances_selected(app);
    let linked = linked_symbols(app);
    let key = |name: &str| SymbolKey { doc: uid, name: name.to_string() };
    let source = symbol_source(SymbolSource { uid, revision, doc: &doc, keys: &keys });
    // The symbols drawn selected: the ones Select All Unused picked, which further picks change,
    // else the current one.
    let (mut selection, select) = if marked.is_empty() {
        (sel.as_deref().map(|name| Selection::single(key(name))).unwrap_or_default(), SelectMode::Single)
    } else {
        (Selection::new(marked.iter().map(|name| key(name)).collect(), None, None), SelectMode::Multi)
    };
    let (view, zoom) = app.ui.symbols_view.preset();
    let browser_id = ui.make_persistent_id("symbols-browser");
    let list = PresetBrowser::<SymbolKey>::list_id(browser_id);
    // A symbol dragged from this panel's list, of the document it shows.
    let ours = move |drag: &ListDrag<SymbolKey>| drag.list == list && drag.grabbed.doc == uid;
    let svg = |name: &'static str| IconSpec::svg(IconId::new(name));
    let actions = [
        FooterAction::new(svg("library"), tl!(SymbolLibraries::MENU))
            .popup(PopupKind::Menu)
            .pressed(library_panel::any_open::<SymbolLibraries>(&app.ui)),
        FooterAction::new(svg("dc-place-symbol"), tl!("Place Symbol Instance")).enabled(sel.is_some()),
        FooterAction::new(svg("link-2-off"), tl!("Break Link to Symbol")).enabled(inst),
        // Symbol Options on the current symbol, as the panel menu's Symbol Options… opens it.
        FooterAction::new(svg("dc-options"), tl!("Symbol Options")).enabled(sel.is_some()),
        FooterAction::new(svg("trash-2"), tl!("Delete Symbol"))
            .enabled(sel.is_some() || !marked.is_empty())
            .takes_drops_disabled(true)
            .accepts_drops(ours),
        // A symbol dropped here is duplicated with or without art selected.
        FooterAction::new(svg("dc-new-item"), tl!("New Symbol")).enabled(has_sel).takes_drops_disabled(true).accepts_drops(ours),
    ];
    let footer: RefCell<Option<FooterOutput>> = RefCell::new(None);
    let mut shown = Panel::new(tl!("Symbols"))
        .collection_body()
        .footer(|ui, _| *footer.borrow_mut() = Some(FooterBar::actions(actions.clone()).at_start(AT_START).show_footer(ui)))
        .show(ui, |ui, _| {
            // Art dropped anywhere on the list becomes a symbol; dropped on a symbol, the browser's
            // own target takes it.
            DropAction::<PanelDrag>::new(move |drag| art_of(drag, uid, reverts).is_some()).show(ui, |ui, _| {
                // With no symbols the body says how to make one, as the other panels' empty states do.
                if doc.symbols.is_empty() {
                    super::empty_state(ui, "spray-can", tl!("No symbols in this document"), tl!("Select art and click New Symbol to make one."));
                    return None;
                }
                let mut state = PresetState::new();
                state.zoom = zoom;
                let shown = PresetBrowser::new(tl!("Symbols"), Bind::bound(&mut state), &source)
                    .browser_id(browser_id)
                    .selection(Bind::given(selection.clone()))
                    .select(select)
                    .view_lock(view)
                    .drags(true)
                    .row_menu(PresetRowMenu::Host)
                    .row_items(|key: &SymbolKey| item_rows(&linked, &key.name))
                    .takes_drop(PresetDrop::new::<PanelDrag>(DropPlace::Onto, move |drag: &PanelDrag, _: &DropAt<SymbolKey>| {
                        art_of(drag, uid, reverts).is_some()
                    }))
                    .show(ui);
                Some(shown)
            })
        });
    let taken = shown.settle(|_, list| {
        let dropped = list.output.dropped.take().map(|drag| (drag, list.output.mods));
        (dropped, list.inner.take().flatten().map(|out| out.events).unwrap_or_default())
    });
    let footer = footer.into_inner();
    if let Some(opener) = footer.as_ref().and_then(|out| out.responses.get(AT_LIBRARY)) {
        Menu::new(tl!(SymbolLibraries::MENU))
            .opened_by(opener, Trigger::Press)
            .show(ui, |ui, _| library_panel::library_menu::<SymbolLibraries>(app, ui));
    }
    if let Some((dropped, events)) = taken {
        if let Some((drag, mods)) = dropped
            && let Some(ids) = art_of(&drag, uid, reverts)
        {
            new_symbol(app, ids, mods.shift);
        }
        for event in events {
            match event {
                PresetEvent::Select { key, .. } => pick(app, &key.name),
                PresetEvent::Activate { key, .. } => {
                    pick(app, &key.name);
                    run_reported(app, "symbol.edit", json!({ "name": key.name }));
                }
                PresetEvent::Selection(change) => {
                    selection.apply(change);
                    let mut names: Vec<String> = selection.items.iter().filter(|key| key.doc == uid).map(|key| key.name.clone()).collect();
                    // One symbol left selected is the current one again (a secondary press on another
                    // symbol, a Cmd-click that leaves one), not a pick.
                    if names.len() == 1
                        && let Some(name) = names.pop()
                    {
                        pick(app, &name);
                    } else {
                        set_picks(app, names);
                    }
                }
                PresetEvent::Dropped { at, payload, mods, .. } => {
                    let Some(ids) = payload.get::<PanelDrag>().and_then(|drag| art_of(drag, uid, reverts)) else { continue };
                    if !mods.alt {
                        new_symbol(app, ids, mods.shift);
                    } else if let Some(target) = at.key()
                        && redefines(&doc, ids)
                    {
                        // Redefine Symbol with the dropped art, which becomes an instance of it, in one
                        // undo step. Alt art that cannot redefine changes nothing.
                        run_reported(app, "symbol.update", json!({ "name": target.name, "ids": node_ids(ids) }));
                    }
                }
                PresetEvent::Menu { target, command, .. } => item_command(app, &target.name, &command),
                _ => {}
            }
        }
    }
    let Some(out) = footer else { return };
    match &out.dropped {
        Some(dropped) => {
            let Some(drag) = dropped.payload::<ListDrag<SymbolKey>>() else { return };
            let name = &drag.grabbed.name;
            // A symbol deleted while it was dragged drops nothing.
            if !doc.symbols.iter().any(|s| s.name == *name) {
                return;
            }
            match dropped.index {
                AT_NEW => run_reported(app, "symbol.duplicate", json!({ "name": name })),
                // The dragged symbol, or every symbol Select All Unused picked when it is one of them.
                AT_DELETE => {
                    let picked: &[String] = if marked.contains(name) { &marked } else { &[] };
                    delete(app, picked, Some(name), out.mods.alt);
                }
                _ => {}
            }
        }
        None => match out.picked {
            Some(AT_PLACE) => run_reported(app, "symbol.place", json!({})),
            Some(AT_BREAK_LINK) => run_reported(app, "symbol.breakLink", json!({})),
            Some(AT_OPTIONS) => run_reported(app, "ui.symbolOptions", json!({ "edit": true, "name": sel })),
            Some(AT_NEW) => {
                // Alt skips Symbol Options: the symbol is made under the default name, dynamic as the
                // dialog's default. Shift keeps the selected art as ordinary art, with or without Alt.
                let (id, mut params) = if out.mods.alt { ("symbol.new", json!({ "type": "dynamic" })) } else { ("ui.symbolOptions", json!({})) };
                fired_params(id, &mut params, out.mods);
                run_reported(app, id, params);
            }
            // Alt deletes an unused symbol without asking; a symbol with instances still asks.
            Some(AT_DELETE) => delete(app, &marked, sel.as_deref(), out.mods.alt),
            _ => {}
        },
    }
}

/// The ids of the nonempty artwork dragged from the displayed drawing.
fn art_of(drag: &PanelDrag, uid: u64, reverts: u64) -> Option<&[NodeId]> {
    drag.artwork_for(uid, reverts).filter(|ids| !ids.is_empty())
}

/// Whether the art `ids` can redefine a symbol of `doc`: every object is still there and none is a
/// layer.
fn redefines(doc: &Document, ids: &[NodeId]) -> bool {
    ids.iter().all(|id| doc.node(*id).is_some_and(|n| !n.is_layer()))
}

/// The ids as the symbol commands take them.
fn node_ids(ids: &[NodeId]) -> Vec<u64> {
    ids.iter().map(|id| id.0).collect()
}

/// A dynamic symbol of the art `ids`, which stays ordinary art when `keep_art`.
fn new_symbol(app: &mut VectorcraftApp, ids: &[NodeId], keep_art: bool) {
    let mut params = json!({ "ids": node_ids(ids), "type": "dynamic" });
    if keep_art {
        params["keepArt"] = json!(true);
    }
    run_reported(app, "symbol.new", params);
}

/// Makes `name` the current symbol and drops the symbols Select All Unused picked: a click picks
/// one symbol again.
fn pick(app: &mut VectorcraftApp, name: &str) {
    set_picks(app, vec![]);
    if current(app).as_deref() != Some(name) {
        run_reported(app, "symbol.setCurrent", json!({ "name": name }));
    }
}

/// The parameters of a command fired with `mods` from the panel's menu or its New Symbol button:
/// Shift keeps the art as ordinary art (`keepArt`) for New Symbol… (`ui.symbolOptions` for a new
/// symbol), Redefine Symbol (`symbol.update`) and New Symbol with Alt (`symbol.new`). Other commands
/// and Symbol Options… on an existing symbol are left as they are.
pub(crate) fn fired_params(id: &str, params: &mut Value, mods: Mods) {
    let keeps_art = match id {
        "symbol.update" | "symbol.new" => true,
        "ui.symbolOptions" => params.get("edit").is_none_or(|edit| *edit == json!(false)) && params.get("name").is_none_or(Value::is_null),
        _ => false,
    };
    if !(mods.shift && keeps_art) {
        return;
    }
    if params.is_null() {
        *params = json!({});
    }
    if let Some(p) = params.as_object_mut() {
        p.insert("keepArt".into(), json!(true));
    }
}

/// The panel menu's rows that have no catalog command: `ui.symbols.<row>`, run by [`run`].
const ROW_DELETE: &str = "ui.symbols.delete";
const ROW_SELECT_UNUSED: &str = "ui.symbols.selectUnused";
const ROW_SAVE_LIBRARY: &str = "ui.symbols.saveLibrary";

/// One row of the panel menu with the state it shows.
enum MenuLine {
    Command(CommandRef, CommandState),
    Separator,
    /// Open Symbol Library, a submenu of the libraries.
    Libraries,
}

/// The panel menu in its order, each row with its state: one list for [`menu_rows`] and
/// [`menu_states`], so a row and its state cannot part.
fn menu_lines(app: &VectorcraftApp) -> Vec<MenuLine> {
    let sel = current(app);
    let (symbols, has_sel) = app.session.active().map_or((0, false), |st| (st.doc.symbols.len(), !st.selection.is_empty()));
    let picked = !marked(app).is_empty();
    let inst = instance_flags(app);
    let name = json!({ "name": sel });
    let row = |id: &str, params: Value, enabled: bool| MenuLine::Command(CommandRef::with_params(id, params), CommandState::new().enabled(enabled));
    let view = app.ui.symbols_view;
    let views = View::ALL.iter().map(|&(v, _)| {
        let params = json!({ "panel": "symbols", "view": v });
        MenuLine::Command(CommandRef::with_params("ui.panelView", params), CommandState::new().checked(view == v))
    });
    let mut lines = vec![
        // Shift on New Symbol… and Redefine Symbol keeps the selected art as ordinary art
        // ([`fired_params`]).
        row("ui.symbolOptions", json!({}), has_sel),
        row("symbol.update", name.clone(), has_sel && sel.is_some()),
        row("symbol.duplicate", name.clone(), sel.is_some()),
        // Asks first, as the panel's button does; the menu row has no Alt shortcut.
        row(ROW_DELETE, Value::Null, sel.is_some() || picked),
        row("ui.symbolOptions", json!({ "edit": true, "name": sel }), sel.is_some()),
        // The selected instance's symbol, else the symbol selected in the panel.
        row("symbol.edit", if inst.any { json!({}) } else { name.clone() }, inst.any || sel.is_some()),
        row("symbol.place", name.clone(), sel.is_some()),
        row("symbol.replace", name.clone(), inst.any && sel.is_some()),
        row("symbol.breakLink", json!({}), inst.any),
        row("symbol.instance.reset", json!({}), inst.changed),
        row("symbol.instance.resetTransform", json!({}), inst.transformed),
        row("symbol.selectInstances", name, sel.is_some()),
        MenuLine::Separator,
        // Select All Unused (#4918): the symbols nothing places show selected, and Delete Symbol
        // deletes them all.
        row(ROW_SELECT_UNUSED, Value::Null, symbols > 0),
        // The document's symbols in name order, as one undo step.
        row("symbol.sortByName", json!({}), symbols > 1),
        MenuLine::Separator,
    ];
    lines.extend(views);
    lines.extend([MenuLine::Separator, MenuLine::Libraries, row(ROW_SAVE_LIBRARY, Value::Null, symbols > 0)]);
    lines
}

/// The panel menu's rows.
pub(crate) fn menu_rows(app: &VectorcraftApp) -> Vec<PanelRow> {
    menu_lines(app)
        .into_iter()
        .map(|line| match line {
            MenuLine::Command(command, _) => PanelRow::Command(command),
            MenuLine::Separator => PanelRow::Separator,
            // PLAN: assumed `library_panel::library_rows::<K: LibraryKind>(app: &VectorcraftApp) -> Vec<PanelRow>`
            // (appearance-w17; ap.AS9 names `library_menu` and `library_items` only).
            MenuLine::Libraries => PanelRow::Submenu(tl!("Open Symbol Library").into(), library_panel::library_rows::<SymbolLibraries>(app)),
        })
        .collect()
}

/// The enabled and checked states of the panel menu's rows.
pub(crate) fn menu_states(app: &VectorcraftApp, states: &mut CommandStates) {
    for line in menu_lines(app) {
        if let MenuLine::Command(command, state) = line {
            states.set(command, state);
        }
    }
}

/// Runs the panel menu's own rows (`ui.symbols.<row>`); `None` for another id.
pub(crate) fn run(app: &mut VectorcraftApp, id: &str, _params: &Value) -> Option<Result<Value, String>> {
    let sel = current(app);
    Some(match id {
        ROW_DELETE => {
            let picked = marked(app);
            delete(app, &picked, sel.as_deref(), false);
            Ok(Value::Null)
        }
        ROW_SELECT_UNUSED => select_unused(app),
        ROW_SAVE_LIBRARY => {
            save_library(app, sel.into_iter().collect());
            Ok(Value::Null)
        }
        _ => return None,
    })
}

/// Select All Unused: picks the symbols nothing places, which show selected and which Delete
/// Symbol deletes.
fn select_unused(app: &mut VectorcraftApp) -> Result<Value, String> {
    let r = app.run("symbol.unused", json!({}))?;
    let unused: Vec<String> = serde_json::from_value(r["names"].clone()).unwrap_or_default();
    if unused.is_empty() {
        app.status(tl!("No unused symbols"));
    }
    set_picks(app, unused);
    Ok(Value::Null)
}

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
    /// The browser's view and thumbnail zoom that show this view: the grid at its own tile, or the
    /// list with small or large thumbnails (the browser's list takes small ones below a zoom of 0.75
    /// and large ones from 1.5).
    // PLAN: the sheet keeps `View::row` (row height, thumbnail side, text size of the old egui rows);
    // the browser lays its own rows out, so the view maps to the browser's view and zoom instead.
    fn preset(self) -> (PresetView, f64) {
        match self {
            View::Thumbnail => (PresetView::Grid, 1.0),
            View::SmallList => (PresetView::List, 0.6),
            View::LargeList => (PresetView::List, 1.5),
        }
    }
}

/// The symbols Select All Unused picked in the active document that it still has; none once
/// another document is active.
fn marked(app: &VectorcraftApp) -> Vec<String> {
    let Some(st) = app.session.active() else { return vec![] };
    let (of, picks) = &app.ui.symbol_picks;
    if *of != st.uid {
        return vec![];
    }
    picks.iter().filter(|m| st.doc.symbols.iter().any(|s| s.name == **m)).cloned().collect()
}

/// Picks the symbols `names` of the active document, as Select All Unused does; none clears the
/// picks.
fn set_picks(app: &mut VectorcraftApp, names: Vec<String>) {
    let uid = app.session.active().map_or(0, |st| st.uid);
    app.ui.symbol_picks = (uid, names);
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
    type Thumbs = ();

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
    fn rows<'a>(lib: &'a Self::Lib, _id: &str) -> Vec<Row<'a, LibSymbol>> {
        lib.iter().map(|s| Row { name: s.name(), item: Some(s) }).collect()
    }
    /// A built-in library's symbol in the UI language; a User Defined or loaded library's symbol as
    /// it is named.
    fn label<'a>(id: &str, name: &'a str) -> Cow<'a, str> {
        super::label_or_name(name, symbollib::BUILTIN.iter().any(|b| b.id == id)).into()
    }
    /// Find matches a symbol's name as shown (`label`) and as stored.
    fn row(id: &str, s: &LibSymbol, label: &str) -> PresetRow<LibraryKey> {
        let key = LibraryKey { panel: library_panel::panel_id(Self::KIND, id), name: s.name().to_string(), folder: false };
        let mut row = PresetRow::item(key, label, 0);
        row.search_name = Some(s.name().to_string());
        row
    }
    fn thumbs(_: &VectorcraftApp, _: &Self::Lib) {}
    /// The symbol's art on white, as the Symbols panel shows it.
    fn thumb(_: &(), cx: &ContentPainterCx<'_>, rect: Rect, s: &LibSymbol) {
        paint_symbol(cx, rect, &s.lib.doc, &library_symbols(&s.lib), s.name());
    }
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
    fn drags() -> bool {
        true
    }
    fn menu_tail(_: &mut VectorcraftApp, ui: &mut Ui) {
        Divider::horizontal().show(ui);
        // As a command, so its file dialog, shown off the UI thread, loads the library when it
        // answers.
        MenuItem::command(CommandRef::new("window.symbolLibrary.other")).show(ui);
        // The panel menu's Save Symbol Library…, enabled while the document has symbols.
        MenuItem::command(CommandRef::new(ROW_SAVE_LIBRARY)).show(ui);
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
    library_panel::pick_library_file(app, path, library_panel::library_pick_filters(SymbolLibraries::KIND), load_library)
}

/// The id prefix of the Window → Symbol Libraries → User Defined slots.
pub(crate) const USER_SLOT: &str = "window.userSymbolLibrary";

/// The User Defined library slot `id` (`window.userSymbolLibrary3`) stands for.
pub(crate) fn user_library(app: &VectorcraftApp, id: &str) -> Option<LibraryInfo> {
    library_panel::user_slot(id, USER_SLOT, symbollib::libraries(&app.session))
}

/// Window → Symbol Libraries.
pub(crate) fn window_menu() -> Vec<crate::menus::Node> {
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
    let open = |b: &'static symbollib::BuiltinSymbolLibrary| cp(b.name, SymbolLibraries::OPEN, json!({ "library": b.id }));
    let mut items: Vec<crate::menus::Node> = symbollib::BUILTIN.iter().map(open).collect();
    items.extend([
        Sep,
        sub("User Defined", SLOTS.iter().map(|&id| c("User Library", id)).collect()),
        Sep,
        c("Other Library…", "window.symbolLibrary.other"),
        c("Save Symbols…", "ui.saveSymbolLibrary"),
    ]);
    items
}

// PLAN: decisions.md has the lead split this inline module into `symbols/tests.rs` (`mod tests;`)
// before this set writes; the split is not in this tree, so the module is left as it was for it.
#[cfg(test)]
mod tests;
