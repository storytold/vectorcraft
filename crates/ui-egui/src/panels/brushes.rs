//! Brushes panel: the document's brush library with rendered stroke previews. Clicking a brush
//! applies it to the selected paths (and makes it the Paintbrush's current brush); double-clicking
//! one opens its Brush Options. Shift-click selects a range of brushes, Cmd/Ctrl-click adds or
//! removes one, and the panel menu's Select All Unused selects the brushes no stroke paints with,
//! for Delete Brush and Duplicate Brush. A scatter, art or pattern brush dragged onto an empty part
//! of the canvas places a copy of its art there, and art from the canvas Alt-dragged (Option-dragged
//! on macOS) onto such a brush replaces its art. Brush libraries open in library panels
//! ([`BrushLibraries`]).

use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use craft::behavior::{EditEvent, GesturePhase, IconId, Selection, SelectionChange, SelectionModel as _};
use craft::ui::contract::{
    Bind, DropAt, DropPlace, Entry, ListDrag, MemoProps, PopupKind, Props as _, ReorderMove, SelectMode, Show as _, ShowContainer as _, Sizing,
    Trigger,
};
use craft::ui::display::Divider;
use craft::ui::layout_components::Stack;
use craft::ui::menu::{Menu, MenuItem, PanelRow};
use craft::ui::patterns::presets::{
    CellShape, PresetBrowser, PresetDrop, PresetEvent, PresetFns, PresetReorder, PresetRow, PresetRowMenu, PresetSource, PresetState, PresetView,
};
use craft::ui::primitives::{
    ContentPainterCx, IconSpec, Priority, Revision, ThumbPaint, ThumbRender, ThumbRequest, ThumbSource, ThumbnailOptions, Thumbnails,
};
use craft::ui::runtime::commands::{CommandRef, CommandState, CommandStates};
use craft::ui::sections::{DropAction, FooterAction, FooterBar, FooterOutput};
use craft::ui::style::DataColor;
use craft::ui::tile_grid::GroupHeads;
use egui::{Rect, Ui};
use serde_json::{Value, json};
use vectorcraft_brush::Brush;
use vectorcraft_doc::{AngleProfile, Appearance, Document, Node, PressureProfile, color::Color, color::Paint};
use vectorcraft_engine::DocState;
use vectorcraft_engine::cmd::brushlib::{self, BrushLibrary};
use vectorcraft_engine::cmd::clipboard::{ArtResources, art_resources};
use vectorcraft_engine::cmd::swatchlib::LibraryInfo;

use super::first_node;
use super::library_panel::{self, LibraryKey, LibraryKind, LibraryRef, Row};
use super::memo::Memo;
use crate::VectorcraftApp;
use crate::dialogs::brush_options;
use crate::graphics::doc_image;
use crate::menus::{Sep, c, cp, sub};
use crate::panel_drag::PanelDrag;

const KINDS: [(&str, &str); 5] =
    [("calligraphic", "Calligraphic"), ("scatter", "Scatter"), ("art", "Art"), ("bristle", "Bristle"), ("pattern", "Pattern")];

/// (name, type) of every brush in the active document's library, and the current brush.
pub fn brushes(app: &VectorcraftApp) -> (Vec<(String, String)>, Option<String>) {
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

/// The brushes of `shown` by name: of two with one name, the first, the one a stroke that names
/// it draws with and `brush.get` gives.
fn by_name(shown: &[Shown]) -> HashMap<&str, &Shown> {
    let mut map = HashMap::new();
    for b in shown {
        map.entry(b.name.as_str()).or_insert(b);
    }
    map
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

/// The thumbnail store every stroke preview is kept in: the panel's, the library panels' and Brush Options'.
const THUMBS: &str = "vectorcraft.brushes";

/// A stroke preview in the thumbnail store: the [`preview_key`] of the brush with the resources it renders with, and
/// the cell's width over its height in thousandths.
// PLAN: the contract's `BrushThumb { key }` lacks the cell's shape: a `ThumbRequest` gives only the longest side, and
// the stroke runs across the cell's own shape (a 64:28 tile, Brush Options' wide strip), so the shape is in the key.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct BrushThumb {
    key: u64,
    aspect: u32,
}

impl BrushThumb {
    fn of(key: u64, rect: Rect) -> Self {
        let aspect = (rect.width() / rect.height().max(1.0) * 1000.0).round().clamp(1.0, 64_000.0);
        // `clamp` passes NaN through; the cast makes it 0 and `max` keeps the shape usable.
        Self { key, aspect: (aspect as u32).max(1) }
    }
}

/// Renders the stroke preview of brush definition `def` with the resources of `doc` ([`preview_doc`]).
struct BrushPictures<'a> {
    doc: &'a Document,
    def: &'a Value,
}

impl ThumbSource<BrushThumb> for BrushPictures<'_> {
    fn revision(&self, thumb: &BrushThumb, _: &ThumbRequest) -> Revision {
        Revision::new(thumb.key)
    }

    fn render(&self, thumb: &BrushThumb, request: &ThumbRequest) -> ThumbRender {
        #[cfg(test)]
        counters::RENDERED.with(|r| r.set(r.get() + 1));
        let side = request.side as f32;
        let aspect = thumb.aspect as f32 / 1000.0;
        let (w, h) = if aspect >= 1.0 { (side, side / aspect) } else { (side * aspect, side) };
        match doc_image([w.round().max(1.0) as usize, h.round().max(1.0) as usize], |w, h| preview_doc(self.doc, self.def, w, h)) {
            Some(image) => ThumbRender::Ready(image),
            None => ThumbRender::Blank,
        }
    }
}

/// The stroke preview of brush definition `def` (with the resources of `doc`), stored by `key` ([`preview_key`]), on
/// white paper in `rect`.
fn paint_picture(cx: &ContentPainterCx, rect: Rect, doc: &Document, def: &Value, key: u64) {
    let ctx = cx.painter.ctx();
    let thumbs = Thumbnails::<BrushThumb>::in_context(ctx, THUMBS, ThumbnailOptions::new());
    let request = ThumbRequest::for_cell(rect.width().max(rect.height()), ctx.pixels_per_point());
    let state = thumbs.state(ctx, &BrushPictures { doc, def }, &BrushThumb::of(key, rect), &request, Priority::VISIBLE);
    thumbs.paint(cx, rect, &state, &ThumbPaint::default().paper(DataColor::new(egui::Color32::WHITE)));
}

/// The stroke preview of brush `shown` of `doc` (a document or a brush library's) in `rect`: the panels' rows and
/// tiles.
pub(crate) fn paint_brush(cx: &ContentPainterCx, rect: Rect, doc: &Document, shown: &Shown) {
    paint_picture(cx, rect, doc, &shown.def, shown.key);
}

/// The stroke preview of a brush the fields of Brush Options describe (`def`), with the resources of `doc` (none
/// without a document), in `rect`: its key is worked out again only when the definition or the document changes, not
/// every pass.
pub(crate) fn paint_definition(cx: &ContentPainterCx, rect: Rect, doc: Option<&Document>, def: &Value) {
    type Last = (usize, Value, u64, ArtResources);
    thread_local! {
        static LAST: RefCell<Option<Last>> = const { RefCell::new(None) };
        static BLANK: Document = Document::new(1.0, 1.0);
    }
    BLANK.with(|blank| {
        let doc = doc.unwrap_or(blank);
        // shortcut: the document is known by its address, so one freed and another allocated at that address between
        // two passes keeps the old key until the definition changes; upgrade when the painter can hold the version.
        let at = std::ptr::from_ref(doc).addr();
        let key = LAST.with(|l| l.borrow().as_ref().filter(|l| l.0 == at && l.1 == *def).map(|l| l.2)).unwrap_or_else(|| {
            let brush = Brush::from_json(def.clone()).ok();
            let (key, deps) = preview_key(doc, def, brush.as_ref().map(Brush::art).unwrap_or_default());
            LAST.with(|l| *l.borrow_mut() = Some((at, def.clone(), key, deps)));
            key
        });
        paint_picture(cx, rect, doc, def, key);
    });
}

/// What a stroke preview of brush definition `def`, `w` × `h` pt, renders: a copy of `resources`
/// (a document whose patterns, symbols, swatches and images the brush art may use) without its art,
/// and a stroke painted with the brush across it. The stroke goes from light to heavy pen pressure
/// and back, from an upright pen to one lying flat and back, once round every bearing and barrel
/// rotation, and from one end of an airbrush pen's finger wheel to the other and back, so brushes
/// that vary with the pen show how they vary.
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
    st.tilt = Some(PressureProfile::sample());
    st.bearing = Some(AngleProfile::sample());
    st.rotation = Some(AngleProfile::sample());
    st.wheel = Some(PressureProfile::sample());
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

fn selected_brush(app: &VectorcraftApp) -> Option<String> {
    first_node(app).and_then(|n| n.appearance.stroke().and_then(|s| s.brush.clone()))
}

/// What the panel's last click was made with: the document, its selected objects and the current
/// brush.
pub(crate) type ClickKey = (u64, Vec<u64>, Option<String>);

fn click_key(app: &VectorcraftApp, current: Option<&str>) -> Option<ClickKey> {
    app.session.active().map(|st| (st.uid, st.selection.objects.iter().map(|id| id.0).collect(), current.map(str::to_string)))
}

/// The brushes that Shift-clicks, Cmd/Ctrl-clicks, a secondary click and Select All Unused selected
/// (`app.ui.brush_selection`), of those in `list`. It is `None` after a plain click, once none of its brushes is
/// left, and once another document, other objects or another current brush is active than the click was made with;
/// `Some` of an empty selection after Cmd/Ctrl-clicks deselected every brush or Select All Unused found none.
fn picked(app: &VectorcraftApp, list: &[(String, String)], current: Option<&str>) -> Option<Selection<String>> {
    let (key, sel) = app.ui.brush_selection.as_ref()?;
    if click_key(app, current).as_ref() != Some(key) {
        return None;
    }
    if sel.items.is_empty() {
        return Some(sel.clone());
    }
    let listed = |name: &String| list.iter().any(|b| &b.0 == name);
    let items: Vec<String> = sel.items.iter().filter(|n| listed(n)).cloned().collect();
    (!items.is_empty()).then(|| Selection::new(items, sel.lead.clone().filter(listed), sel.anchor.clone().filter(listed)))
}

/// Stores `sel` as the panel's multi-selection, made with the document, objects and current brush active now.
fn store_selection(app: &mut VectorcraftApp, sel: Selection<String>) {
    let current = brushes(app).1;
    app.ui.brush_selection = click_key(app, current.as_deref()).map(|key| (key, sel));
}

/// The brushes Delete Brush and Duplicate Brush act on, which the panel shows selected: the multi-selection, else
/// the selected path's brush, else the current one.
fn targets(app: &VectorcraftApp, list: &[(String, String)], current: Option<&str>) -> Vec<String> {
    picked(app, list, current).map_or_else(|| selected_brush(app).or(current.map(str::to_string)).into_iter().collect(), |sel| sel.items)
}

/// Runs `brush.delete` or `brush.duplicate` on `names`.
fn run_on(app: &mut VectorcraftApp, id: &str, names: &[String]) {
    let p = match names {
        [name] => json!({ "name": name }),
        _ => json!({ "names": names }),
    };
    app.run(id, p).ok();
}

/// Delete `names` after asking: Yes or No when none of them is in use, else Remove Strokes or
/// Cancel. With `now` (Alt-click on the bottom button), brushes that are not in use go at once.
fn delete(app: &mut VectorcraftApp, names: &[String], now: bool) {
    let Some(used) = crate::dialogs::brush_delete::used(app, names) else {
        return;
    };
    if now && !used {
        run_on(app, "brush.delete", names);
        return;
    }
    let params = match names {
        [name] => json!({ "name": name }),
        _ => json!({ "names": names }),
    };
    crate::dialogs::brush_delete::ask(app, params, used);
}

/// The Brushes panel's view choices for the session (`UiState::brushes_view`).
#[derive(Clone, Debug, Default)]
pub(crate) struct BrushesView {
    /// List View; Thumbnail View when off.
    pub list: bool,
    /// The brush types (`KINDS` ids) the menu hid.
    pub hidden: Vec<String>,
}

/// A row of the Brushes panel: brush `name` of the document `doc` (its uid), or with `group` the rule over the brushes
/// of type `name`. A brush may be named like a type, so the flag keeps a brush's key apart from its group's.
#[derive(Clone, Debug, PartialEq, Eq, Hash, MemoProps)]
pub(crate) struct BrushKey {
    /// The uid of the document whose brush it is: a drag from another document's panel names another brush.
    pub doc: u64,
    /// The brush's name, or the group's type.
    pub name: String,
    /// Whether the row is a type's group.
    pub group: bool,
}

impl BrushKey {
    fn brush(doc: u64, name: &str) -> Self {
        Self { doc, name: name.to_string(), group: false }
    }
}

/// The panel's rows: a group per shown type in [`KINDS`] order, without the `hidden` types, each over its brushes of
/// `shown` in library order. Of two brushes with one name only the first, the one a stroke that names it draws with,
/// has a row, so row keys stay unique.
fn rows(doc: u64, shown: &[Shown], hidden: &[String]) -> Vec<PresetRow<BrushKey>> {
    let first = by_name(shown);
    let mut rows = Vec::new();
    for (ty, label) in KINDS.iter().filter(|(ty, _)| !hidden.iter().any(|h| h == ty)) {
        let of_type: Vec<&Shown> = shown.iter().filter(|b| b.ty == *ty && first.get(b.name.as_str()).is_some_and(|f| std::ptr::eq(*f, *b))).collect();
        if of_type.is_empty() {
            continue;
        }
        rows.push(PresetRow::group(BrushKey { doc, name: ty.to_string(), group: true }, tl!(label), 0));
        rows.extend(of_type.into_iter().map(|b| PresetRow::item(BrushKey::brush(doc, &b.name), b.name.clone(), 1)));
    }
    rows
}

/// What the panel's rows, its reorder rule and its stroke previews read: the brushes of one version of a document,
/// without the `hidden` types.
#[derive(MemoProps)]
struct BrushSource<'a> {
    uid: u64,
    revision: u64,
    #[memo_props(covered_by = "revision", reason = "a document version's brushes and resources are fixed")]
    doc: Option<&'a Document>,
    #[memo_props(covered_by = "revision", reason = "worked out once from the document version")]
    shown: &'a [Shown],
    hidden: Vec<String>,
}

/// The panel's presets: [`rows`]; a brush moves only among those of its type, and paints its stroke preview.
fn brush_source(inputs: BrushSource<'_>) -> PresetSource<'_, BrushKey> {
    let fns = PresetFns::<BrushSource<'_>, BrushKey>::new(|s, _| rows(s.uid, s.shown, &s.hidden))
        .accepts(|s, drag, over, _| {
            let first = by_name(s.shown);
            over.and_then(|over| type_of(&first, over)).is_some_and(|ty| type_of(&first, &drag.grabbed) == Some(ty))
        })
        .thumb(|s, cx, rect, info| {
            // The first brush of a name is the one its row stands for, as `by_name` keeps it.
            if let (Some(doc), false, Some(b)) = (s.doc, info.key.group, s.shown.iter().find(|b| b.name == info.key.name)) {
                paint_brush(cx, rect, doc, b);
            }
        });
    PresetSource::new(inputs, fns)
}

/// The type of the brush or group row `key` (`first`: the brushes by name).
fn type_of<'a>(first: &HashMap<&str, &'a Shown>, key: &'a BrushKey) -> Option<&'a str> {
    if key.group { Some(key.name.as_str()) } else { first.get(key.name.as_str()).map(|b| b.ty) }
}

/// Whether art dropped on brush `ty` can replace its art: scatter, art and pattern brushes have art.
fn takes_art(ty: &str) -> bool {
    matches!(ty, "scatter" | "art" | "pattern")
}

/// The ids of art dragged from the displayed drawing.
fn art_of(payload: &PanelDrag, uid: u64, reverts: u64) -> Option<Vec<u64>> {
    payload.artwork_for(uid, reverts).map(|ids| ids.iter().map(|id| id.0).collect())
}

/// The library and name of a brush dragged out of a brush library panel; a folder or another kind's item is none.
fn library_brush(drag: &ListDrag<LibraryKey>) -> Option<(&str, &str)> {
    let key = &drag.grabbed;
    let (kind, id) = library_panel::parse(&key.panel)?;
    (kind.kind == BrushLibraries::KIND && !key.folder).then_some((id, key.name.as_str()))
}

/// Where the move `mv` of the browser's rows puts its grabbed brush of type `ty`: the index among the brushes of that
/// type (itself included) of the brush it lands before, or past the last of them. `mv.to` counts the rows left once
/// every moved row is out (`first`: the brushes by name).
fn type_index(rows: &[PresetRow<BrushKey>], first: &HashMap<&str, &Shown>, mv: &ReorderMove<BrushKey>, ty: &str) -> usize {
    let of_type = |key: &BrushKey| !key.group && type_of(first, key) == Some(ty);
    let same: Vec<&BrushKey> = rows.iter().map(|row| &row.key).filter(|key| of_type(key)).collect();
    let before = rows.iter().map(|row| &row.key).filter(|key| !mv.keys.contains(key)).nth(mv.to);
    before.filter(|key| of_type(key)).and_then(|key| same.iter().position(|k| *k == key)).unwrap_or(same.len())
}

/// The brushes an item menu row acts on for `target`: the multi-selection when the target is one of several selected,
/// else the target.
fn item_targets(target: &BrushKey, multi: Option<&Selection<String>>) -> Vec<String> {
    match multi {
        Some(sel) if sel.items.len() > 1 && sel.items.contains(&target.name) => sel.items.clone(),
        _ => vec![target.name.clone()],
    }
}

/// The rows of brush `target`'s menu: Brush Options…, Duplicate Brush and Delete Brush, or Duplicate Brushes and
/// Delete Brushes when it is one of several selected (`multi`). Delete is off when it would leave no brush (`list`:
/// the (name, type) of every brush).
fn item_menu(target: &BrushKey, multi: Option<&Selection<String>>, list: &[(String, String)]) -> Vec<Entry<String>> {
    if target.group {
        return Vec::new();
    }
    let targets = item_targets(target, multi);
    let exist = targets.iter().all(|n| list.iter().any(|b| &b.0 == n));
    let can_delete = exist && targets.len() < list.len() && list.len() > 1;
    let row = |command: &str, label: &'static str, on: bool| Entry::new(command.to_string(), label).enabled(on);
    if targets.len() > 1 {
        vec![row(ITEM_DUPLICATE, tl!("Duplicate Brushes"), exist), row(ITEM_DELETE, tl!("Delete Brushes"), can_delete)]
    } else {
        vec![
            row(ITEM_OPTIONS, tl!("Brush Options…"), exist),
            row(ITEM_DUPLICATE, tl!("Duplicate Brush"), exist),
            row(ITEM_DELETE, tl!("Delete Brush"), can_delete),
        ]
    }
}

/// The commands of the item menu's rows.
const ITEM_OPTIONS: &str = "options";
const ITEM_DUPLICATE: &str = "duplicate";
const ITEM_DELETE: &str = "delete";

/// Runs the item menu's row `command` for `target`.
fn item_command(app: &mut VectorcraftApp, target: &BrushKey, command: &str, multi: Option<&Selection<String>>) {
    let targets = item_targets(target, multi);
    let done = match command {
        ITEM_OPTIONS => app.run("ui.brushOptions", json!({ "name": target.name })),
        ITEM_DUPLICATE => match targets.as_slice() {
            [name] => app.run("brush.duplicate", json!({ "name": name })),
            _ => app.run("brush.duplicate", json!({ "names": targets })),
        },
        ITEM_DELETE => {
            delete(app, &targets, true);
            Ok(Value::Null)
        }
        _ => Ok(Value::Null),
    };
    if let Err(e) = done {
        app.status(e);
    }
}

/// The footer's actions, in the order [`FooterBar::actions`] is given them: the first three at the start edge, Delete
/// at the end edge with New before it.
const LIBRARY: usize = 0;
const REMOVE: usize = 1;
const EXPAND: usize = 2;
const DELETE: usize = 3;
const NEW: usize = 4;

/// Which footer actions are on.
#[derive(Clone, Copy)]
struct FooterOn {
    libraries_open: bool,
    brushed: bool,
    document: bool,
    targets: bool,
}

/// The footer: Libraries (its menu), Remove Brush Stroke and Expand Brush Strokes, then New Brush and Delete Brush.
fn footer(app: &mut VectorcraftApp, ui: &mut Ui, on: FooterOn) -> FooterOutput {
    let action = |icon: &'static str, label: &'static str| FooterAction::new(IconSpec::svg(IconId::new(icon)), label);
    let actions = [
        action("library", BrushLibraries::MENU).popup(PopupKind::Menu).pressed(on.libraries_open),
        action("dc-remove-brush", tl!("Remove Brush Stroke")).enabled(on.brushed),
        action("dc-options", tl!("Expand Brush Strokes")).enabled(on.brushed),
        action("trash-2", tl!("Delete Brush")).enabled(on.targets),
        // New Brush asks for the type, then opens that type's Brush Options.
        action("dc-new-item", tl!("New Brush")).enabled(on.document),
    ];
    let out = FooterBar::actions(actions).at_start(EXPAND + 1).show_footer(ui);
    if let Some(opener) = out.responses.get(LIBRARY) {
        let _ = Menu::new(BrushLibraries::MENU)
            .opened_by(opener, Trigger::Press)
            .show(ui, |ui, _| library_panel::library_menu::<BrushLibraries>(app, ui));
    }
    out
}

/// Runs the footer action the user picked; Delete Brush with Alt (Option on macOS) at the release deletes brushes no
/// stroke uses at once.
fn run_footer(app: &mut VectorcraftApp, out: &FooterOutput, targets: &[String]) {
    let done = match out.picked {
        Some(REMOVE) => app.run("brush.remove", json!({})),
        Some(EXPAND) => app.run("object.expandBrush", json!({})),
        Some(NEW) => app.run("ui.newBrush", json!({})),
        Some(DELETE) => {
            delete(app, targets, out.mods.alt);
            Ok(Value::Null)
        }
        _ => Ok(Value::Null),
    };
    if let Err(e) = done {
        app.status(e);
    }
}

/// A plain pick of brush `name` applies it to the selected objects, else makes it the current brush, and ends the
/// multi-selection.
fn pick(app: &mut VectorcraftApp, name: &str) {
    let has_sel = app.session.active().is_some_and(|d| !d.selection.is_empty());
    let id = if has_sel { "brush.apply" } else { "brush.setCurrent" };
    if let Err(e) = app.run(id, json!({ "name": name })) {
        app.status(e);
    }
    app.ui.brush_selection = None;
}

/// The names of the brushes of `sel` of the document `doc`, with its lead and anchor.
fn names_of(sel: &Selection<BrushKey>, doc: u64) -> Selection<String> {
    let name = |key: &BrushKey| (!key.group && key.doc == doc).then(|| key.name.clone());
    Selection::new(sel.items.iter().filter_map(name).collect(), sel.lead.as_ref().and_then(name), sel.anchor.as_ref().and_then(name))
}

/// The keys of the brushes `sel` names in the document `doc`.
fn keys_of(sel: &Selection<String>, doc: u64) -> Selection<BrushKey> {
    let key = |name: &String| BrushKey::brush(doc, name);
    Selection::new(sel.items.iter().map(key).collect(), sel.lead.as_ref().map(key), sel.anchor.as_ref().map(key))
}

/// The Brushes panel: the document's brushes, by type between rules, over the footer's actions. A plain click applies
/// a brush or makes it current, a double click opens its Brush Options, Shift and Cmd/Ctrl clicks select several for
/// Delete and Duplicate, a drag moves a brush among those of its type, art dropped on the list opens New Brush for it
/// (with Alt on a scatter, art or pattern brush it replaces that brush's art), and a library brush dropped on it is
/// added.
pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let (list, current) = brushes(app);
    let doc = app.session.active().map(|st| st.doc.clone());
    let (uid, revision, reverts) = app.session.active().map_or((0, 0, 0), |st| (st.uid, st.revision, st.reverts));
    let memo = app.session.active().map(document_brushes).unwrap_or_default();
    let first = by_name(&memo);
    let multi = picked(app, &list, current.as_deref());
    // Read as `picked` reads it: another document, other objects, another current brush or no brush left ends the
    // selection for good, so selecting the first art again does not bring it back.
    match &multi {
        Some(sel) if app.ui.brush_selection.as_ref().is_some_and(|(_, held)| held == sel) => {}
        Some(sel) => store_selection(app, sel.clone()),
        None => app.ui.brush_selection = None,
    }
    let sel_brush = selected_brush(app);
    let highlighted = sel_brush.clone().or_else(|| current.clone());
    let given = match &multi {
        Some(sel) => keys_of(sel, uid),
        None => highlighted.as_deref().map(|name| Selection::single(BrushKey::brush(uid, name))).unwrap_or_default(),
    };
    let hidden = app.ui.brushes_view.hidden.clone();
    let view = if app.ui.brushes_view.list { PresetView::List } else { PresetView::Grid };
    let targets = targets(app, &list, current.as_deref());
    let on = FooterOn {
        libraries_open: library_panel::any_open::<BrushLibraries>(&app.ui),
        brushed: sel_brush.is_some(),
        document: doc.is_some(),
        targets: !targets.is_empty(),
    };
    let ctx = ui.ctx().clone();
    let (doc, first, multi_ref, list_ref) = (doc.as_deref(), &first, multi.as_ref(), list.as_slice());
    let source = brush_source(BrushSource { uid, revision, doc, shown: &memo, hidden: hidden.clone() });
    let browser = |ui: &mut Ui| {
        let mut state = PresetState::new();
        let ctx = ctx.clone();
        // Art from this document's canvas, released over a brush with art while Alt (Option on macOS) is held, takes
        // its art; without Alt it falls to the list, which opens New Brush. The pick's modifiers are the release's in
        // the pass that drops and the held ones in a rerun pass, which takes no drop.
        let art_onto = PresetDrop::new::<PanelDrag>(DropPlace::Onto, move |payload: &PanelDrag, at: &DropAt<BrushKey>| {
            let DropAt::Onto(key) = at else { return false };
            payload.artwork_for(uid, reverts).is_some()
                && !key.group
                && type_of(first, key).is_some_and(takes_art)
                && craft::ui::runtime::pointer::frame(&ctx).pick_mods.alt
        });
        let empty = format!("{}\n{}", tl!("No brushes"), tl!("Select art and use New Brush to make one."));
        PresetBrowser::new(tl!("Brushes"), Bind::bound(&mut state), &source)
            .group_heads(GroupHeads::Rule)
            .cell_shape(CellShape::ratio(64, 28))
            .view_lock(view)
            .select(SelectMode::Multi)
            .selection(Bind::given(given.clone()))
            .row_menu(PresetRowMenu::Host)
            .row_items(move |key: &BrushKey| item_menu(key, multi_ref, list_ref))
            .reorder(PresetReorder::items())
            .takes_drop(art_onto)
            .empty(empty)
            .show(ui)
            .events
    };
    let mut footed = None;
    let shown = Stack::new().label(tl!("Brushes")).fill().footer(|ui, _| footed = Some(footer(app, ui, on))).show(ui, |ui, _| {
        let library = DropAction::<ListDrag<LibraryKey>>::new(|drag| library_brush(drag).is_some()).sizing(Sizing::Fill).show(ui, |ui, _| {
            let art = DropAction::<PanelDrag>::new(move |payload| payload.artwork_for(uid, reverts).is_some())
                .sizing(Sizing::Fill)
                .show(ui, |ui, _| browser(ui));
            (art.output.dropped, art.inner.unwrap_or_default())
        });
        (library.output.dropped, library.inner.unwrap_or_default())
    });
    let (library_drop, (art_drop, events)) = shown.inner.unwrap_or_default();

    let picked_now = events.iter().any(|event| matches!(event, PresetEvent::Select { .. }));
    let mut sel = given;
    for event in events {
        match event {
            PresetEvent::Select { key, .. } if !key.group => pick(app, &key.name),
            // A plain pick is the `Select` of the same pass; Shift and Cmd/Ctrl picks only select.
            PresetEvent::Selection(change) if !picked_now => {
                sel.apply(change.clone());
                // A secondary press on a brush not selected reports it set alone before its menu opens (a Cmd/Ctrl
                // click toggles instead): it becomes the current brush, and the menu acts on it.
                if let SelectionChange::Set(keys) = &change
                    && let [key] = keys.as_slice()
                    && !key.group
                    && highlighted.as_deref() != Some(key.name.as_str())
                    && let Err(e) = app.run("brush.setCurrent", json!({ "name": key.name }))
                {
                    app.status(e);
                }
                store_selection(app, names_of(&sel, uid));
            }
            PresetEvent::Activate { key, .. } => {
                if let Err(e) = app.run("ui.brushOptions", json!({ "name": key.name })) {
                    app.status(e);
                }
            }
            PresetEvent::Menu { target, command, .. } => item_command(app, &target, &command, multi.as_ref()),
            PresetEvent::Reorder(edit) => move_brush(app, &rows(uid, &memo, &hidden), first, &edit),
            PresetEvent::Dropped { at, payload, .. } => {
                if let (DropAt::Onto(key), Some(ids)) = (&at, payload.get::<PanelDrag>().and_then(|p| art_of(p, uid, reverts))) {
                    crate::dialogs::brush_replace_art::replace(app, &key.name, &ids);
                }
            }
            _ => {}
        }
    }
    // Art dropped on the list opens New Brush for that art, as the New Brush button opens it for the selection.
    if let Some(ids) = art_drop.as_ref().and_then(|p| art_of(p, uid, reverts))
        && let Err(e) = app.run("ui.newBrush", json!({ "ids": ids }))
    {
        app.status(e);
    }
    if let Some((library, name)) = library_drop.as_ref().and_then(library_brush) {
        add(app, json!({"library": library, "name": name}));
    }
    if let Some(out) = footed {
        run_footer(app, &out, &targets);
    }
}

/// `brush.move` for the brush a committed reorder grabbed: one brush moves, among those of its type.
fn move_brush(app: &mut VectorcraftApp, rows: &[PresetRow<BrushKey>], first: &HashMap<&str, &Shown>, edit: &EditEvent<ReorderMove<BrushKey>>) {
    if !matches!(edit.phase(), Some(GesturePhase::Commit) | None) {
        return;
    }
    let Some(mv) = edit.value() else { return };
    let Some(ty) = type_of(first, &mv.key).filter(|_| !mv.key.group) else { return };
    let to = type_index(rows, first, mv, ty);
    if let Err(e) = app.run("brush.move", json!({ "name": mv.key.name, "to": to, "type": ty })) {
        app.status(e);
    }
}

/// The panel menu's own rows, which no catalog command names.
const DUPLICATE: &str = "ui.brushes.duplicate";
const DELETE_BRUSH: &str = "ui.brushes.delete";
const SELECT_UNUSED: &str = "ui.brushes.selectUnused";
/// `ui.brushes.show.<type>`: shows or hides the brushes of a type.
const SHOW: &str = "ui.brushes.show.";
const THUMBNAIL_VIEW: &str = "ui.brushes.thumbnailView";
const LIST_VIEW: &str = "ui.brushes.listView";
const SAVE_LIBRARY: &str = "ui.brushes.saveLibrary";

/// The types the menu's New Calligraphic Brush and New Bristle Brush make at once, without their Brush Options, and
/// those the New … Brush from Selection rows make from the selected art.
const NEW_AT_ONCE: [&str; 2] = ["calligraphic", "bristle"];
const NEW_FROM_SELECTION: [&str; 3] = ["art", "scatter", "pattern"];

fn new_brush(ty: &str) -> CommandRef {
    CommandRef::with_params("brush.new", json!({ "type": ty }))
}

fn show_kind(ty: &str) -> CommandRef {
    CommandRef::new(format!("{SHOW}{ty}"))
}

/// The panel menu's rows, in the panel's order.
pub(crate) fn menu_rows(app: &VectorcraftApp) -> Vec<PanelRow> {
    let row = |id: &str| PanelRow::Command(CommandRef::new(id));
    let mut rows = vec![row("ui.newBrush")];
    rows.extend(NEW_AT_ONCE.into_iter().chain(NEW_FROM_SELECTION).map(|ty| PanelRow::Command(new_brush(ty))));
    rows.extend([
        row(DUPLICATE),
        row(DELETE_BRUSH),
        row("brush.remove"),
        row("object.expandBrush"),
        // On the selected path's brush, else the current one.
        row("ui.brushOptions"),
        // The selected brushed paths' own settings: the brush stays as it is.
        row("ui.brushObjectOptions"),
        PanelRow::Separator,
        // Selects the brushes no stroke paints with, for Delete Brush and Duplicate Brush.
        row(SELECT_UNUSED),
        PanelRow::Separator,
    ]);
    rows.extend(KINDS.iter().map(|(ty, _)| PanelRow::Command(show_kind(ty))));
    rows.extend([
        PanelRow::Separator,
        row(THUMBNAIL_VIEW),
        row(LIST_VIEW),
        PanelRow::Separator,
        PanelRow::Submenu(tl!("Open Brush Library").into(), library_panel::library_rows::<BrushLibraries>(app)),
        row(SAVE_LIBRARY),
    ]);
    rows
}

/// The enabled and checked states of the panel menu's rows.
pub(crate) fn menu_states(app: &VectorcraftApp, states: &mut CommandStates) {
    let (list, current) = brushes(app);
    let document = app.session.active().is_some();
    let has_sel = app.session.active().is_some_and(|d| !d.selection.is_empty());
    let brushed = selected_brush(app).is_some();
    let targets = !targets(app, &list, current.as_deref()).is_empty();
    let on = |on: bool| CommandState::new().enabled(on);
    let new = CommandRef::new("ui.newBrush");
    // Dimmed with no document, saying why as the menus do.
    let reason = (!document).then(|| crate::menus::model::off_reason(app, &new, crate::i18n::t("New Brush…")));
    states.set(new, reason.into_iter().fold(on(document), CommandState::hint));
    for ty in NEW_AT_ONCE {
        states.set(new_brush(ty), on(true));
    }
    for ty in NEW_FROM_SELECTION {
        states.set(new_brush(ty), on(has_sel));
    }
    states.set(CommandRef::new(DUPLICATE), on(targets));
    states.set(CommandRef::new(DELETE_BRUSH), on(targets));
    states.set(CommandRef::new("brush.remove"), on(brushed));
    states.set(CommandRef::new("object.expandBrush"), on(brushed));
    states.set(CommandRef::new("ui.brushOptions"), on(brush_options::available(app)));
    states.set(CommandRef::new("ui.brushObjectOptions"), on(brush_options::object_available(app)));
    states.set(CommandRef::new(SELECT_UNUSED), on(document));
    let view = &app.ui.brushes_view;
    for (ty, _) in KINDS {
        states.set(show_kind(ty), on(true).checked(!view.hidden.iter().any(|h| h == ty)));
    }
    states.set(CommandRef::new(THUMBNAIL_VIEW), on(true).checked(!view.list));
    states.set(CommandRef::new(LIST_VIEW), on(true).checked(view.list));
    states.set(CommandRef::new(SAVE_LIBRARY), on(!list.is_empty()));
}

/// Runs the panel menu's own row `id`; `None` for an id that is not one of them.
pub(crate) fn run(app: &mut VectorcraftApp, id: &str, _params: &Value) -> Option<Result<Value, String>> {
    let (list, current) = brushes(app);
    let targets = targets(app, &list, current.as_deref());
    match id {
        DUPLICATE => run_on(app, "brush.duplicate", &targets),
        DELETE_BRUSH => delete(app, &targets, false),
        SELECT_UNUSED => {
            let found = match app.run("brush.unused", json!({})) {
                Ok(found) => found,
                Err(e) => return Some(Err(e)),
            };
            let names: Vec<String> = serde_json::from_value(found["names"].clone()).unwrap_or_default();
            store_selection(app, Selection::new(names, None, None));
        }
        THUMBNAIL_VIEW | LIST_VIEW => app.ui.brushes_view.list = id == LIST_VIEW,
        SAVE_LIBRARY => save_library(app, targets),
        _ => {
            let ty = id.strip_prefix(SHOW).filter(|ty| KINDS.iter().any(|(kind, _)| kind == ty))?;
            let hidden = &mut app.ui.brushes_view.hidden;
            if hidden.iter().any(|h| h == ty) {
                hidden.retain(|h| h != ty);
            } else {
                hidden.push(ty.to_string());
            }
        }
    }
    Some(Ok(Value::Null))
}

/// Save Brush Library… for the document's brushes (`names`: those selected in the panel).
fn save_library(app: &mut VectorcraftApp, names: Vec<String>) {
    if let Err(e) = app.run("ui.saveBrushLibrary", json!({ "names": names })) {
        app.status(e);
    }
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
    type Thumbs = ();

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
    fn rows<'a>(lib: &'a Self::Lib, _id: &str) -> Vec<Row<'a, LibBrush>> {
        lib.iter().map(|b| Row { name: b.name(), item: Some(b) }).collect()
    }
    /// A built-in library's brush in the UI language; a User Defined or loaded library's brush as
    /// it is named.
    fn label<'a>(id: &str, name: &'a str) -> Cow<'a, str> {
        super::label_or_name(name, brushlib::BUILTIN.iter().any(|b| b.id == id)).into()
    }
    /// The brush's row, named in the UI language; Find matches the shown name and the name as stored.
    fn row(id: &str, b: &LibBrush, label: &str) -> PresetRow<LibraryKey> {
        let key = LibraryKey { panel: library_panel::panel_id(Self::KIND, id), name: b.name().to_string(), folder: false };
        let mut row = PresetRow::item(key, label, 0).tooltip(Self::describe(b));
        row.search_name = Some(b.name().to_string());
        row
    }
    fn thumbs(_: &VectorcraftApp, _: &Self::Lib) {}
    /// The brush's stroke preview, drawn with the library's resources.
    fn thumb(_: &(), cx: &ContentPainterCx, rect: Rect, b: &LibBrush) {
        paint_brush(cx, rect, &b.lib.doc, &b.shown);
    }
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
    fn drags() -> bool {
        true
    }
    fn menu_tail(app: &mut VectorcraftApp, ui: &mut Ui) {
        Divider::horizontal().show(ui);
        // As a command, so its file dialog, shown off the UI thread, loads the library when it
        // answers.
        if MenuItem::new(tl!("Other Library…")).show(ui).ran
            && let Err(e) = app.run("window.brushLibrary.other", json!({}))
        {
            app.status(e);
        }
        if MenuItem::new(tl!("Save Brushes…")).enabled(has_brushes(app)).show(ui).ran {
            let (list, current) = brushes(app);
            let names = targets(app, &list, current.as_deref());
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
    let filters = library_panel::library_pick_filters(BrushLibraries::KIND);
    library_panel::pick_library_file(app, path, filters, load_library)
}

/// The id prefix of the Window → Brush Libraries → User Defined slots.
pub(crate) const USER_SLOT: &str = "window.userBrushLibrary";

/// The User Defined library slot `id` (`window.userBrushLibrary3`) stands for.
pub(crate) fn user_library(app: &VectorcraftApp, id: &str) -> Option<LibraryInfo> {
    library_panel::user_slot(id, USER_SLOT, brushlib::libraries(&app.session))
}

/// Window → Brush Libraries.
pub(crate) fn window_menu() -> Vec<crate::menus::Node> {
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
    let open = |b: &'static brushlib::BuiltinBrushLibrary| cp(b.name, BrushLibraries::OPEN, json!({ "library": b.id }));
    let mut items: Vec<crate::menus::Node> = brushlib::BUILTIN.iter().map(open).collect();
    items.extend([
        Sep,
        sub("User Defined", SLOTS.iter().map(|&id| c("User Library", id)).collect()),
        Sep,
        c("Other Library…", "window.brushLibrary.other"),
        c("Save Brushes…", "ui.saveBrushLibrary"),
    ]);
    items
}

/// Brush definitions serialised and preview keys hashed, and stroke previews rendered, on this test thread.
#[cfg(test)]
pub(crate) mod counters {
    thread_local! {
        pub(crate) static KEYED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
        pub(crate) static RENDERED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    }
}

#[cfg(test)]
mod tests;
