//! The Layers panel: a row for every layer, sublayer, group and object, nested and indented, with
//! the eye (visibility) and lock columns, the layer colour bar, a disclosure triangle, a thumbnail
//! and the name, then the target circle and the selection column; a bottom bar with the panel's
//! buttons and the ≡ menu.
//!
//! - Clicking a row highlights it and makes its layer current (`layer.setCurrent`); Shift-click
//!   highlights the range of rows down to it, Ctrl/Cmd-click toggles one (`layer.highlight`). Panel
//!   commands (Duplicate, Delete, Merge, Options…) act on the highlighted rows.
//! - The selection column selects a row's art (`layer.selectAll`, Shift adds or removes); a row
//!   whose art is selected shows a square in its layer's colour, and dragging that square onto
//!   another row moves the selected art there (Alt copies it).
//! - The eye and lock columns toggle a row (`layer.setProps`); dragging down a column sets every
//!   row it passes to the same state, in one undo step. Ctrl/Cmd-click an eye switches the layer
//!   between Preview and Outline. Alt-click an eye hides the other layers (`layer.hideOthers`), or
//!   shows every layer when they are hidden (`layer.showAll`); Alt-click a lock does the same with
//!   `layer.lockOthers` and `layer.unlockAll`. Ctrl/Cmd+Alt-click an eye switches the other layers
//!   to Outline and the row's layer to Preview (`layer.outlineOthers`), or switches every layer to
//!   Preview when the others are in Outline (`layer.previewAll`).
//! - Dragging rows moves them above, below or into a layer or group (`layer.move`, a drop line or
//!   a box shows where); Alt copies them. A dimmed copy of the row follows the pointer. Dropped on
//!   the trash they are deleted, on New Layer duplicated.
//! - The triangle opens or closes a row; Alt-click does the same to everything inside it.
//! - Double-clicking a name renames it; double-clicking elsewhere on a row opens its options.
//! - A target circle targets its layer, group or object (`layer.target`); dragging it onto another
//!   row's circle moves the appearance there (Alt copies it, `appearance.transfer`) and dropping it
//!   on the trash clears it.
//! - Search All and the Filter button at its right (`ui.layersFilter`, lit while a filter is on)
//!   narrow the rows, and every row that holds others shows open while they do. A row stays when
//!   its name holds the text typed, it is one of the object kinds ticked under By Object and it is
//!   in one of the states ticked under By Layer (a section with nothing ticked takes every row), or
//!   when a row inside it stays. Neither changes the document, its open rows or the selection.
//!
//! While an opacity mask is edited the panel lists only its art, under an `<Opacity Mask>` entry.
//!
//! The rows open are the document's ([`OpenRows`], saved in native files): a document opens with
//! only its top-level layers open. The rows shown are listed once per document change, and only
//! those scrolled into view are laid out, so a document of many thousand objects costs no more
//! per frame than a small one.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Weak};

use egui::{Color32, Sense, Stroke, StrokeKind, Ui, vec2};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use vectorcraft_doc::{Document, LiveShape, Node, NodeId, NodeKind};
use vectorcraft_engine::OpenRows;

use crate::theme::Tokens;
use crate::thumb_worker::{Settle, ThumbWorker};
use crate::widgets::{Live, PanelDrag};
use crate::{VectorcraftApp, icons, widgets};

/// Width of the eye and the lock columns.
const COLUMN: f32 = 25.0;
/// Indent per nesting level.
const INDENT: f32 = 14.0;
/// Right edge offsets of the target circle and the selection square.
const TARGET_X: f32 = 28.0;
const SQUARE_X: f32 = 11.0;

/// Layers panel options (≡ › Panel Options…), kept with the UI state.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PanelOptions {
    /// Show Layers Only: no rows for groups and objects.
    pub layers_only: bool,
    /// Row height in points: [`ROW_SMALL`], [`ROW_MEDIUM`], [`ROW_LARGE`] or another (12–100).
    pub row_size: f32,
    /// Thumbnails on layer, group and object rows.
    pub thumb_layers: bool,
    pub thumb_groups: bool,
    pub thumb_objects: bool,
}

pub const ROW_SMALL: f32 = 20.0;
pub const ROW_MEDIUM: f32 = 26.0;
pub const ROW_LARGE: f32 = 40.0;

impl Default for PanelOptions {
    fn default() -> Self {
        Self { layers_only: false, row_size: ROW_MEDIUM, thumb_layers: true, thumb_groups: true, thumb_objects: true }
    }
}

impl PanelOptions {
    /// The row height, kept to 12–100 points.
    pub fn row(&self) -> f32 {
        if self.row_size.is_finite() { self.row_size.clamp(12.0, 100.0) } else { ROW_MEDIUM }
    }
    /// Whether `n`'s row shows a thumbnail.
    fn thumb(&self, n: &Node) -> bool {
        self.row() >= 18.0
            && match n.kind {
                NodeKind::Layer { .. } => self.thumb_layers,
                NodeKind::Group { .. } => self.thumb_groups,
                _ => self.thumb_objects,
            }
    }
}

/// What a drag in the panel carries.
#[derive(Clone, Debug)]
enum LayersDrag {
    /// Rows dragged to move (or, with Alt, copy) them.
    Rows(Vec<NodeId>),
    /// The selected-art square: the selected objects go to the row it is dropped on.
    Art,
    /// Ruler guide rows (indexes into the document's guides): they go to the layer of the row
    /// they are dropped on.
    Guides(Vec<usize>),
}

/// A drag down the eye or lock column: every row it passes takes `value`.
#[derive(Clone, Debug, Default)]
struct ColumnDrag {
    /// The eye column (visibility), else the lock column.
    eye: bool,
    value: bool,
    ids: Vec<u64>,
}

fn key(name: &str) -> egui::Id {
    egui::Id::new(("layers-panel", name))
}

fn rename_id() -> egui::Id {
    egui::Id::new("layers-rename")
}

/// Ghost rows are drawn at this opacity.
const GHOST_OPACITY: f32 = 0.5;

/// The colour of the layer holding `id`.
fn layer_colour(doc: &Document, id: NodeId) -> Color32 {
    let [r, g, b] = doc.layer_color(id);
    Color32::from_rgb(r, g, b)
}

/// The layer colour bar of row `n` (rect `r`) at `x`: wider on layers.
fn colour_bar(ui: &Ui, n: &Node, r: egui::Rect, x: f32, colour: Color32) {
    let w = if n.is_layer() { 4.0 } else { 2.0 };
    ui.painter().rect_filled(egui::Rect::from_min_size(egui::pos2(x - 1.0, r.top()), vec2(w, r.height())), 0.0, colour);
}

/// The thumbnail of row `n` (rect `r`) at `x`; returns where it went.
fn row_thumb(ui: &Ui, view: &View, n: &Arc<Node>, r: egui::Rect, x: f32) -> egui::Rect {
    let s = (view.h - 4.0).min(40.0);
    let th = egui::Rect::from_min_size(egui::pos2(x, r.center().y - s / 2.0), vec2(s, s));
    ui.painter().rect_filled(th, 0.0, Color32::WHITE);
    ui.painter().rect_stroke(th, 0.0, Stroke::new(1.0, view.t.border), StrokeKind::Outside);
    if !real_thumb(ui, &view.thumbs, n, th) {
        thumb(ui, n, th);
    }
    th
}

/// The dragged row `src`, dimmed, following the pointer at `pos` above everything else: its
/// colour bar, thumbnail and name.
fn drag_ghost(ui: &Ui, view: &View, src: NodeId, pos: egui::Pos2) {
    let (doc, t, h) = (view.doc, &view.t, view.h);
    let Some(n) = node_arc(doc, src) else { return };
    let n = &n;
    let (grab, width, depth) = ui.data(|d| d.get_temp::<(egui::Vec2, f32, usize)>(key("grab"))).unwrap_or((vec2(h, h / 2.0), 240.0, 0));
    // A plain Ui on the tooltip layer, not an Area: a new Area is invisible for a frame and fades in.
    let r = egui::Rect::from_min_size(pos - grab, vec2(width, h));
    let id = key("ghost");
    let mut ui = Ui::new(ui.ctx().clone(), id, egui::UiBuilder::new().layer_id(egui::LayerId::new(egui::Order::Tooltip, id)).max_rect(r));
    ui.set_opacity(GHOST_OPACITY);
    let p = ui.painter();
    p.rect_filled(r, 0.0, t.row_selected);
    p.rect_stroke(r, 0.0, Stroke::new(1.0, t.accent), StrokeKind::Inside);
    let mut x = r.left() + 2.0 * COLUMN + 2.0;
    colour_bar(&ui, n, r, x, layer_colour(doc, src));
    x += 22.0 + depth as f32 * INDENT;
    if view.opts.thumb(n) {
        x = row_thumb(&ui, view, n, r, x).right() + 4.0;
    }
    let label = n.identify_label(view.identify_xml_id);
    let name = if view.identify_xml_id { label } else { painted_name(n, &label, crate::i18n::current()) };
    ui.painter().text(egui::pos2(x, r.center().y), egui::Align2::LEFT_CENTER, name, egui::FontId::proportional(13.0), t.text);
}

/// Where a dragged row goes relative to the row it is dropped on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Place {
    Above,
    Below,
    Inside,
}

impl Place {
    fn id(self) -> &'static str {
        match self {
            Place::Above => "above",
            Place::Below => "below",
            Place::Inside => "inside",
        }
    }
}

/// The name painted for a row in `lang`: a generated `<Kind>` name is translated, anything else is
/// user data (an unnamed text object shows its text, which can look like `<Path>`; only an empty
/// one is called `<Text>`).
fn painted_name(n: &Node, name: &str, lang: crate::i18n::Lang) -> String {
    let generated = n.name.is_none() && !matches!(&n.kind, NodeKind::Text(t) if t.runs.iter().any(|r| !r.text.is_empty()));
    if generated && let Some(inner) = name.strip_prefix('<').and_then(|s| s.strip_suffix('>')) {
        return format!("<{}>", crate::i18n::tr(lang, inner));
    }
    name.to_string()
}

/// The name the panel shows for `n` in `lang`: its XML ID while Identify Objects By is XML ID,
/// otherwise [`painted_name`].
pub(crate) fn shown_name(app: &VectorcraftApp, n: &Node, lang: crate::i18n::Lang) -> String {
    let xml_id = app.session.prefs.identify_objects_by == "xmlId";
    let label = n.identify_label(xml_id);
    if xml_id { label } else { painted_name(n, &label, lang) }
}

/// Whether `n` gets rows for what it holds (a compound path's paths are one object).
fn opens(n: &Node, opts: &PanelOptions) -> bool {
    match n.kind {
        NodeKind::Compound { .. } => false,
        _ => n.children().is_some_and(|c| c.iter().any(|c| !opts.layers_only || c.is_layer())),
    }
}

/// One row of the panel.
#[derive(Clone)]
struct Row {
    node: Arc<Node>,
    /// Nesting level (0: a top-level layer).
    depth: usize,
    /// A clipping path: its group or layer clips.
    clip_path: bool,
    /// It holds rows ([`opens`]), and shows them.
    opens: bool,
    open: bool,
    /// The colour of its layer (its own, for a layer).
    colour: Color32,
    /// Drawn dimmed: hidden itself or in a hidden layer or group, or a template layer.
    dim: bool,
    /// A ruler guide on the layer `node` (its index in the document's guides): the guide's row.
    guide: Option<usize>,
}

/// An object kind under the Filter's By Object (`ui.layersFilter`'s `objects`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObjectKind {
    Paths,
    Shapes,
    Text,
    Images,
    Effects,
    Groups,
    LinkedFiles,
    Guides,
}

impl ObjectKind {
    /// Every kind with its parameter name, in the popup's order.
    pub const ALL: [(Self, &'static str); 8] = [
        (Self::Paths, "paths"),
        (Self::Shapes, "shapes"),
        (Self::Text, "text"),
        (Self::Images, "images"),
        (Self::Effects, "effects"),
        (Self::Groups, "groups"),
        (Self::LinkedFiles, "linkedFiles"),
        (Self::Guides, "guides"),
    ];

    fn label(self) -> &'static str {
        match self {
            Self::Paths => tl!("Paths"),
            Self::Shapes => tl!("Shapes"),
            Self::Text => tl!("Text"),
            Self::Images => tl!("Images"),
            Self::Effects => tl!("Effects"),
            Self::Groups => tl!("Groups"),
            Self::LinkedFiles => tl!("Linked Files"),
            Self::Guides => tl!("Guides"),
        }
    }

    /// Whether `n`'s row is of this kind. Paths take plain paths, the paths whose corners Live
    /// Corners cut and compound paths; Shapes take live rectangles, ellipses, polygons and lines;
    /// Paths and Shapes leave out paths made into guides. Effects take a row with an effect, shown
    /// or hidden, on itself or on one of its fills or strokes. Linked Files take linked images and
    /// placed documents.
    fn takes(self, n: &Node) -> bool {
        match (self, &n.kind) {
            (Self::Guides, NodeKind::Path { guide, .. }) => *guide,
            (Self::Paths | Self::Shapes, NodeKind::Path { guide: true, .. }) => false,
            (Self::Paths, NodeKind::Path { live, .. }) => live.as_deref().is_none_or(|l| matches!(l, LiveShape::Path { .. })),
            (Self::Shapes, NodeKind::Path { live, .. }) => live.as_deref().is_some_and(|l| !matches!(l, LiveShape::Path { .. })),
            (Self::Paths, NodeKind::Compound { .. })
            | (Self::Text, NodeKind::Text(_))
            | (Self::Images, NodeKind::Image(_))
            | (Self::Groups, NodeKind::Group { .. })
            | (Self::LinkedFiles, NodeKind::PlacedDocument(_)) => true,
            (Self::LinkedFiles, NodeKind::Image(im)) => im.link.is_some(),
            (Self::Effects, _) => !n.appearance.effects.is_empty() || n.appearance.items.iter().any(|i| !i.effects().is_empty()),
            _ => false,
        }
    }
}

/// A row state under the Filter's By Layer (`ui.layersFilter`'s `layers`): the row's own lock or
/// eye, as its lock and eye columns show them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowState {
    Locked,
    Hidden,
}

impl RowState {
    /// Every state with its parameter name, in the popup's order.
    pub const ALL: [(Self, &'static str); 2] = [(Self::Locked, "locked"), (Self::Hidden, "hidden")];

    fn label(self) -> &'static str {
        match self {
            Self::Locked => tl!("Locked"),
            Self::Hidden => tl!("Hidden"),
        }
    }

    fn takes(self, n: &Node) -> bool {
        match self {
            Self::Locked => n.locked,
            Self::Hidden => !n.visible,
        }
    }
}

/// The Layers panel's Filter (`ui.layersFilter`): the object kinds ticked under By Object and the
/// row states ticked under By Layer, each in the popup's order. A section with nothing ticked takes
/// every row.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Filter {
    pub objects: Vec<ObjectKind>,
    pub layers: Vec<RowState>,
}

impl Filter {
    /// Whether anything is ticked.
    pub fn is_on(&self) -> bool {
        !self.objects.is_empty() || !self.layers.is_empty()
    }

    /// Whether `n`'s row is of a ticked kind and in a ticked state.
    fn keeps(&self, n: &Node) -> bool {
        (self.objects.is_empty() || self.objects.iter().any(|k| k.takes(n))) && (self.layers.is_empty() || self.layers.iter().any(|s| s.takes(n)))
    }

    /// Whether the ruler guide rows stay: with no filter on, or with Guides ticked and no row state
    /// ticked.
    fn keeps_guides(&self) -> bool {
        !self.is_on() || (self.objects.contains(&ObjectKind::Guides) && self.layers.is_empty())
    }
}

/// The parameter names of the ones of `all` in `on`, in `all`'s order.
fn names<T: Copy + PartialEq>(all: &[(T, &'static str)], on: &[T]) -> Vec<&'static str> {
    all.iter().filter(|(t, _)| on.contains(t)).map(|(_, name)| *name).collect()
}

/// The parameter names of `on` with `t` added, or taken out when it is in `on`.
fn toggled<T: Copy + PartialEq>(all: &[(T, &'static str)], on: &[T], t: T) -> Vec<&'static str> {
    all.iter().filter(|(x, _)| if *x == t { !on.contains(x) } else { on.contains(x) }).map(|(_, name)| *name).collect()
}

/// The list `key` of `p` read by the parameter names of `all`, in `all`'s order: `None` when `p`
/// leaves it out or sets it to null, and an error when it is anything other than an array of
/// those names.
fn parse_names<T: Copy + PartialEq>(p: &Value, key: &str, all: &[(T, &'static str)]) -> Result<Option<Vec<T>>, String> {
    let given = match p.get(key) {
        None | Some(Value::Null) => return Ok(None),
        Some(v) => v
            .as_array()
            .and_then(|a| a.iter().map(|name| all.iter().find(|(_, n)| Some(*n) == name.as_str()).map(|(t, _)| *t)).collect::<Option<Vec<T>>>()),
    };
    let Some(given) = given else {
        let names: Vec<&str> = all.iter().map(|(_, n)| *n).collect();
        return Err(format!("`{key}` must be an array of these names: {}", names.join(", ")));
    };
    Ok(Some(all.iter().map(|(t, _)| *t).filter(|t| given.contains(t)).collect()))
}

/// What the rows were listed from: listed again when any of it changes.
#[derive(Clone, PartialEq)]
struct RowsKey {
    uid: u64,
    revision: u64,
    open: u64,
    layers_only: bool,
    query: String,
    filter: Filter,
}

/// The rows the panel shows, top down: the layers (only the opacity mask's while one is edited)
/// with the rows of the open ones inside them. A search or a filter shows every row whose name
/// contains `query` (lowercase) and that `filter` keeps, and every row holding one.
fn list_rows(doc: &Document, open: &OpenRows, opts: &PanelOptions, query: &str, filter: &Filter) -> Vec<Row> {
    let mask_layer = doc.mask_edit.map(|m| m.layer);
    let mut out = vec![];
    for l in doc.layers.iter().rev().filter(|l| mask_layer.is_none_or(|m| m == l.id)) {
        push_rows(l, 0, false, (Color32::PLACEHOLDER, true), (open, opts, query, filter, doc), &mut out);
    }
    out
}

/// Add the rows of `n` and those open inside it to `out`; `(colour, shown)` are those of what holds
/// it (the layer's colour, and whether it is visible).
fn push_rows(
    n: &Arc<Node>,
    depth: usize,
    clip_path: bool,
    (colour, shown): (Color32, bool),
    cx: (&OpenRows, &PanelOptions, &str, &Filter, &Document),
    out: &mut Vec<Row>,
) {
    let (open_rows, opts, query, filter, doc) = cx;
    if opts.layers_only && !n.is_layer() {
        return;
    }
    let colour = match &n.kind {
        NodeKind::Layer { color, .. } => {
            let [r, g, b] = color.rgb();
            Color32::from_rgb(r, g, b)
        }
        _ => colour,
    };
    let shown = shown && n.visible;
    let searching = !query.is_empty();
    let narrowing = searching || filter.is_on();
    // A layer's ruler guides are rows in it, above its art (not while searching, nor while the
    // filter leaves them out).
    let guides: Vec<usize> =
        if n.is_layer() && !opts.layers_only && !searching && filter.keeps_guides() { doc.guides_on(n.id).map(|(i, _)| i).collect() } else { vec![] };
    let opens = opens(n, opts) || !guides.is_empty();
    let open = opens && (narrowing || open_rows.contains(n.id));
    let at = out.len();
    out.push(Row { node: n.clone(), depth, clip_path, opens, open, colour, dim: !shown || n.is_template(), guide: None });
    if open {
        for i in guides.into_iter().rev() {
            out.push(Row { node: n.clone(), depth: depth + 1, clip_path: false, opens: false, open: false, colour, dim: !shown, guide: Some(i) });
        }
    }
    if open && let Some(children) = n.children() {
        for (i, c) in children.iter().enumerate().rev() {
            push_rows(c, depth + 1, i == 0 && n.clips(), (colour, shown), cx, out);
        }
    }
    // Searching or filtering, a row stays for a match of its own (its name holds the search text
    // and the filter keeps it) or for a row it keeps inside it.
    if narrowing && out.len() == at + 1 && !(filter.keeps(n) && (!searching || n.display_name().to_lowercase().contains(query))) {
        out.truncate(at);
    }
}

/// The rows of the active document as [`list_rows`] lists them, listed again only when the
/// document, its open rows, the search, the filter or Show Layers Only changed.
fn rows_of_doc(ui: &Ui, st: &vectorcraft_engine::DocState, opts: &PanelOptions, query: &str, filter: &Filter) -> Arc<Vec<Row>> {
    let now = RowsKey {
        uid: st.uid,
        revision: st.revision,
        open: st.layers_open.generation(),
        layers_only: opts.layers_only,
        query: query.to_string(),
        filter: filter.clone(),
    };
    let id = key("rows");
    if let Some((was, rows)) = ui.data(|d| d.get_temp::<(RowsKey, Arc<Vec<Row>>)>(id))
        && was == now
    {
        return rows;
    }
    let rows = Arc::new(list_rows(&st.doc, &st.layers_open, opts, query, filter));
    ui.data_mut(|d| d.insert_temp(id, (now, rows.clone())));
    rows
}

/// Everything one frame of the panel reads.
struct View<'a> {
    doc: &'a Arc<Document>,
    /// Where the row thumbnails come from.
    thumbs: ThumbSource<'a>,
    sel: HashSet<NodeId>,
    /// The selected ruler guides.
    guides: Vec<usize>,
    target: Option<NodeId>,
    current: Option<NodeId>,
    rows: Vec<NodeId>,
    opts: PanelOptions,
    t: Tokens,
    h: f32,
    column: Option<ColumnDrag>,
    /// The row to scroll into view.
    reveal: Option<NodeId>,
    /// Preferences › Units › Identify Objects By › XML ID.
    identify_xml_id: bool,
}

/// What one frame of the panel decided.
#[derive(Default)]
struct Out {
    actions: Vec<(String, Value)>,
    /// A row clicked with these modifiers (resolved once every row is listed).
    click: Option<(NodeId, egui::Modifiers)>,
    /// The column drag after this frame (`Some(None)`: it ended).
    column: Option<Option<ColumnDrag>>,
    /// Rows to open or close, and whether to open them.
    toggle: Vec<(NodeId, bool)>,
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let Some(st) = app.session.active() else { return };
    // A rename belongs to the document it started in (node ids are per document): drop it when
    // another document shows, so it can't rename that document's node with the same id.
    let shown_doc = egui::Id::new("layers-doc");
    let before = ui.data(|d| d.get_temp::<u64>(shown_doc));
    if before != Some(st.uid) {
        ui.data_mut(|d| {
            if before.is_some() {
                d.remove::<(u64, String)>(rename_id());
            }
            d.insert_temp(shown_doc, st.uid);
        });
    }
    let doc = st.doc.clone();
    let rows = st.highlighted_rows();
    let current = st.current_layer();
    // Rows highlighted anew (a click, Locate Object, a new layer): open the layers around them.
    let seen: Vec<NodeId> = ui.data(|d| d.get_temp(key("seen"))).unwrap_or_default();
    let mut reveal = None;
    if rows != seen {
        let around: Vec<NodeId> =
            rows.iter().flat_map(|r| doc.ancestry(*r).unwrap_or_default().split_last().map(|(_, a)| a.to_vec()).unwrap_or_default()).collect();
        if let Some(st) = app.session.active_mut() {
            for a in around {
                st.layers_open.set(a, true);
            }
        }
        reveal = rows.first().copied();
        ui.data_mut(|d| d.insert_temp(key("seen"), rows.clone()));
    }
    // A new active layer (a canvas selection highlights no row): reveal it too, unless a row
    // highlighted in the panel holds the view.
    let seen_current: Option<NodeId> = ui.data(|d| d.get_temp(key("seen-current"))).flatten();
    if current != seen_current {
        if rows.is_empty()
            && let Some(layer) = current
        {
            let around = doc.ancestry(layer).unwrap_or_default().split_last().map(|(_, a)| a.to_vec()).unwrap_or_default();
            if let Some(st) = app.session.active_mut() {
                for a in around {
                    st.layers_open.set(a, true);
                }
            }
            reveal = Some(layer);
        }
        ui.data_mut(|d| d.insert_temp(key("seen-current"), current));
    }
    // Search All, and the Filter button at its right.
    let query = ui
        .horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                filter_button(app, ui);
                crate::widgets::search_field(ui, egui::Id::new("layers-search"), tl!("Search All"))
            })
            .inner
        })
        .inner
        .trim()
        .to_lowercase();
    let Some(st) = app.session.active() else { return };
    let list = rows_of_doc(ui, st, &app.ui.layers_panel, &query, &app.ui.layers_filter);
    let view = View {
        doc: &doc,
        thumbs: ThumbSource::of(app, st, &doc),
        sel: st.selection.objects.iter().copied().collect(),
        guides: st.selection.guides.clone(),
        target: st.selection.target,
        current: st.current_layer(),
        rows,
        opts: app.ui.layers_panel.clone(),
        t,
        h: app.ui.layers_panel.row(),
        column: ui.data(|d| d.get_temp(key("column"))),
        reveal,
        identify_xml_id: app.session.prefs.identify_objects_by == "xmlId",
    };
    let mut out = Out::default();
    ui.add_space(6.0);
    let list_h = ui.available_height() - 34.0;
    // Only the rows in view are laid out; one revealed out of view is scrolled to.
    let revealed = reveal.and_then(|id| list.iter().position(|r| r.node.id == id));
    let scrolled = ui.scope(|ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        egui::ScrollArea::vertical().max_height(list_h).auto_shrink([false, false]).show_rows(ui, view.h, list.len(), |ui, range| {
            if let Some(i) = revealed.filter(|i| !range.contains(i)) {
                let top = ui.max_rect().top() + (i as f32 - range.start as f32) * view.h;
                ui.scroll_to_rect(egui::Rect::from_min_size(egui::pos2(ui.max_rect().left(), top), vec2(ui.max_rect().width(), view.h)), None);
            }
            for r in list.get(range).unwrap_or_default() {
                row(ui, &view, r, &mut out);
            }
        })
    });
    let list_rect = scrolled.inner.inner_rect;
    resolve_click(ui, &view, &list, &mut out);
    if let Some(st) = app.session.active_mut() {
        for (id, open) in out.toggle.drain(..) {
            st.layers_open.set(id, open);
        }
    }
    // The column drag: live while it lasts, one undo step when the button is released.
    let released = ui.input(|i| i.pointer.any_released());
    let changed = out.column.is_some();
    let column = out.column.take().unwrap_or_else(|| view.column.clone());
    if let Some(c) = column.as_ref().filter(|_| changed || released) {
        let (prop, label) = if c.eye { ("visible", "Visibility") } else { ("locked", "Lock") };
        let phase = if released { Live::Released } else { Live::Dragging };
        let params = json!({"ids": c.ids, prop: c.value});
        super::live_run(app, label, "layer.setProps", params, phase);
    }
    ui.data_mut(|d| match column.filter(|_| !released) {
        Some(c) => {
            d.insert_temp(key("column"), c);
        }
        None => d.remove::<ColumnDrag>(key("column")),
    });
    if released {
        ui.data_mut(|d| d.remove::<u64>(egui::Id::new("layers-drag")));
    }
    // Dragged rows show as a dimmed row under the pointer.
    if !released
        && let Some(drag) = egui::DragAndDrop::payload::<LayersDrag>(ui.ctx())
        && let LayersDrag::Rows(ids) = &*drag
        && let Some(src) = ids.first()
        && let Some(pos) = ui.input(|i| i.pointer.latest_pos())
    {
        drag_ghost(ui, &view, *src, pos);
    }
    // Keyboard: Delete removes the highlighted rows while the panel has the keyboard (a row was
    // clicked last, not the canvas).
    let pressed_outside = ui.input(|i| i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|p| !list_rect.contains(p)));
    if pressed_outside {
        ui.data_mut(|d| d.insert_temp(key("focus"), false));
    }
    let focus: bool = ui.data(|d| d.get_temp(key("focus"))).unwrap_or(false);
    if focus
        && !view.rows.is_empty()
        && ui.ctx().memory(|m| m.focused().is_none())
        && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Delete) || i.consume_key(egui::Modifiers::NONE, egui::Key::Backspace))
    {
        out.actions.push(("layer.delete".into(), json!({})));
    }
    bottom_bar(app, ui, &view, &doc, &mut out.actions);
    for (c, p) in out.actions {
        app.run(&c, p).ok();
    }
}

/// The Filter button, lit while a filter is on, and its popup: Clear All, the object kinds under
/// By Object and the row states under By Layer, each ticked or not. A tick, or Clear All, runs
/// `ui.layersFilter`; the popup stays open until a click outside it or Escape.
fn filter_button(app: &mut VectorcraftApp, ui: &mut Ui) {
    let f = app.ui.layers_filter.clone();
    let button = widgets::icon_button(ui, "dc-filter", tl!("Filter"), f.is_on(), 24.0);
    let mut set = None;
    egui::Popup::menu(&button).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
        ui.horizontal(|ui| {
            widgets::subheader(ui, tl!("Filter"));
            ui.add_space(12.0);
            if ui.add_enabled(f.is_on(), egui::Button::new(egui::RichText::new(tl!("Clear All")).size(12.5)).frame(false)).clicked() {
                set = Some(json!({"objects": [], "layers": []}));
            }
        });
        widgets::subheader(ui, tl!("By Object"));
        for (k, _) in ObjectKind::ALL {
            if widgets::check(ui, k.label(), f.objects.contains(&k), true) {
                set = Some(json!({"objects": toggled(&ObjectKind::ALL, &f.objects, k)}));
            }
        }
        widgets::subheader(ui, tl!("By Layer"));
        for (s, _) in RowState::ALL {
            if widgets::check(ui, s.label(), f.layers.contains(&s), true) {
                set = Some(json!({"layers": toggled(&RowState::ALL, &f.layers, s)}));
            }
        }
    });
    if let Some(p) = set {
        app.run("ui.layersFilter", p).ok();
    }
}

/// The bottom bar: the layer count and the panel's buttons.
fn bottom_bar(app: &mut VectorcraftApp, ui: &mut Ui, view: &View, doc: &Document, actions: &mut Vec<(String, Value)>) {
    let t = &view.t;
    let (bar, _) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::hover());
    ui.painter().line_segment([bar.left_top(), bar.right_top()], Stroke::new(1.0, t.divider));
    let n = doc.layers.len();
    ui.painter().text(
        bar.left_center() + vec2(4.0, 0.0),
        egui::Align2::LEFT_CENTER,
        crate::i18n::tn(n as u64, "{n} Layer", "{n} Layers"),
        egui::FontId::proportional(11.5),
        t.text_dim,
    );
    let m = ui.input(|i| i.modifiers);
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(bar).layout(egui::Layout::right_to_left(egui::Align::Center)));
    let trash = widgets::icon_button(&mut child, "trash-2", tl!("Delete Selection"), false, 24.0);
    // A target circle dropped on the trash clears that appearance (dragged rows: below). Peek at
    // the payload's type before taking it: egui's take drops a payload of another type.
    if trash.dnd_hover_payload::<PanelDrag>().is_some()
        && let Some(d) = trash.dnd_release_payload::<PanelDrag>()
        && let PanelDrag::Appearance(id) = *d
    {
        actions.push(("appearance.clear".into(), json!({"ids": [id.0]})));
    } else if trash.clicked() {
        if view.rows.is_empty() && app.session.active().is_some_and(|d| !d.selection.is_empty()) {
            actions.push(("edit.clear".into(), json!({})));
        } else {
            actions.push(("layer.delete".into(), json!({})));
        }
    }
    // Create New Layer: Alt asks for its options first, Ctrl/Cmd puts it on top of every layer.
    let new_layer = widgets::icon_button(&mut child, "file-plus", tl!("Create New Layer"), false, 24.0);
    if new_layer.clicked() {
        if m.alt {
            actions.push(("ui.newLayer".into(), json!({})));
        } else {
            actions.push(("layer.new".into(), json!({"top": m.command})));
        }
    }
    // Rows dropped on the trash are deleted, on New Layer duplicated, as in Illustrator. Both are
    // outlined while rows (or, on the trash, a target circle) hang over them.
    for (button, cmd) in [(&trash, "layer.delete"), (&new_layer, "layer.duplicate")] {
        if let Some(d) = button.dnd_release_payload::<LayersDrag>()
            && let LayersDrag::Rows(ids) = &*d
        {
            actions.push((cmd.into(), json!({"ids": ids.iter().map(|i| i.0).collect::<Vec<_>>()})));
        }
        let over = button.dnd_hover_payload::<LayersDrag>().is_some_and(|d| matches!(*d, LayersDrag::Rows(_)))
            || (cmd == "layer.delete" && button.dnd_hover_payload::<PanelDrag>().is_some_and(|d| matches!(*d, PanelDrag::Appearance(_))));
        if over {
            child.painter().rect_stroke(button.rect, 3.0, Stroke::new(1.5, t.accent), StrokeKind::Inside);
        }
    }
    if widgets::icon_button(&mut child, "plus", tl!("Create New Sublayer"), false, 24.0).clicked() {
        actions.push(if m.alt { ("ui.newLayer".into(), json!({"sublayer": true})) } else { ("layer.newSublayer".into(), json!({})) });
    }
    if widgets::icon_button(&mut child, "frame", tl!("Make/Release Clipping Mask"), false, 24.0).clicked() {
        actions.push(("layer.clippingMask.toggle".into(), json!({})));
    }
    if widgets::icon_button(&mut child, "search", tl!("Locate Object"), false, 24.0).clicked() {
        actions.push(("layer.locate".into(), json!({})));
    }
}

/// Resolve the row clicked this frame: plain (highlight it), Ctrl/Cmd (toggle it) or Shift
/// (highlight the rows from the last one clicked down to it).
fn resolve_click(ui: &Ui, view: &View, list: &[Row], out: &mut Out) {
    let Some((id, m)) = out.click.take() else { return };
    let anchor: Option<NodeId> = ui.data(|d| d.get_temp(key("anchor")));
    ui.data_mut(|d| d.insert_temp(key("focus"), true));
    let at = |id: NodeId| list.iter().position(|r| r.node.id == id);
    if m.shift
        && let Some(a) = anchor.or(view.rows.last().copied())
        && let (Some(i), Some(j)) = (at(a), at(id))
    {
        let range: Vec<u64> = list.get(i.min(j)..=i.max(j)).unwrap_or_default().iter().map(|r| r.node.id.0).collect();
        out.actions.push(("layer.highlight".into(), json!({"ids": range, "mode": if m.command { "add" } else { "set" }})));
        return;
    }
    ui.data_mut(|d| d.insert_temp(key("anchor"), id));
    if m.command {
        out.actions.push(("layer.highlight".into(), json!({"ids": [id.0], "mode": "toggle"})));
    } else {
        out.actions.push(("layer.setCurrent".into(), json!({"id": id.0})));
    }
}

/// Whether row `n` can take `drag` inside it.
fn takes(n: &Node, drag: &LayersDrag, doc: &Document) -> bool {
    match n.kind {
        NodeKind::Layer { .. } => true,
        NodeKind::Group { .. } => match drag {
            LayersDrag::Rows(ids) => !ids.iter().any(|i| doc.node(*i).is_some_and(Node::is_layer)),
            LayersDrag::Art => true,
            LayersDrag::Guides(_) => false,
        },
        _ => false,
    }
}

/// The row of ruler guide `i` on the layer of `item`: a guide mark and its name, the selection
/// square when it is selected. A click selects the guide; Shift or Cmd adds it to (or takes it
/// from) the selected guides. A guide on a hidden or locked layer can't be selected or dragged.
fn guide_row(ui: &mut Ui, view: &View, item: &Row, i: usize, out: &mut Out) {
    let (t, h) = (&view.t, view.h);
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), h), Sense::click_and_drag());
    let Some(g) = view.doc.guides.get(i) else { return };
    let selected = view.guides.contains(&i);
    let editable = view.doc.guide_editable(g);
    if editable && resp.hovered() {
        ui.painter().rect_filled(r, 0.0, t.hover.gamma_multiply(0.6));
    }
    ui.painter().line_segment([r.left_bottom(), r.right_bottom()], Stroke::new(1.0, t.input_border));
    let mut x = r.left() + 2.0 * COLUMN + 2.0;
    colour_bar(ui, &item.node, r, x, item.colour);
    x += 6.0 + item.depth as f32 * INDENT + 16.0;
    // The guide mark: a short line across or down, in the guides' colour.
    let c = egui::pos2(x + 8.0, r.center().y);
    let half = if g.vertical { vec2(0.0, 6.0) } else { vec2(6.0, 0.0) };
    ui.painter().line_segment([c - half, c + half], Stroke::new(1.5, t.accent));
    x += 22.0;
    let name = format!("<{}>", if g.vertical { tl!("Vertical Guide") } else { tl!("Horizontal Guide") });
    let color = if item.dim { t.text_dim } else { t.text };
    ui.painter().text(egui::pos2(x, r.center().y), egui::Align2::LEFT_CENTER, name, egui::FontId::proportional(13.0), color);
    let col_x = r.right() - 43.5;
    ui.painter().line_segment([egui::pos2(col_x, r.top()), egui::pos2(col_x, r.bottom())], Stroke::new(1.0, t.input_border));
    if selected {
        let q = egui::Rect::from_center_size(egui::pos2(r.right() - SQUARE_X, r.center().y), vec2(7.0, 7.0));
        ui.painter().rect_filled(q, 0.0, item.colour);
        ui.painter().rect_stroke(q, 0.0, Stroke::new(1.0, t.text), StrokeKind::Inside);
    }
    if editable && resp.clicked() {
        let m = ui.input(|i| i.modifiers);
        out.actions.push(("guide.select".into(), json!({"indexes": [i], "toggle": m.shift || m.command})));
    }
    // Dragged: the selected guides when this is one of them, else this one, to another layer's row.
    if editable && resp.drag_started() {
        let guides = if selected { view.guides.clone() } else { vec![i] };
        egui::DragAndDrop::set_payload(ui.ctx(), LayersDrag::Guides(guides));
    }
    if resp.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
    }
    // Guides dropped here go to this guide's layer.
    if let Some(drag) = resp.dnd_hover_payload::<LayersDrag>()
        && let LayersDrag::Guides(guides) = &*drag
    {
        guide_drop(ui, view, guides, item.node.id, r, &resp, out);
    }
}

/// Guide rows `guides` dragged over row `r` (`resp`) of layer `layer`: the row lights up, and the
/// release puts them on that layer (`guide.setLayer`, one undo step).
fn guide_drop(ui: &Ui, view: &View, guides: &[usize], layer: NodeId, r: egui::Rect, resp: &egui::Response, out: &mut Out) {
    ui.painter().rect_stroke(r.shrink(1.0), 0.0, Stroke::new(2.0, view.t.accent), StrokeKind::Inside);
    if resp.dnd_release_payload::<LayersDrag>().is_some() {
        out.actions.push(("guide.setLayer".into(), json!({"indexes": guides, "layer": layer.0})));
    }
}

/// The widgets of row `item`.
fn row(ui: &mut Ui, view: &View, item: &Row, out: &mut Out) {
    if let Some(i) = item.guide {
        return guide_row(ui, view, item, i, out);
    }
    let n = &*item.node;
    let (depth, clip_path, has_children, open, color) = (item.depth, item.clip_path, item.opens, item.open, item.colour);
    let doc = view.doc;
    let t = &view.t;
    let h = view.h;
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), h), Sense::click_and_drag());
    if view.reveal == Some(n.id) {
        ui.scroll_to_rect(r, None);
    }
    let is_sel = view.sel.contains(&n.id);
    let child_sel = !is_sel
        && n.children().is_some_and(|_| {
            let mut any = false;
            n.walk(&mut |c| any |= c.id != n.id && view.sel.contains(&c.id));
            any
        });
    // Highlighted rows (else the current layer's).
    let highlighted = if view.rows.is_empty() { n.is_layer() && Some(n.id) == view.current } else { view.rows.contains(&n.id) };
    if highlighted {
        ui.painter().rect_filled(r, 0.0, t.row_selected);
    } else if resp.hovered() {
        ui.painter().rect_filled(r, 0.0, t.hover.gamma_multiply(0.6));
    }
    ui.painter().line_segment([r.left_bottom(), r.right_bottom()], Stroke::new(1.0, t.input_border));
    if n.is_layer() && Some(n.id) == view.current {
        // Current-layer marker: a small triangle in the top-right corner.
        let c = r.right_top();
        ui.painter().add(egui::Shape::convex_polygon(vec![c, c + vec2(-6.0, 0.0), c + vec2(0.0, 6.0)], t.text, Stroke::NONE));
    }
    eye_and_lock(ui, view, n, r, out);
    // Layer colour bar (every row shows the colour of its layer).
    let mut x = r.left() + 2.0 * COLUMN + 2.0;
    colour_bar(ui, n, r, x, color);
    x += 6.0 + depth as f32 * INDENT;
    // Disclosure triangle; Alt opens or closes everything inside too.
    if has_children {
        let dr = egui::Rect::from_min_size(egui::pos2(x, r.center().y - 8.0), vec2(14.0, 16.0));
        let dresp = ui.interact(dr, ui.id().with(("disc", n.id.0)), Sense::click());
        icons::paint(ui, if open { "chevron-down" } else { "chevron-right" }, dr, t.text_dim);
        if dresp.clicked() {
            out.toggle.push((n.id, !open));
            if ui.input(|i| i.modifiers.alt) {
                n.walk(&mut |c| {
                    if c.id != n.id && opens(c, &view.opts) {
                        out.toggle.push((c.id, !open));
                    }
                });
            }
        }
    }
    x += 16.0;
    // Thumbnail.
    let thumb_rect = view.opts.thumb(n).then(|| {
        let th = row_thumb(ui, view, &item.node, r, x);
        x = th.right() + 4.0;
        th
    });
    // Name (Preferences › Units › Identify Objects By: Object Name or XML ID).
    let xml_id = view.identify_xml_id;
    let name = if doc.mask_edit.is_some_and(|m| m.layer == n.id) { "<Opacity Mask>".to_string() } else { n.identify_label(xml_id) };
    let font = egui::FontId::proportional(13.0);
    let renaming: Option<(u64, String)> = ui.data(|d| d.get_temp(rename_id()));
    let name_rect = egui::Rect::from_min_max(egui::pos2(x - 2.0, r.center().y - 10.0), egui::pos2(r.right() - 44.0, r.center().y + 10.0));
    let name_end;
    match renaming {
        Some((rid, mut buf)) if rid == n.id.0 => {
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(name_rect));
            let te = child.add(egui::TextEdit::singleline(&mut buf).desired_width(name_rect.width()).font(font.clone()));
            // Focus the field as it opens. Asking every frame took the focus back from Enter,
            // Escape or a click elsewhere, so the rename never ended.
            if !te.has_focus() && !te.lost_focus() {
                te.request_focus();
            }
            if te.lost_focus() {
                ui.data_mut(|d| d.remove::<(u64, String)>(rename_id()));
                if child.input(|i| !i.key_pressed(egui::Key::Escape)) && buf != name {
                    out.actions.push(("layer.setProps".into(), json!({"ids": [n.id.0], "name": buf})));
                }
            } else {
                ui.data_mut(|d| d.insert_temp(rename_id(), (n.id.0, buf)));
            }
            name_end = name_rect.right();
        }
        _ => {
            let painter = ui.painter().with_clip_rect(name_rect);
            // Generated names ("<Path>", "<Opacity Mask>") are translated where painted; the stored name stays English.
            // XML IDs stay as ids (not translated).
            let shown = if doc.mask_edit.is_some_and(|m| m.layer == n.id) {
                tl!("<Opacity Mask>").to_string()
            } else if xml_id {
                name.clone()
            } else {
                painted_name(n, &n.display_name(), crate::i18n::current())
            };
            let text = painter.text(egui::pos2(x, r.center().y), egui::Align2::LEFT_CENTER, shown, font, if item.dim { t.text_dim } else { t.text });
            name_end = text.right();
            // A clipping path's name is underlined, a masked object's with a dashed line.
            if clip_path {
                painter.line_segment([text.left_bottom(), text.right_bottom()], Stroke::new(1.0, t.text));
            } else if n.extras().mask.is_some() {
                painter.extend(egui::Shape::dashed_line(&[text.left_bottom(), text.right_bottom()], Stroke::new(1.0, t.text), 3.0, 2.0));
            }
        }
    }
    // Target circle and selection square.
    let col_x = r.right() - 43.5;
    ui.painter().line_segment([egui::pos2(col_x, r.top()), egui::pos2(col_x, r.bottom())], Stroke::new(1.0, t.input_border));
    let tc = egui::pos2(r.right() - TARGET_X, r.center().y);
    let tresp = target_circle(ui, n, tc, h, view.target.map_or(is_sel, |id| id == n.id), &mut out.actions, t);
    let sq = egui::pos2(r.right() - SQUARE_X, r.center().y);
    if is_sel {
        let q = egui::Rect::from_center_size(sq, vec2(7.0, 7.0));
        ui.painter().rect_filled(q, 0.0, color);
        ui.painter().rect_stroke(q, 0.0, Stroke::new(1.0, t.text), StrokeKind::Inside);
    } else if child_sel {
        ui.painter().rect_filled(egui::Rect::from_center_size(sq, vec2(4.0, 4.0)), 0.0, color);
    }
    let sq_resp = ui.interact(egui::Rect::from_center_size(sq, vec2(18.0, h)), ui.id().with(("selsq", n.id.0)), Sense::click_and_drag());
    // Dragging the selected-art square carries the selection to another row.
    if (is_sel || child_sel) && sq_resp.drag_started() {
        egui::DragAndDrop::set_payload(ui.ctx(), LayersDrag::Art);
    }
    let m = ui.input(|i| i.modifiers);
    if tresp.clicked() {
        out.actions.push(("layer.target".into(), json!({"id": n.id.0, "add": m.shift})));
    } else if sq_resp.clicked() {
        out.actions.push(("layer.selectAll".into(), json!({"id": n.id.0, "add": m.shift})));
    } else if resp.clicked() {
        out.click = Some((n.id, m));
    }
    if resp.double_clicked() {
        let on_name = ui.input(|i| i.pointer.interact_pos()).is_some_and(|p| p.x <= name_end.max(name_rect.left() + 20.0) && p.x >= name_rect.left());
        let on_thumb = thumb_rect.is_some_and(|th| ui.input(|i| i.pointer.interact_pos()).is_some_and(|p| th.contains(p)));
        if on_name && !on_thumb {
            ui.data_mut(|d| d.insert_temp(rename_id(), (n.id.0, n.identify_label(view.identify_xml_id))));
        } else {
            out.actions.push(("ui.layerOptions".into(), json!({"ids": [n.id.0]})));
        }
    }
    // Row menu: Collect for Export on this row (or every highlighted row when this is one of them).
    let collect_ids: Vec<u64> = if view.rows.contains(&n.id) { view.rows.iter().map(|id| id.0).collect() } else { vec![n.id.0] };
    resp.context_menu(|ui| {
        collect_for_export_menu(ui, true, |multiple| {
            out.actions.push(("assets.add".into(), json!({ "ids": collect_ids.clone(), "multiple": multiple })));
        });
    });
    // Dragging rows: the highlighted rows when this is one of them, else this row.
    if resp.drag_started() && view.column.is_none() {
        let ids = if view.rows.contains(&n.id) { view.rows.clone() } else { vec![n.id] };
        egui::DragAndDrop::set_payload(ui.ctx(), LayersDrag::Rows(ids));
        let grab = resp.interact_pointer_pos().map_or(vec2(h, h / 2.0), |p| p - r.min);
        ui.data_mut(|d| d.insert_temp(key("grab"), (grab, r.width(), depth)));
    }
    if resp.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
    }
    drop_target(ui, view, n, r, x - 16.0, open, &resp, out);
}

/// Alt-clicking row `n`'s eye (`eye`) or lock: Hide Others or Lock Others, every top-level layer
/// but the one holding the row; when they all are hidden (locked) already, Show All Layers (Unlock
/// All Layers). Each is one undo step.
fn others_action(doc: &Document, n: &Node, eye: bool) -> (String, Value) {
    let own = doc.layer_of(n.id);
    let mut others = doc.layers.iter().filter(|l| Some(l.id) != own);
    let cmd = match (eye, if eye { others.any(|l| l.visible) } else { others.any(|l| !l.locked) }) {
        (true, true) => "layer.hideOthers",
        (true, false) => "layer.showAll",
        (false, true) => "layer.lockOthers",
        (false, false) => "layer.unlockAll",
    };
    // Show All and Unlock All take no rows.
    let params = if cmd.ends_with("Others") { json!({ "ids": [n.id.0] }) } else { json!({}) };
    (cmd.into(), params)
}

/// Ctrl/Cmd+Alt-clicking row `n`'s eye runs Outline Others, which puts every top-level layer but the
/// one holding the row in Outline and puts that one in Preview. When all the other layers are in
/// Outline already, it runs Preview All Layers. Each is one undo step.
fn outline_others_action(doc: &Document, n: &Node) -> (String, Value) {
    let own = doc.layer_of(n.id);
    if doc.layers.iter().any(|l| Some(l.id) != own && !matches!(l.kind, NodeKind::Layer { preview: false, .. })) {
        ("layer.outlineOthers".into(), json!({ "ids": [n.id.0] }))
    } else {
        // Preview All takes no rows.
        ("layer.previewAll".into(), json!({}))
    }
}

/// The eye and lock columns of row `n` at `r`.
fn eye_and_lock(ui: &mut Ui, view: &View, n: &Node, r: egui::Rect, out: &mut Out) {
    let t = &view.t;
    let h = view.h;
    let eye = egui::Rect::from_min_size(r.min, vec2(COLUMN, h));
    let lock = egui::Rect::from_min_size(r.min + vec2(COLUMN, 0.0), vec2(COLUMN, h));
    ui.painter().line_segment([eye.right_top(), eye.right_bottom()], Stroke::new(1.0, t.input_border));
    ui.painter().line_segment([lock.right_top(), lock.right_bottom()], Stroke::new(1.0, t.input_border));
    let preview = !matches!(n.kind, NodeKind::Layer { preview: false, .. });
    let er = ui.interact(eye, ui.id().with(("eye", n.id.0)), Sense::click_and_drag());
    if n.visible {
        let icon = egui::Rect::from_center_size(eye.center(), vec2(14.0, 14.0));
        if n.is_template() {
            // A template layer shows the template mark instead of the eye.
            icons::paint(ui, "shapes", icon, t.icon);
        } else if preview {
            icons::paint(ui, "eye", icon, t.icon);
        } else {
            // Outline view: a hollow eye.
            icons::paint(ui, "eye", icon, t.text_dim);
            ui.painter().circle_stroke(eye.center(), 7.5, Stroke::new(1.0, t.text_dim));
        }
    }
    let lr = ui.interact(lock, ui.id().with(("lock", n.id.0)), Sense::click_and_drag());
    if n.locked {
        icons::paint(ui, "lock", egui::Rect::from_center_size(lock.center(), vec2(12.0, 12.0)), t.icon);
    } else if lr.hovered() {
        icons::paint(ui, "lock", egui::Rect::from_center_size(lock.center(), vec2(12.0, 12.0)), t.text_disabled);
    }
    let m = ui.input(|i| i.modifiers);
    if er.clicked() {
        if m.alt && m.command {
            out.actions.push(outline_others_action(view.doc, n));
        } else if m.alt {
            out.actions.push(others_action(view.doc, n, true));
        } else if m.command && n.is_layer() {
            // Ctrl/Cmd-click: Preview ↔ Outline for this layer.
            out.actions.push(("layer.setProps".into(), json!({"ids": [n.id.0], "preview": !preview})));
        } else {
            out.actions.push(("layer.setProps".into(), json!({"ids": [n.id.0], "visible": !n.visible})));
        }
    }
    if lr.clicked() && m.alt {
        out.actions.push(others_action(view.doc, n, false));
    } else if lr.clicked() {
        out.actions.push(("layer.setProps".into(), json!({"ids": [n.id.0], "locked": !n.locked})));
    }
    // Dragging down a column gives every row it passes this row's new state.
    if er.drag_started() {
        out.column = Some(Some(ColumnDrag { eye: true, value: !n.visible, ids: vec![n.id.0] }));
    } else if lr.drag_started() {
        out.column = Some(Some(ColumnDrag { eye: false, value: !n.locked, ids: vec![n.id.0] }));
    } else if let Some(c) = out.column.clone().unwrap_or_else(|| view.column.clone())
        && !c.ids.contains(&n.id.0)
        && ui.input(|i| i.pointer.primary_down() && i.pointer.interact_pos().is_some_and(|p| p.y >= r.top() && p.y < r.bottom()))
    {
        let mut c = c;
        c.ids.push(n.id.0);
        out.column = Some(Some(c));
    }
}

/// Row `n` at `r` as a drop target for dragged rows or the selected-art square: shows where they
/// would go (a line above or below it, or a box around it for inside) and moves them on release
/// (Alt copies). `indent` is where the row's content starts.
#[allow(clippy::too_many_arguments)]
fn drop_target(ui: &Ui, view: &View, n: &Node, r: egui::Rect, indent: f32, open: bool, resp: &egui::Response, out: &mut Out) {
    let Some(drag) = resp.dnd_hover_payload::<LayersDrag>() else { return };
    let doc = view.doc;
    let moving: Vec<NodeId> = match &*drag {
        LayersDrag::Rows(ids) => ids.clone(),
        LayersDrag::Art => view.sel.iter().copied().collect(),
        LayersDrag::Guides(guides) => {
            // Guides go to the layer of the row (the row's own, for a layer).
            if let Some(layer) = if n.is_layer() { Some(n.id) } else { doc.layer_containing(n.id) } {
                guide_drop(ui, view, guides, layer, r, resp, out);
            }
            return;
        }
    };
    // Never into itself or a row inside it.
    let around = doc.ancestry(n.id).unwrap_or_default();
    if moving.is_empty() || moving.iter().any(|m| around.contains(m)) {
        return;
    }
    let Some(p) = ui.input(|i| i.pointer.hover_pos()) else { return };
    let rel = (p.y - r.top()) / r.height().max(1.0);
    let inside = takes(n, &drag, doc);
    let place = match &*drag {
        LayersDrag::Art if inside => Place::Inside,
        LayersDrag::Art => Place::Above,
        LayersDrag::Rows(_) if inside && rel >= 0.25 && (rel <= 0.75 || open) => Place::Inside,
        LayersDrag::Rows(_) if rel < 0.5 => Place::Above,
        LayersDrag::Rows(_) | LayersDrag::Guides(_) => Place::Below,
    };
    let stroke = Stroke::new(2.0, view.t.accent);
    match place {
        Place::Above => {
            ui.painter().line_segment([egui::pos2(indent, r.top()), egui::pos2(r.right(), r.top())], stroke);
        }
        Place::Below => {
            ui.painter().line_segment([egui::pos2(indent, r.bottom()), egui::pos2(r.right(), r.bottom())], stroke);
        }
        Place::Inside => {
            ui.painter().rect_stroke(r.shrink(1.0), 0.0, stroke, StrokeKind::Inside);
        }
    }
    if resp.dnd_release_payload::<LayersDrag>().is_some() {
        let copy = ui.input(|i| i.modifiers.alt);
        let ids: Vec<u64> = match &*drag {
            // The selection's top objects, as Arrange and Group take them.
            LayersDrag::Art => doc.paint_order(moving.iter().copied()).iter().map(|i| i.0).collect(),
            LayersDrag::Rows(ids) => ids.iter().map(|i| i.0).collect(),
            LayersDrag::Guides(_) => return,
        };
        out.actions.push(("layer.move".into(), json!({"ids": ids, "target": n.id.0, "place": place.id(), "copy": copy})));
    }
}

/// The target circle of `n`'s row at `c`: a ring, doubled while `n` is targeted, filled when `n`
/// has an appearance or transparency of its own. Its response is clicked to target `n`; dragging
/// it carries `n`'s appearance ([`PanelDrag::Appearance`]) onto another row's circle (Alt
/// copies it) or the trash.
fn target_circle(ui: &mut Ui, n: &Node, c: egui::Pos2, h: f32, targeted: bool, actions: &mut Vec<(String, Value)>, t: &Tokens) -> egui::Response {
    let resp = ui.interact(egui::Rect::from_center_size(c, vec2(16.0, h)), ui.id().with(("target", n.id.0)), Sense::click_and_drag());
    widgets::drag_source(ui, &resp, || PanelDrag::Appearance(n.id));
    let held = resp.dnd_hover_payload::<PanelDrag>().is_some_and(|d| matches!(*d, PanelDrag::Appearance(id) if id != n.id));
    if let Some(d) = resp.dnd_release_payload::<PanelDrag>()
        && let PanelDrag::Appearance(source) = *d
        && source != n.id
    {
        let copy = ui.input(|i| i.modifiers.alt);
        actions.push(("appearance.transfer".into(), json!({"source": source.0, "target": n.id.0, "copy": copy})));
    }
    let ring = if held { t.accent } else { t.icon };
    ui.painter().circle_stroke(c, 5.0, Stroke::new(if held { 1.5 } else { 1.0 }, ring));
    if targeted {
        ui.painter().circle_stroke(c, 2.8, Stroke::new(1.0, t.icon));
    }
    if has_styled_target(n) {
        ui.painter().circle_filled(c, 3.2, t.icon);
    }
    if resp.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
    }
    resp
}

/// Whether an object's target circle is filled: its appearance is not basic (a group or layer:
/// it has fills, strokes or effects of its own) or its transparency (opacity, blend mode,
/// isolation, knockout or an opacity mask) is not the default.
fn has_styled_target(n: &Node) -> bool {
    !n.appearance.is_basic()
        || (matches!(n.kind, NodeKind::Group { .. } | NodeKind::Layer { .. }) && !n.appearance.items.is_empty())
        || !n.has_default_transparency()
}

/// Where row thumbnails come from: document `uid` at `revision`, and whether a drag is in
/// progress (the thumbnails shown then stay until it ends).
pub(crate) struct ThumbSource<'a> {
    pub uid: u64,
    pub revision: u64,
    pub doc: &'a Arc<Document>,
    pub busy: bool,
}

impl<'a> ThumbSource<'a> {
    /// The thumbnails of the document `st` holds (`doc`, its document).
    pub(crate) fn of(app: &VectorcraftApp, st: &vectorcraft_engine::DocState, doc: &'a Arc<Document>) -> Self {
        Self { uid: st.uid, revision: st.revision, doc, busy: st.interaction.is_some() || app.session.tool_busy() }
    }
}

/// Node `id` of `doc`, as the `Arc` its parent holds.
pub(crate) fn node_arc(doc: &Document, id: NodeId) -> Option<Arc<Node>> {
    let path = doc.index_path(id)?;
    let (first, rest) = path.split_first()?;
    let mut cur = doc.layers.get(*first)?;
    for &i in rest {
        cur = cur.children()?.get(i)?;
    }
    Some(cur.clone())
}

/// A row thumbnail's image.
struct ThumbEntry {
    /// The node it shows. Weak, so the cache doesn't keep old art alive, but its allocation stays
    /// and no other node can come to have its address.
    node: Weak<Node>,
    px: u32,
    /// None: the node draws nothing.
    tex: Option<egui::TextureHandle>,
    /// How long it took to render (ms).
    ms: f64,
    /// The pass it was last drawn in.
    used: u64,
}

/// A row thumbnail job: the row (document uid, node), the node and the size rendered.
type ThumbJob = ((u64, NodeId), Weak<Node>, u32);

#[derive(Default)]
struct Thumbs {
    entries: HashMap<(u64, NodeId), ThumbEntry>,
    /// What the worker is rendering, by row.
    pending: HashMap<(u64, NodeId), (Weak<Node>, u32)>,
    worker: Option<ThumbWorker<(u64, NodeId), ThumbJob>>,
    worker_started: bool,
    settle: Settle,
    /// (pass, ms, count) rendered on the UI thread in that pass.
    spent: (u64, f64, u32),
    /// Thumbnails rendered on the UI thread (tests count them).
    renders: u64,
}

thread_local! {
    static THUMBS: crate::graphics::TexCache<Thumbs> = crate::graphics::TexCache::default();
}

/// The most row thumbnails kept: past it the ones drawn longest ago go.
const MAX_THUMBS: usize = 512;
/// Rendering a thumbnail that took longer than this (ms) again goes to the worker.
const LIGHT_THUMB_MS: f64 = 4.0;
/// A node with more objects than this inside has its first thumbnail made on the worker.
const LIGHT_THUMB_NODES: usize = 200;
/// At most this long (ms) and this many thumbnails are rendered on the UI thread in a pass; the
/// rest go to the worker, or wait for the next pass where there is none.
const THUMB_BUDGET_MS: f64 = 8.0;
const THUMB_BUDGET_COUNT: u32 = 16;

/// How row thumbnails render: images far smaller than they are sample their mip levels.
pub(crate) fn thumb_options() -> vectorcraft_render::RenderOptions {
    vectorcraft_render::RenderOptions { image_mips: true, ..Default::default() }
}

/// Has `n` at most `limit` objects inside it (counting stops there)?
fn small_node(n: &Node, limit: usize) -> bool {
    fn count(n: &Node, left: &mut usize) -> bool {
        for c in n.children().into_iter().flatten() {
            if *left == 0 {
                return false;
            }
            *left -= 1;
            if !count(c, left) {
                return false;
            }
        }
        true
    }
    let mut left = limit;
    count(n, &mut left)
}

impl Thumbs {
    /// Keep `img` (None: nothing drawn) as the thumbnail of row `key` showing `node`.
    fn upload(&mut self, ctx: &egui::Context, key: (u64, NodeId), node: Weak<Node>, px: u32, img: Option<&vectorcraft_render::Rendered>, ms: f64) {
        let color = img.map(|img| egui::ColorImage::from_rgba_premultiplied([img.width as usize, img.height as usize], &img.pixels));
        let used = ctx.cumulative_pass_nr();
        let name = format!("layer-thumb-{}", key.1.0);
        match self.entries.get_mut(&key) {
            Some(e) => {
                match (color, &mut e.tex) {
                    (Some(color), Some(tex)) => tex.set(color, egui::TextureOptions::LINEAR),
                    (Some(color), tex) => *tex = Some(ctx.load_texture(name, color, egui::TextureOptions::LINEAR)),
                    (None, tex) => *tex = None,
                }
                (e.node, e.px, e.ms, e.used) = (node, px, ms, used);
            }
            None => {
                let tex = color.map(|color| ctx.load_texture(name, color, egui::TextureOptions::LINEAR));
                self.entries.insert(key, ThumbEntry { node, px, tex, ms, used });
                if self.entries.len() > MAX_THUMBS {
                    // Keep the three quarters drawn last.
                    let mut by_use: Vec<u64> = self.entries.values().map(|e| e.used).collect();
                    by_use.sort_unstable();
                    let cut = by_use.get(by_use.len() - MAX_THUMBS * 3 / 4).copied().unwrap_or(0);
                    self.entries.retain(|_, e| e.used >= cut);
                }
            }
        }
    }

    /// The texture of row `n` (`px` pixels square) to draw: made or updated now, or on the
    /// worker (the old one is drawn meanwhile). Thumbnails shown stay during a drag and until
    /// the document settles.
    fn texture(&mut self, ctx: &egui::Context, src: &ThumbSource, n: &Arc<Node>, px: u32) -> Option<egui::TextureHandle> {
        if !self.worker_started {
            self.worker_started = true;
            self.worker = ThumbWorker::spawn(ctx, "thumbnails");
        }
        let done = match self.worker.as_ref().map(ThumbWorker::poll) {
            Some(Some(done)) => done,
            Some(None) => {
                // The thread is gone: render here from now on.
                self.worker = None;
                self.pending.clear();
                vec![]
            }
            None => vec![],
        };
        for d in done {
            let (key, node, px) = d.key;
            if self.pending.get(&key).is_some_and(|(p, ppx)| p.ptr_eq(&node) && *ppx == px) {
                self.pending.remove(&key);
            }
            self.upload(ctx, key, node, px, d.img.as_ref(), d.ms);
        }
        let key = (src.uid, n.id);
        let pass = ctx.cumulative_pass_nr();
        let (fresh, light) = match self.entries.get_mut(&key) {
            Some(e) => {
                e.used = pass;
                (std::ptr::eq(e.node.as_ptr(), Arc::as_ptr(n)) && e.px == px, Some(e.ms <= LIGHT_THUMB_MS))
            }
            None => (false, None),
        };
        let shown = self.entries.get(&key).and_then(|e| e.tex.clone());
        if fresh || self.pending.get(&key).is_some_and(|(p, ppx)| std::ptr::eq(p.as_ptr(), Arc::as_ptr(n)) && *ppx == px) {
            return shown;
        }
        if light.is_some() && (src.busy || !self.settle.settled(ctx, src.uid, src.revision, ctx.input(|i| i.time))) {
            return shown;
        }
        if self.spent.0 != pass {
            self.spent = (pass, 0.0, 0);
        }
        let in_budget = self.spent.1 < THUMB_BUDGET_MS && self.spent.2 < THUMB_BUDGET_COUNT;
        let light = light.unwrap_or_else(|| small_node(n, LIGHT_THUMB_NODES)) && in_budget;
        if !light && let Some(w) = &self.worker {
            let (doc, node) = (src.doc.clone(), n.clone());
            let job: ThumbJob = (key, Arc::downgrade(n), px);
            if w.submit(key, job, Box::new(move |r| r.render_node_thumbnail_with(&doc, &node, px, None, &thumb_options()))) {
                self.pending.insert(key, (Arc::downgrade(n), px));
                return shown;
            }
            self.worker = None;
            self.pending.clear();
        }
        if !in_budget {
            ctx.request_repaint();
            return shown;
        }
        thread_local! {
            static RENDERER: std::cell::RefCell<vectorcraft_render::Renderer> = std::cell::RefCell::new(vectorcraft_render::Renderer::new());
        }
        let t0 = crate::now_ms();
        let img = RENDERER.with(|r| r.borrow_mut().render_node_thumbnail_with(src.doc, n, px, None, &thumb_options()));
        let ms = crate::now_ms() - t0;
        self.spent.1 += ms;
        self.spent.2 += 1;
        self.renders += 1;
        self.upload(ctx, key, Arc::downgrade(n), px, img.as_ref(), ms);
        self.entries.get(&key).and_then(|e| e.tex.clone())
    }
}

/// A real rendered thumbnail of row `n`, cached by node identity (unchanged nodes keep their `Arc`
/// allocation, so the address is a free change detector). Only rendered for visible rows; see
/// [`Thumbs::texture`] for when.
pub(crate) fn real_thumb(ui: &Ui, src: &ThumbSource, n: &Arc<Node>, r: egui::Rect) -> bool {
    if !ui.is_rect_visible(r) {
        return true;
    }
    let ppp = ui.ctx().pixels_per_point();
    let px = ((r.width() * ppp).round() as u32).max(8);
    match THUMBS.with(|c| c.borrow_mut().texture(ui.ctx(), src, n, px)) {
        Some(t) => {
            ui.painter().image(t.id(), r, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
            true
        }
        None => false,
    }
}

/// Tiny vector thumbnail painted with egui (fallback when rendering isn't possible).
fn thumb(ui: &Ui, n: &Node, r: egui::Rect) {
    let Some(b) = n.visual_bounds() else { return };
    let s = ((r.width() - 3.0) as f64 / b.width().max(b.height()).max(1e-6)) as f32;
    let off = r.center() - vec2(b.center().x as f32 * s, b.center().y as f32 * s);
    let p = ui.painter().with_clip_rect(r);
    let mut count = 0;
    n.walk(&mut |c| {
        if count > 40 {
            return;
        }
        if let Some(pd) = c.path_data() {
            count += 1;
            let col = c.appearance.fill_paint().color().or(c.appearance.stroke_paint().color()).map(|cc| {
                let [a, b2, d, _] = cc.to_rgba8(1.0);
                Color32::from_rgb(a, b2, d)
            });
            if let Some(bb) = pd.bounds() {
                let rr = egui::Rect::from_min_max(off + vec2(bb.x0 as f32 * s, bb.y0 as f32 * s), off + vec2(bb.x1 as f32 * s, bb.y1 as f32 * s));
                p.rect_filled(rr, 0.0, col.unwrap_or(Color32::from_gray(120)));
            }
        }
    });
}

/// A panel menu item: label, command, params, enabled, checked.
type MenuItem<'a> = (&'a str, &'a str, Value, bool, bool);

/// The Layers panel's (≡) menu.
pub fn menu(app: &mut VectorcraftApp, ui: &mut Ui) {
    let Some(st) = app.session.active() else { return };
    let doc = st.doc.clone();
    let rows = st.highlighted_rows();
    let current = st.current_layer();
    let has_sel = !st.selection.is_empty();
    let isolated = st.isolation.is_some();
    let targets: Vec<NodeId> = if rows.is_empty() { current.into_iter().collect() } else { rows.clone() };
    let one_container = match targets.as_slice() {
        [id] => doc.node(*id).is_some_and(|n| matches!(n.kind, NodeKind::Layer { .. } | NodeKind::Group { .. })),
        _ => false,
    };
    let layers_in: Vec<&Node> = targets.iter().filter_map(|id| doc.node(*id)).filter(|n| n.is_layer()).collect();
    let template = layers_in.first().is_some_and(|l| l.is_template());
    // The top-level layers holding the rows, and the others.
    let kept: HashSet<NodeId> = targets.iter().filter_map(|id| doc.layer_of(*id)).collect();
    let others: Vec<&Node> = doc.layers.iter().map(|l| &**l).filter(|l| !kept.contains(&l.id)).collect();
    let mut all_layers = vec![];
    doc.walk(|n| {
        if n.is_layer() {
            all_layers.push(n);
        }
    });
    let mut items: Vec<Option<MenuItem>> = vec![
        Some(("New Layer…", "ui.newLayer", json!({}), true, false)),
        Some(("New Sublayer…", "ui.newLayer", json!({"sublayer": true}), current.is_some(), false)),
        Some(("Duplicate Selection", "layer.duplicate", json!({}), !targets.is_empty(), false)),
        Some(("Delete Selection", "layer.delete", json!({}), !targets.is_empty(), false)),
        Some(("Options for Selection…", "ui.layerOptions", json!({}), !targets.is_empty(), false)),
        None,
        Some(("Make/Release Clipping Mask", "layer.clippingMask.toggle", json!({}), true, false)),
        None,
    ];
    items.push(if isolated {
        Some(("Exit Isolation Mode", "object.exitIsolation", json!({}), true, false))
    } else {
        let id = targets.first().map(|i| i.0);
        Some(("Enter Isolation Mode", "object.isolate", json!({ "id": id }), one_container, false))
    });
    items.extend([
        None,
        Some(("Locate Object", "layer.locate", json!({}), has_sel, false)),
        None,
        Some(("Merge Selected", "layer.merge", json!({}), rows.len() >= 2, false)),
        Some(("Flatten Artwork", "layer.flatten", json!({}), true, false)),
        Some(("Collect in New Layer", "layer.collectInNew", json!({}), !rows.is_empty() || has_sel, false)),
        // Sentinel: rendered as Collect for Export › As Single / As Multiple (assets.add).
        Some(("Collect for Export", "", json!({}), !targets.is_empty() || has_sel, false)),
        Some(("Release to Layers (Sequence)", "layer.releaseToLayers", json!({}), one_container, false)),
        Some(("Release to Layers (Build)", "layer.releaseToLayersBuild", json!({}), one_container, false)),
        Some(("Reverse Order", "layer.reverse", json!({}), rows.len() >= 2, false)),
        None,
        Some(("Template", "layer.template", json!({}), !layers_in.is_empty(), template)),
    ]);
    // Each pair reads by state: Hide Others while another layer shows, else Show All Layers.
    let any_other = |f: &dyn Fn(&Node) -> bool| others.iter().any(|l| f(l));
    items.push(Some(if any_other(&|l| l.visible) {
        ("Hide Others", "layer.hideOthers", json!({}), true, false)
    } else {
        ("Show All Layers", "layer.showAll", json!({}), all_layers.iter().any(|l| !l.visible), false)
    }));
    items.push(Some(if any_other(&|l| !matches!(l.kind, NodeKind::Layer { preview: false, .. })) {
        ("Outline Others", "layer.outlineOthers", json!({}), true, false)
    } else {
        (
            "Preview All Layers",
            "layer.previewAll",
            json!({}),
            all_layers.iter().any(|l| matches!(l.kind, NodeKind::Layer { preview: false, .. })),
            false,
        )
    }));
    items.push(Some(if any_other(&|l| !l.locked) {
        ("Lock Others", "layer.lockOthers", json!({}), true, false)
    } else {
        ("Unlock All Layers", "layer.unlockAll", json!({}), all_layers.iter().any(|l| l.locked), false)
    }));
    items.push(None);
    let remembers = doc.paste_remembers_layers;
    items.push(Some(("Paste Remembers Layers", "layer.pasteRemembersLayers", json!({"on": !remembers}), true, remembers)));
    items.push(None);
    items.push(Some(("Panel Options…", "ui.layersPanelOptions", json!({}), true, false)));
    for item in items {
        match item {
            None => {
                ui.separator();
            }
            Some(("Collect for Export", _, _, enabled, _)) => {
                collect_for_export_menu(ui, enabled, |multiple| {
                    let mut p = json!({ "multiple": multiple });
                    if !targets.is_empty() {
                        p["ids"] = json!(targets.iter().map(|id| id.0).collect::<Vec<_>>());
                    }
                    app.run("assets.add", p).ok();
                });
            }
            Some((label, cmd, p, enabled, checked)) => {
                if widgets::menu_item(ui, label, enabled, checked) {
                    app.run(cmd, p).ok();
                }
            }
        }
    }
}

/// Collect for Export › As Single Asset / As Multiple Assets (`assets.add`).
fn collect_for_export_menu(ui: &mut Ui, enabled: bool, mut run: impl FnMut(bool)) {
    ui.add_enabled_ui(enabled, |ui| {
        // English labels match the rest of this panel menu (and Object › Collect for Export).
        ui.menu_button("Collect for Export", |ui| {
            if widgets::menu_item(ui, "As Single Asset", true, false) {
                run(false);
                ui.close();
            }
            if widgets::menu_item(ui, "As Multiple Assets", true, false) {
                run(true);
                ui.close();
            }
        });
    });
}

/// `ui.layersExpand {ids?, open?}`: open (or close) rows of the active document's Layers panel;
/// without `ids`, every row that holds others.
pub fn expand(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let st = app.session.active().ok_or("no document open")?;
    let open = p.get("open").and_then(Value::as_bool).unwrap_or(true);
    let ids: Vec<u64> = match p.get("ids").and_then(Value::as_array) {
        Some(a) => a.iter().filter_map(Value::as_u64).filter(|i| st.doc.node(NodeId(*i)).is_some()).collect(),
        None => {
            let mut v = vec![];
            st.doc.walk(|n| {
                if n.is_container() {
                    v.push(n.id.0);
                }
            });
            v
        }
    };
    let st = app.session.active_mut().ok_or("no document open")?;
    for i in &ids {
        st.layers_open.set(NodeId(*i), open);
    }
    Ok(json!({ "count": ids.len() }))
}

/// `ui.layersFilter {objects?, layers?}`: set the Layers panel's Filter, `objects` to the object
/// kinds ticked under By Object and `layers` to the row states ticked under By Layer, by parameter
/// name ([`ObjectKind::ALL`], [`RowState::ALL`]). A key left out or set to null keeps its list; any
/// other value that is not an array of those names is an error, and then nothing changes.
/// → {objects, layers}
pub fn set_filter(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let objects = parse_names(p, "objects", &ObjectKind::ALL)?;
    let layers = parse_names(p, "layers", &RowState::ALL)?;
    let f = &mut app.ui.layers_filter;
    if let Some(objects) = objects {
        f.objects = objects;
    }
    if let Some(layers) = layers {
        f.layers = layers;
    }
    Ok(json!({"objects": names(&ObjectKind::ALL, &f.objects), "layers": names(&RowState::ALL, &f.layers)}))
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use vectorcraft_engine::Session;

    use super::*;

    /// Filled target circles drawn by one headless frame of the panel.
    fn filled_targets(app: &mut VectorcraftApp, ctx: &egui::Context) -> usize {
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| show(app, ui));
        out.textures_delta.clear();
        out.shapes.iter().filter(|c| matches!(&c.shape, egui::Shape::Circle(cs) if cs.radius == 3.2 && cs.fill != Color32::TRANSPARENT)).count()
    }

    /// A generated `<Kind>` name is translated where painted; an unnamed text object's text never
    /// is, even when it reads like one.
    #[test]
    fn only_generated_names_are_translated() {
        let zh = crate::i18n::Lang::from_code("zh-hant").unwrap();
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        let run = |app: &mut VectorcraftApp, id: &str, p: serde_json::Value| app.session.execute(id, &p).unwrap();
        run(&mut app, "file.new", json!({"width": 100, "height": 100}));
        let rect = run(&mut app, "shape.rectangle", json!({"x": 0, "y": 0, "width": 50, "height": 50}))["id"].as_u64().unwrap();
        let text = run(&mut app, "text.create", json!({"x": 10, "y": 40, "text": "<Path>"}))["id"].as_u64().unwrap();
        let doc = &app.session.active().unwrap().doc;
        let names = |id: u64| {
            let n = doc.node(NodeId(id)).unwrap();
            (n.display_name(), painted_name(n, &n.display_name(), zh))
        };
        assert_eq!(names(text), ("<Path>".to_string(), "<Path>".to_string()));
        let (stored, painted) = names(rect);
        let inner = stored.trim_start_matches('<').trim_end_matches('>');
        assert_eq!(painted, format!("<{}>", crate::i18n::tr(zh, inner)));
        assert_ne!(painted, stored, "translated");
    }

    #[test]
    fn target_circle_fills_for_non_default_transparency() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        let run = |app: &mut VectorcraftApp, id: &str, p: serde_json::Value| app.session.execute(id, &p).unwrap();
        run(&mut app, "file.new", json!({"width": 100, "height": 100}));
        let id = run(&mut app, "shape.rectangle", json!({"x": 0, "y": 0, "width": 50, "height": 50}))["id"].clone();
        run(&mut app, "select.set", json!({ "ids": [id] }));
        let ctx = egui::Context::default();
        assert_eq!(filled_targets(&mut app, &ctx), 0);
        run(&mut app, "transparency.set", json!({"blend": "multiply"}));
        assert_eq!(filled_targets(&mut app, &ctx), 1, "a Multiply object");
        run(&mut app, "transparency.set", json!({"blend": "normal", "knockout": true}));
        assert_eq!(filled_targets(&mut app, &ctx), 1, "a knockout group");
        run(&mut app, "transparency.set", json!({"knockout": false}));
        run(&mut app, "appearance.addStroke", json!({}));
        assert_eq!(filled_targets(&mut app, &ctx), 1, "two strokes");
    }

    #[test]
    fn a_layer_clipping_mask_underlines_its_clipping_path() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        let run = |app: &mut VectorcraftApp, id: &str, p: serde_json::Value| app.session.execute(id, &p).unwrap();
        run(&mut app, "file.new", json!({"width": 100, "height": 100}));
        run(&mut app, "shape.rectangle", json!({"x": 0, "y": 0, "width": 100, "height": 100}));
        run(&mut app, "shape.rectangle", json!({"x": 25, "y": 25, "width": 50, "height": 50}));
        run(&mut app, "select.none", json!({}));
        let ctx = egui::Context::default();
        let text = Tokens::get(&ctx).text;
        // Line segments in the text colour: the underlines.
        let underlines = |app: &mut VectorcraftApp| {
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| show(app, ui));
            out.textures_delta.clear();
            out.shapes.iter().filter(|c| matches!(&c.shape, egui::Shape::LineSegment { stroke, .. } if stroke.color == text)).count()
        };
        assert_eq!(underlines(&mut app), 0);
        assert_eq!(run(&mut app, "layer.clippingMask.toggle", json!({}))["clip"], true);
        assert_eq!(underlines(&mut app), 1);
        run(&mut app, "layer.clippingMask.toggle", json!({}));
        assert_eq!(underlines(&mut app), 0);
    }

    /// One headless frame of the panel with `events` (Alt held when `alt`): the centres of the
    /// target circles, top row first, and the text-coloured line segments (underlines) drawn.
    fn frame(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<egui::Event>, alt: bool) -> (Vec<egui::Pos2>, usize) {
        let mut events = events;
        events.insert(0, egui::Event::ModifiersChanged(egui::Modifiers { alt, ..Default::default() }));
        let mut out = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| show(app, ui));
        out.textures_delta.clear();
        let text = Tokens::get(ctx).text;
        let mut circles = vec![];
        let mut lines = 0;
        for c in &out.shapes {
            match &c.shape {
                egui::Shape::Circle(cs) if cs.radius == 5.0 => circles.push(cs.center),
                egui::Shape::LineSegment { stroke, .. } if stroke.color == text => lines += 1,
                _ => {}
            }
        }
        circles.sort_by(|a, b| a.y.total_cmp(&b.y));
        (circles, lines)
    }

    fn button(pos: egui::Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() }
    }

    /// Drag from `a` to `b` over a few frames (Alt held when `alt`).
    fn drag(app: &mut VectorcraftApp, ctx: &egui::Context, a: egui::Pos2, b: egui::Pos2, alt: bool) {
        let steps = [
            vec![egui::Event::PointerMoved(a)],
            vec![button(a, true)],
            vec![egui::Event::PointerMoved(a + vec2(0.0, 6.0))],
            vec![egui::Event::PointerMoved(b)],
            vec![button(b, false)],
            vec![],
        ];
        for e in steps {
            frame(app, ctx, e, alt);
        }
    }

    #[test]
    fn alt_clicking_an_eye_or_lock_toggles_the_other_layers() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        let run = |app: &mut VectorcraftApp, id: &str, p: serde_json::Value| app.session.execute(id, &p).unwrap();
        run(&mut app, "file.new", json!({"width": 200, "height": 200}));
        run(&mut app, "layer.new", json!({}));
        run(&mut app, "layer.new", json!({}));
        let ctx = egui::Context::default();
        // Rows top first: Layers 3, 2, 1.
        let (circles, _) = frame(&mut app, &ctx, vec![], false);
        assert_eq!(circles.len(), 3);
        let state =
            |app: &VectorcraftApp| -> Vec<(bool, bool)> { app.session.active().unwrap().doc.layers.iter().map(|l| (l.visible, l.locked)).collect() };
        let click = |app: &mut VectorcraftApp, at: egui::Pos2, alt: bool| {
            frame(app, &ctx, vec![egui::Event::PointerMoved(at)], alt);
            frame(app, &ctx, vec![button(at, true)], alt);
            frame(app, &ctx, vec![button(at, false)], alt);
        };
        let (eye, lock) = (egui::pos2(12.0, circles[1].y), egui::pos2(37.0, circles[1].y));
        let undo = |app: &VectorcraftApp| app.session.active().unwrap().history.undo.len();
        let before = undo(&app);
        // Alt-click Layer 2's eye: Layers 1 and 3 hide in one step; again, they show.
        click(&mut app, eye, true);
        assert_eq!(state(&app), [(false, false), (true, false), (false, false)]);
        assert_eq!(undo(&app), before + 1);
        click(&mut app, eye, true);
        assert_eq!(state(&app), [(true, false); 3]);
        // Alt-click its lock: the others lock; again, every layer unlocks.
        click(&mut app, lock, true);
        assert_eq!(state(&app), [(true, true), (true, false), (true, true)]);
        assert_eq!(app.session.active().unwrap().history.undo.last().map(|u| u.label.as_str()), Some("Lock Others"));
        // A plain click still toggles only that layer.
        click(&mut app, eye, false);
        assert_eq!(state(&app), [(true, true), (false, false), (true, true)]);
        click(&mut app, lock, true);
        assert_eq!(state(&app), [(true, false), (false, false), (true, false)]);
        // A sublayer's row stands for its top-level layer: the other layers hide.
        click(&mut app, eye, false);
        let layer2 = app.session.active().unwrap().doc.layers[1].id.0;
        let sub = run(&mut app, "layer.newSublayer", json!({"parent": layer2}));
        // Rows: Layer 3, Layer 2, its sublayer, Layer 1.
        let (circles, _) = frame(&mut app, &ctx, vec![], false);
        assert_eq!(circles.len(), 4);
        click(&mut app, egui::pos2(12.0, circles[2].y), true);
        assert_eq!(state(&app), [(false, false), (true, false), (false, false)]);
        let doc = &app.session.active().unwrap().doc;
        assert!(doc.node(NodeId(sub["id"].as_u64().unwrap())).unwrap().visible);
    }

    #[test]
    fn ctrl_alt_clicking_an_eye_switches_the_other_layers_between_outline_and_preview() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        let run = |app: &mut VectorcraftApp, id: &str, p: serde_json::Value| app.session.execute(id, &p).unwrap();
        run(&mut app, "file.new", json!({"width": 200, "height": 200}));
        run(&mut app, "layer.new", json!({}));
        run(&mut app, "layer.new", json!({}));
        let ctx = egui::Context::default();
        // Rows top first: Layers 3, 2, 1.
        let (circles, _) = frame(&mut app, &ctx, vec![], false);
        assert_eq!(circles.len(), 3);
        // Each layer bottom first: (visible, preview).
        let state = |app: &VectorcraftApp| -> Vec<(bool, bool)> {
            let doc = &app.session.active().unwrap().doc;
            doc.layers.iter().map(|l| (l.visible, !matches!(l.kind, NodeKind::Layer { preview: false, .. }))).collect()
        };
        let last = |app: &VectorcraftApp| app.session.active().unwrap().history.undo.last().map(|u| u.label.clone());
        let undo = |app: &VectorcraftApp| app.session.active().unwrap().history.undo.len();
        let eye = |i: usize| egui::pos2(12.0, circles[i].y);
        let ctrl_alt = egui::Modifiers { alt: true, command: true, ..Default::default() };
        let before = undo(&app);
        // Ctrl/Cmd+Alt-click Layer 1's eye: Layers 2 and 3 go to Outline in one step, and stay visible.
        click_with(&mut app, &ctx, eye(2), ctrl_alt);
        assert_eq!(state(&app), [(true, true), (true, false), (true, false)]);
        assert_eq!(undo(&app), before + 1);
        assert_eq!(last(&app).as_deref(), Some("Outline Others"));
        // Again: every layer previews, in one step.
        click_with(&mut app, &ctx, eye(2), ctrl_alt);
        assert_eq!(state(&app), [(true, true); 3]);
        assert_eq!(undo(&app), before + 2);
        assert_eq!(last(&app).as_deref(), Some("Preview All Layers"));
        // On a layer in Outline whose others preview: the others go to Outline and it previews.
        click_with(&mut app, &ctx, eye(0), egui::Modifiers::COMMAND);
        assert_eq!(state(&app), [(true, true), (true, true), (true, false)]);
        click_with(&mut app, &ctx, eye(0), ctrl_alt);
        assert_eq!(state(&app), [(true, false), (true, false), (true, true)]);
    }

    /// A document with two rectangles on one layer → (app, layer, [bottom, top]).
    fn two_rects() -> (VectorcraftApp, NodeId, [NodeId; 2]) {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        let run = |app: &mut VectorcraftApp, id: &str, p: serde_json::Value| app.session.execute(id, &p).unwrap();
        run(&mut app, "file.new", json!({"width": 200, "height": 200}));
        let a = run(&mut app, "shape.rectangle", json!({"x": 0, "y": 0, "width": 50, "height": 50}))["id"].as_u64().unwrap();
        let b = run(&mut app, "shape.rectangle", json!({"x": 60, "y": 0, "width": 50, "height": 50}))["id"].as_u64().unwrap();
        run(&mut app, "select.none", json!({}));
        let layer = app.session.active().unwrap().doc.layers[0].id;
        (app, layer, [NodeId(a), NodeId(b)])
    }

    #[test]
    fn row_thumbnails_wait_for_the_end_of_a_drag() {
        let (mut app, _, _) = two_rects();
        app.session.execute("select.all", &json!({})).unwrap();
        let ctx = egui::Context::default();
        let renders = || THUMBS.with(|c| c.borrow().renders);
        frame(&mut app, &ctx, vec![], false);
        let first = renders();
        assert!(first >= 1, "the rows' thumbnails render at once");
        frame(&mut app, &ctx, vec![], false);
        assert_eq!(renders(), first, "and stay");
        // A move drag: the layer (and the moved objects) change on every preview.
        app.session.begin_interaction("Move").unwrap();
        for i in 1..=10 {
            app.session.preview("object.transform", &json!({"matrix": [1, 0, 0, 1, i, 0]})).unwrap();
            frame(&mut app, &ctx, vec![], false);
        }
        assert_eq!(renders(), first, "nothing renders while the drag goes on");
        app.session.commit_interaction().unwrap();
        for _ in 0..12 {
            frame(&mut app, &ctx, vec![], false);
        }
        assert!(renders() > first, "the changed rows render after it");
        let after = renders();
        frame(&mut app, &ctx, vec![], false);
        assert_eq!(renders(), after, "once");
    }

    #[test]
    fn clicking_a_target_circle_targets_the_layer() {
        let (mut app, layer, [a, b]) = two_rects();
        let ctx = egui::Context::default();
        // Rows top down: the layer, the top rectangle, the bottom one.
        let (c, _) = frame(&mut app, &ctx, vec![], false);
        assert_eq!(c.len(), 3);
        for e in [egui::Event::PointerMoved(c[0]), button(c[0], true), button(c[0], false)] {
            frame(&mut app, &ctx, vec![e], false);
        }
        let st = app.session.active().unwrap();
        assert_eq!(st.selection.target, Some(layer));
        assert_eq!(st.selection.objects, vec![a, b]);
        assert_eq!(super::super::appearance::object_label(&app), "Layer", "the Appearance panel lists the layer");
        app.session.execute("transparency.set", &json!({"opacity": 25})).unwrap();
        assert!((app.session.active().unwrap().doc.node(layer).unwrap().opacity - 0.25).abs() < 1e-6);
        // The layer's circle is filled now; clicking an object's circle targets that object.
        assert_eq!(filled_targets(&mut app, &ctx), 1);
        for e in [egui::Event::PointerMoved(c[2]), button(c[2], true), button(c[2], false)] {
            frame(&mut app, &ctx, vec![e], false);
        }
        let st = app.session.active().unwrap();
        assert_eq!((st.selection.target, st.selection.objects.clone()), (Some(a), vec![a]));
        // Shift-clicking another object's circle adds it to the selection (#901), and again takes
        // it out; nothing is targeted meanwhile.
        click_with(&mut app, &ctx, c[1], egui::Modifiers::SHIFT);
        let st = app.session.active().unwrap();
        assert_eq!((st.selection.target, st.selection.objects.clone()), (None, vec![a, b]));
        click_with(&mut app, &ctx, c[1], egui::Modifiers::SHIFT);
        assert_eq!(app.session.active().unwrap().selection.objects, vec![a]);
    }

    #[test]
    fn dragging_a_target_circle_moves_or_copies_the_appearance_and_the_trash_clears_it() {
        let (mut app, layer, [a, b]) = two_rects();
        app.session.execute("transparency.set", &json!({"ids": [a.0], "opacity": 30})).unwrap();
        let ctx = egui::Context::default();
        let (c, _) = frame(&mut app, &ctx, vec![], false);
        let opacity = |app: &VectorcraftApp, id: NodeId| app.session.active().unwrap().doc.node(id).unwrap().opacity;
        // Alt-drag the bottom rectangle's circle onto the top one's: both have it.
        drag(&mut app, &ctx, c[2], c[1], true);
        assert!((opacity(&app, b) - 0.3).abs() < 1e-6 && (opacity(&app, a) - 0.3).abs() < 1e-6);
        // A plain drag onto the layer's circle moves it there.
        drag(&mut app, &ctx, c[1], c[0], false);
        assert!((opacity(&app, layer) - 0.3).abs() < 1e-6 && opacity(&app, b) == 1.0);
        // Dropped on the trash, the layer's appearance is cleared (and nothing is deleted).
        let st = app.session.active().unwrap();
        let count = st.doc.node_count();
        let trash = trash_pos(&mut app, &ctx);
        drag(&mut app, &ctx, c[0], trash, false);
        let st = app.session.active().unwrap();
        assert_eq!(st.doc.node(layer).unwrap().opacity, 1.0);
        assert_eq!(st.doc.node_count(), count);
    }

    /// The trash button: the bottom bar's rightmost button.
    fn trash_pos(app: &mut VectorcraftApp, ctx: &egui::Context) -> egui::Pos2 {
        bar_buttons(app, ctx)[0]
    }

    #[test]
    fn dragging_rows_onto_the_trash_deletes_them() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
        app.session.execute("layer.new", &json!({})).unwrap();
        let ctx = egui::Context::default();
        // Rows top first: Layer 2, Layer 1. Grab Layer 2 by its name, left of its target circle.
        let (c, _) = frame(&mut app, &ctx, vec![], false);
        let top = app.session.active().unwrap().doc.layers[1].id;
        let trash = trash_pos(&mut app, &ctx);
        drag(&mut app, &ctx, c[0] - vec2(100.0, 0.0), trash, false);
        let doc = &app.session.active().unwrap().doc;
        assert_eq!(doc.layers.len(), 1);
        assert!(doc.node(top).is_none());
    }

    #[test]
    fn renaming_a_layer_commits_on_enter_and_escape_cancels() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
        let layer = app.session.doc().unwrap().doc.layers[0].id;
        let name = |app: &VectorcraftApp| app.session.doc().unwrap().doc.layers[0].display_name();
        let ctx = egui::Context::default();
        let key = |key| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
        let select_all =
            egui::Event::Key { key: egui::Key::A, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::COMMAND };
        // What a double-click on the row does: the name field opens with the name.
        let open = |app: &mut VectorcraftApp| {
            ctx.data_mut(|d| d.insert_temp(egui::Id::new("layers-rename"), (layer.0, name(app))));
            frame(app, &ctx, vec![], false);
            frame(app, &ctx, vec![], false);
        };
        open(&mut app);
        frame(&mut app, &ctx, vec![select_all.clone(), egui::Event::Text("Sky".into())], false);
        frame(&mut app, &ctx, vec![key(egui::Key::Enter)], false);
        frame(&mut app, &ctx, vec![], false);
        assert_eq!(name(&app), "Sky", "Enter keeps the new name");
        assert!(ctx.data(|d| d.get_temp::<(u64, String)>(egui::Id::new("layers-rename"))).is_none(), "and closes the field");
        open(&mut app);
        frame(&mut app, &ctx, vec![select_all, egui::Event::Text("Ground".into())], false);
        frame(&mut app, &ctx, vec![key(egui::Key::Escape)], false);
        frame(&mut app, &ctx, vec![], false);
        assert_eq!(name(&app), "Sky", "Escape cancels");
    }

    #[test]
    fn an_open_rename_does_not_follow_into_another_document() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
        let layer = app.session.doc().unwrap().doc.layers[0].id;
        let ctx = egui::Context::default();
        frame(&mut app, &ctx, vec![], false);
        ctx.data_mut(|d| d.insert_temp(egui::Id::new("layers-rename"), (layer.0, "Renamed in the first".to_string())));
        frame(&mut app, &ctx, vec![], false);
        // A second document, whose first layer has the same node id, comes to the front.
        app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
        assert_eq!(app.session.doc().unwrap().doc.layers[0].id, layer);
        let enter = egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
        frame(&mut app, &ctx, vec![], false);
        frame(&mut app, &ctx, vec![enter], false);
        frame(&mut app, &ctx, vec![], false);
        assert_eq!(app.session.doc().unwrap().doc.layers[0].display_name(), "Layer 1");
        assert!(ctx.data(|d| d.get_temp::<(u64, String)>(egui::Id::new("layers-rename"))).is_none());
    }

    #[test]
    fn masks_underline_dashed_and_mask_editing_lists_only_the_mask() {
        let (mut app, _, [a, _]) = two_rects();
        let ctx = egui::Context::default();
        let (_, before) = frame(&mut app, &ctx, vec![], false);
        app.session.execute("select.set", &json!({"ids": [a.0]})).unwrap();
        app.session.execute("transparency.makeOpacityMask", &json!({})).unwrap();
        // Editing the new (empty) mask: one row, `<Opacity Mask>`.
        let (c, _) = frame(&mut app, &ctx, vec![], false);
        assert_eq!(c.len(), 1);
        let texts = super::super::tests_appearance::frame_events(&ctx, &mut app, vec![], show);
        assert!(texts.iter().any(|(t, _)| t == "<Opacity Mask>"), "{texts:?}");
        app.session.execute("transparency.stopEditingOpacityMask", &json!({})).unwrap();
        let (c, after) = frame(&mut app, &ctx, vec![], false);
        assert_eq!(c.len(), 3);
        assert!(after > before + 1, "a dashed underline: {before} → {after}");
    }

    // ---- rows at every depth (layers, sublayers, groups, objects) ----

    /// Layer 1 holding rectangle `c` and a sublayer holding rectangle `a` and a group of
    /// rectangle `b`, every row open, nothing selected → (app, [layer, sub, g, b, a, c]): the rows
    /// top down.
    fn nested() -> (VectorcraftApp, [NodeId; 6]) {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        let run = |app: &mut VectorcraftApp, id: &str, p: serde_json::Value| app.session.execute(id, &p).unwrap();
        let id = |v: serde_json::Value| NodeId(v["id"].as_u64().unwrap());
        run(&mut app, "file.new", json!({"width": 400, "height": 400}));
        let layer = app.session.active().unwrap().doc.layers[0].id;
        let c = id(run(&mut app, "shape.rectangle", json!({"x": 300, "y": 0, "width": 20, "height": 20})));
        let sub = id(run(&mut app, "layer.newSublayer", json!({"name": "Sub"})));
        let a = id(run(&mut app, "shape.rectangle", json!({"x": 0, "y": 0, "width": 20, "height": 20})));
        let b = id(run(&mut app, "shape.rectangle", json!({"x": 100, "y": 0, "width": 20, "height": 20})));
        run(&mut app, "select.set", json!({"ids": [b.0]}));
        let g = id(run(&mut app, "object.group", json!({})));
        run(&mut app, "select.none", json!({}));
        expand(&mut app, &json!({})).unwrap();
        (app, [layer, sub, g, b, a, c])
    }

    /// Press and release at `at` with `mods` held, over three frames.
    fn click_with(app: &mut VectorcraftApp, ctx: &egui::Context, at: egui::Pos2, mods: egui::Modifiers) {
        for e in [egui::Event::PointerMoved(at), button(at, true), button(at, false)] {
            let mut out =
                ctx.run_ui(egui::RawInput { events: vec![egui::Event::ModifiersChanged(mods), e], ..Default::default() }, |ui| show(app, ui));
            out.textures_delta.clear();
        }
    }

    fn rows_of(app: &VectorcraftApp) -> Vec<NodeId> {
        app.session.active().unwrap().highlighted_rows()
    }

    #[test]
    fn rows_at_every_depth_are_clickable() {
        let (mut app, [layer, sub, g, b, a, c]) = nested();
        let ctx = egui::Context::default();
        let (circles, _) = frame(&mut app, &ctx, vec![], false);
        assert_eq!(circles.len(), 6, "a row for the layer, the sublayer, the group, both rectangles in it and the one on the layer");
        let name = |i: usize| egui::pos2(150.0, circles[i].y);
        for (i, row, current) in [(1, sub, sub), (2, g, sub), (3, b, sub), (4, a, sub), (5, c, layer), (0, layer, layer)] {
            click_with(&mut app, &ctx, name(i), egui::Modifiers::NONE);
            let st = app.session.active().unwrap();
            assert_eq!((st.highlighted_rows(), st.current_layer()), (vec![row], Some(current)), "row {i}");
        }
        // New art goes into the current sublayer once its row is clicked.
        click_with(&mut app, &ctx, name(1), egui::Modifiers::NONE);
        let n = app.session.execute("shape.ellipse", &json!({"x": 0, "y": 100, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap();
        assert_eq!(app.session.active().unwrap().doc.parent_of(NodeId(n)), Some(sub));
        app.session.execute("edit.undo", &json!({})).unwrap();
        // Shift-click highlights the range down to the row; Ctrl/Cmd-click toggles one.
        click_with(&mut app, &ctx, name(4), egui::Modifiers::SHIFT);
        assert_eq!(rows_of(&app), vec![sub, g, b, a]);
        click_with(&mut app, &ctx, name(2), egui::Modifiers::COMMAND);
        assert_eq!(rows_of(&app), vec![sub, b, a]);
        click_with(&mut app, &ctx, name(5), egui::Modifiers::COMMAND);
        assert_eq!(rows_of(&app), vec![sub, b, a, c]);
        // Delete with the panel's keyboard: the highlighted rows go.
        let del = egui::Event::Key { key: egui::Key::Delete, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
        frame(&mut app, &ctx, vec![del], false);
        let d = &app.session.active().unwrap().doc;
        assert!(d.node(sub).is_none() && d.node(c).is_none());
        assert_eq!(d.layers.len(), 1);
    }

    /// A layer's ruler guides are rows in it, above its art: a click selects the guide, a drag onto
    /// another layer's row moves it there, and a guide on a locked layer is left alone.
    #[test]
    fn guide_rows_select_and_move_their_guides() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        let run = |app: &mut VectorcraftApp, id: &str, p: serde_json::Value| app.session.execute(id, &p).unwrap();
        run(&mut app, "file.new", json!({"width": 200, "height": 200}));
        run(&mut app, "shape.rectangle", json!({"x": 0, "y": 0, "width": 20, "height": 20}));
        run(&mut app, "guide.add", json!({"vertical": true, "pos": 50}));
        run(&mut app, "select.none", json!({}));
        let second = NodeId(run(&mut app, "layer.new", json!({}))["id"].as_u64().unwrap());
        expand(&mut app, &json!({})).unwrap();
        let ctx = egui::Context::default();
        // Rows top down: Layer 2, Layer 1, its guide (no target circle), the rectangle.
        let (circles, _) = frame(&mut app, &ctx, vec![], false);
        assert_eq!(circles.len(), 3);
        let guide_row = egui::pos2(150.0, (circles[1].y + circles[2].y) / 2.0);
        let guides = |app: &VectorcraftApp| app.session.active().unwrap().selection.guides.clone();
        click_with(&mut app, &ctx, guide_row, egui::Modifiers::NONE);
        assert_eq!(guides(&app), [0], "its row selects it");
        // Dragged onto Layer 2's row, it goes on Layer 2, in one undo step.
        let undo = app.session.active().unwrap().history.undo.len();
        drag(&mut app, &ctx, guide_row, egui::pos2(150.0, circles[0].y), false);
        let st = app.session.active().unwrap();
        assert_eq!((st.doc.guides[0].layer, st.history.undo.len()), (Some(second), undo + 1));
        // Now under Layer 2: locked, its row neither selects nor drags it.
        run(&mut app, "layer.setProps", json!({"ids": [second.0], "locked": true}));
        assert!(guides(&app).is_empty(), "deselected with its layer locked");
        let (circles, _) = frame(&mut app, &ctx, vec![], false);
        let guide_row = egui::pos2(150.0, (circles[0].y + circles[1].y) / 2.0);
        click_with(&mut app, &ctx, guide_row, egui::Modifiers::NONE);
        assert!(guides(&app).is_empty());
        drag(&mut app, &ctx, guide_row, egui::pos2(150.0, circles[2].y), false);
        assert_eq!(app.session.active().unwrap().doc.guides[0].layer, Some(second));
    }

    /// The selection squares drawn: (big ones on rows whose object is selected, small ones on rows
    /// holding selected art).
    fn squares(app: &mut VectorcraftApp, ctx: &egui::Context) -> (usize, usize) {
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| show(app, ui));
        out.textures_delta.clear();
        let size = |w: f32| {
            out.shapes.iter().filter(|c| matches!(&c.shape, egui::Shape::Rect(r) if (r.rect.width() - w).abs() < 0.01 && (r.rect.height() - w).abs() < 0.01 && r.fill != Color32::TRANSPARENT)).count()
        };
        (size(7.0), size(4.0))
    }

    #[test]
    fn selection_squares_show_and_select_a_rows_art() {
        let (mut app, [layer, sub, g, b, a, c]) = nested();
        let ctx = egui::Context::default();
        assert_eq!(squares(&mut app, &ctx), (0, 0));
        app.session.execute("select.set", &json!({"ids": [g.0]})).unwrap();
        assert_eq!(squares(&mut app, &ctx), (1, 2), "the group's square, and small ones on the sublayer and the layer");
        let (circles, _) = frame(&mut app, &ctx, vec![], false);
        let square = |i: usize| circles[i] + vec2(TARGET_X - SQUARE_X, 0.0);
        let sel = |app: &VectorcraftApp| app.session.active().unwrap().selection.objects.clone();
        click_with(&mut app, &ctx, square(4), egui::Modifiers::NONE);
        assert_eq!(sel(&app), vec![a]);
        click_with(&mut app, &ctx, square(5), egui::Modifiers::SHIFT);
        assert_eq!(sel(&app), vec![a, c]);
        click_with(&mut app, &ctx, square(1), egui::Modifiers::NONE);
        assert_eq!(sel(&app), vec![a, g], "a sublayer's square selects its art");
        click_with(&mut app, &ctx, square(0), egui::Modifiers::NONE);
        assert_eq!(sel(&app), vec![c, a, g], "a layer's, its sublayers' too");
        let _ = (layer, sub, b);
    }

    #[test]
    fn eye_and_lock_toggle_nested_rows_and_drag_down_the_column() {
        let (mut app, [_, sub, g, b, a, _]) = nested();
        let ctx = egui::Context::default();
        let (circles, _) = frame(&mut app, &ctx, vec![], false);
        let (eye, lock) = (|i: usize| egui::pos2(12.0, circles[i].y), |i: usize| egui::pos2(37.0, circles[i].y));
        let node = |app: &VectorcraftApp, id: NodeId| app.session.active().unwrap().doc.node(id).unwrap().clone();
        click_with(&mut app, &ctx, eye(3), egui::Modifiers::NONE);
        assert!(!node(&app, b).visible, "an object in a group in a sublayer hides");
        click_with(&mut app, &ctx, eye(3), egui::Modifiers::NONE);
        click_with(&mut app, &ctx, lock(2), egui::Modifiers::NONE);
        assert!(node(&app, g).locked, "a group locks");
        click_with(&mut app, &ctx, lock(2), egui::Modifiers::NONE);
        // Ctrl/Cmd-click a layer's eye: Outline, and back.
        click_with(&mut app, &ctx, eye(1), egui::Modifiers::COMMAND);
        assert!(matches!(node(&app, sub).kind, NodeKind::Layer { preview: false, .. }));
        click_with(&mut app, &ctx, eye(1), egui::Modifiers::COMMAND);
        assert!(matches!(node(&app, sub).kind, NodeKind::Layer { preview: true, .. }));
        // Drag down the eye column from the group to the rectangle below: all three hide, one step.
        let undo = |app: &VectorcraftApp| app.session.active().unwrap().history.undo.len();
        let before = undo(&app);
        let steps = [
            vec![egui::Event::PointerMoved(eye(2))],
            vec![button(eye(2), true)],
            vec![egui::Event::PointerMoved(eye(2) + vec2(0.0, 6.0))],
            vec![egui::Event::PointerMoved(eye(3))],
            vec![egui::Event::PointerMoved(eye(4))],
            vec![button(eye(4), false)],
            vec![],
        ];
        for e in steps {
            frame(&mut app, &ctx, e, false);
        }
        assert!(!node(&app, g).visible && !node(&app, b).visible && !node(&app, a).visible);
        assert!(node(&app, sub).visible, "rows it didn't pass stay");
        assert_eq!(undo(&app), before + 1, "one undo step");
        assert!(!app.session.in_interaction());
        app.session.execute("edit.undo", &json!({})).unwrap();
        assert!(node(&app, g).visible && node(&app, b).visible && node(&app, a).visible);
    }

    #[test]
    fn triangles_open_and_close_rows_and_alt_does_everything_inside() {
        let (mut app, [layer, sub, ..]) = nested();
        let ctx = egui::Context::default();
        let count = |app: &mut VectorcraftApp| frame(app, &ctx, vec![], false).0.len();
        assert_eq!(count(&mut app), 6);
        let (circles, _) = frame(&mut app, &ctx, vec![], false);
        let triangle = |depth: f32, y: f32| egui::pos2(2.0 * COLUMN + 8.0 + depth * INDENT + 7.0, y);
        // Close the sublayer: its group, the group's rectangle and its own rectangle go.
        click_with(&mut app, &ctx, triangle(1.0, circles[1].y), egui::Modifiers::NONE);
        assert_eq!(count(&mut app), 3);
        // Alt-click the layer's: everything inside closes, and opens again.
        click_with(&mut app, &ctx, triangle(0.0, circles[0].y), egui::Modifiers::ALT);
        assert_eq!(count(&mut app), 1);
        click_with(&mut app, &ctx, triangle(0.0, circles[0].y), egui::Modifiers::ALT);
        assert_eq!(count(&mut app), 6);
        // Show Layers Only (Panel Options).
        app.ui.layers_panel.layers_only = true;
        assert_eq!(count(&mut app), 2);
        app.ui.layers_panel = PanelOptions { row_size: ROW_LARGE, ..Default::default() };
        let (c, _) = frame(&mut app, &ctx, vec![], false);
        assert!((c[1].y - c[0].y - ROW_LARGE).abs() < 0.5, "large rows");
        let _ = (layer, sub);
    }

    // ---- Search All and the Filter ----

    /// Layer 1 holding a path, a rectangle, type reading "Sky line", a group of an ellipse with an
    /// effect, a compound path, a guide, an embedded and a linked image and a ruler guide, and on top
    /// a sublayer "Sub" holding a hidden path named "Sky" and a locked path; only Layer 1 open and
    /// nothing selected → (app, the rows' ids by name).
    fn kinds() -> (VectorcraftApp, HashMap<&'static str, NodeId>) {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        let run = |app: &mut VectorcraftApp, id: &str, p: serde_json::Value| app.session.execute(id, &p).unwrap();
        let id = |v: serde_json::Value| NodeId(v["id"].as_u64().unwrap());
        let star = |app: &mut VectorcraftApp, x: u32| id(run(app, "shape.star", json!({"cx": x, "cy": 50, "radius1": 20, "radius2": 10})));
        run(&mut app, "file.new", json!({"width": 400, "height": 400}));
        let mut ids = HashMap::from([("layer", app.session.active().unwrap().doc.layers[0].id)]);
        ids.insert("path", star(&mut app, 50));
        ids.insert("rect", id(run(&mut app, "shape.rectangle", json!({"x": 0, "y": 100, "width": 20, "height": 20}))));
        ids.insert("text", id(run(&mut app, "text.create", json!({"x": 10, "y": 200, "text": "Sky line"}))));
        let ellipse = id(run(&mut app, "shape.ellipse", json!({"x": 100, "y": 100, "width": 20, "height": 20})));
        run(&mut app, "effect.apply", json!({"effect": "distort.twist", "ids": [ellipse.0]}));
        run(&mut app, "select.set", json!({"ids": [ellipse.0]}));
        ids.insert("group", id(run(&mut app, "object.group", json!({}))));
        ids.insert("ellipse", ellipse);
        let parts = [star(&mut app, 150), star(&mut app, 160)];
        run(&mut app, "select.set", json!({"ids": [parts[0].0, parts[1].0]}));
        ids.insert("compound", id(run(&mut app, "object.compoundPath.make", json!({}))));
        let guide = id(run(&mut app, "shape.arc", json!({"x1": 0, "y1": 300, "x2": 50, "y2": 350})));
        run(&mut app, "select.set", json!({"ids": [guide.0]}));
        run(&mut app, "view.guides.make", json!({}));
        ids.insert("guide", guide);
        run(&mut app, "guide.add", json!({"vertical": true, "pos": 50}));
        let mut png = vec![];
        image::RgbaImage::from_pixel(4, 4, image::Rgba([200, 40, 40, 255]))
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let placed = |app: &mut VectorcraftApp, p: serde_json::Value| NodeId(run(app, "file.place", p)["ids"][0].as_u64().unwrap());
        ids.insert("embedded", placed(&mut app, json!({"name": "pic.png", "dataBase64": vectorcraft_format::base64_encode(&png)})));
        // One file for every test that calls this, written by the first.
        static LINKED: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();
        let file = LINKED.get_or_init(|| {
            let file = vectorcraft_testkit::temp_dir("layers-filter").join("linked.png");
            std::fs::write(&file, &png).unwrap();
            file
        });
        ids.insert("linked", placed(&mut app, json!({"path": file.to_string_lossy()})));
        ids.insert("sub", id(run(&mut app, "layer.newSublayer", json!({"name": "Sub"}))));
        let hidden = star(&mut app, 250);
        run(&mut app, "layer.setProps", json!({"ids": [hidden.0], "name": "Sky", "visible": false}));
        let locked = star(&mut app, 300);
        run(&mut app, "layer.setProps", json!({"ids": [locked.0], "locked": true}));
        ids.extend([("hidden", hidden), ("locked", locked)]);
        run(&mut app, "select.none", json!({}));
        (app, ids)
    }

    /// The rows [`list_rows`] lists for `query` and `filter`, by their names in `ids` ("ruler guide"
    /// for a guide's row).
    fn listed(app: &VectorcraftApp, ids: &HashMap<&'static str, NodeId>, query: &str, filter: &Filter) -> Vec<&'static str> {
        let st = app.session.active().unwrap();
        let rows = list_rows(&st.doc, &st.layers_open, &PanelOptions::default(), query, filter);
        let name = |r: &Row| match r.guide {
            Some(_) => "ruler guide",
            None => ids.iter().find(|(_, id)| **id == r.node.id).map_or("?", |(name, _)| *name),
        };
        rows.iter().map(name).collect()
    }

    fn only(k: ObjectKind) -> Filter {
        Filter { objects: vec![k], layers: vec![] }
    }

    /// Each object kind keeps exactly the rows of that kind, with the rows holding them, open or
    /// closed.
    #[test]
    fn each_object_kind_keeps_its_rows_and_the_rows_holding_them() {
        let (app, ids) = kinds();
        for (k, want) in [
            (ObjectKind::Paths, vec!["layer", "sub", "locked", "hidden", "compound", "path"]),
            (ObjectKind::Shapes, vec!["layer", "group", "ellipse", "rect"]),
            (ObjectKind::Text, vec!["layer", "text"]),
            (ObjectKind::Images, vec!["layer", "linked", "embedded"]),
            (ObjectKind::Effects, vec!["layer", "group", "ellipse"]),
            (ObjectKind::Groups, vec!["layer", "group"]),
            (ObjectKind::LinkedFiles, vec!["layer", "linked"]),
            (ObjectKind::Guides, vec!["layer", "ruler guide", "guide"]),
        ] {
            assert_eq!(listed(&app, &ids, "", &only(k)), want, "{k:?}");
        }
    }

    /// By Layer keeps the rows that are locked or hidden themselves: in a hidden, locked sublayer a
    /// row stays only for its own lock or eye. Two choices in a section take either; a choice in
    /// each section takes rows that are both.
    #[test]
    fn row_states_keep_locked_or_hidden_rows_and_combine_with_kinds() {
        let (mut app, ids) = kinds();
        let f = |objects: Vec<ObjectKind>, layers: Vec<RowState>| Filter { objects, layers };
        assert_eq!(listed(&app, &ids, "", &f(vec![], vec![RowState::Hidden])), ["layer", "sub", "hidden"]);
        assert_eq!(listed(&app, &ids, "", &f(vec![], vec![RowState::Locked])), ["layer", "sub", "locked"]);
        assert_eq!(listed(&app, &ids, "", &f(vec![], vec![RowState::Locked, RowState::Hidden])), ["layer", "sub", "locked", "hidden"]);
        assert_eq!(listed(&app, &ids, "", &f(vec![ObjectKind::Text, ObjectKind::Images], vec![])), ["layer", "linked", "embedded", "text"]);
        assert_eq!(listed(&app, &ids, "", &f(vec![ObjectKind::Paths], vec![RowState::Hidden])), ["layer", "sub", "hidden"]);
        assert!(listed(&app, &ids, "", &f(vec![ObjectKind::Shapes], vec![RowState::Hidden])).is_empty());
        // Once Sub is hidden and locked, the rows in it stay only for their own lock or eye.
        app.session.execute("layer.setProps", &json!({"ids": [ids["sub"].0], "visible": false, "locked": true})).unwrap();
        assert_eq!(listed(&app, &ids, "", &f(vec![], vec![RowState::Hidden])), ["layer", "sub", "hidden"]);
        assert_eq!(listed(&app, &ids, "", &f(vec![], vec![RowState::Locked])), ["layer", "sub", "locked"]);
    }

    /// The filter and the search combine: a row stays when its name holds the text and the filter
    /// keeps it, or when it holds such a row.
    #[test]
    fn the_filter_combines_with_the_search() {
        let (app, ids) = kinds();
        assert_eq!(listed(&app, &ids, "sky", &Filter::default()), ["layer", "sub", "hidden", "text"]);
        assert_eq!(listed(&app, &ids, "sky", &only(ObjectKind::Text)), ["layer", "text"]);
        assert_eq!(listed(&app, &ids, "sky", &Filter { objects: vec![], layers: vec![RowState::Hidden] }), ["layer", "sub", "hidden"]);
        assert!(listed(&app, &ids, "sky", &only(ObjectKind::Groups)).is_empty());
    }

    /// (target circles, lit 24 pt buttons) one frame of the panel draws.
    fn circles_and_lit_buttons(app: &mut VectorcraftApp, ctx: &egui::Context) -> (usize, usize) {
        let lit = Tokens::get(ctx).tool_active;
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| show(app, ui));
        out.textures_delta.clear();
        let circles = out.shapes.iter().filter(|c| matches!(&c.shape, egui::Shape::Circle(cs) if cs.radius == 5.0)).count();
        let buttons =
            out.shapes.iter().filter(|c| matches!(&c.shape, egui::Shape::Rect(r) if r.fill == lit && (r.rect.width() - 24.0).abs() < 0.01)).count();
        (circles, buttons)
    }

    /// `ui.layersFilter` has the panel list its rows again at once and lights the Filter button
    /// while a filter is on; clearing it lists every row again. The document, its open rows and the
    /// selection stay as they were.
    #[test]
    fn the_panel_lists_its_rows_again_when_the_filter_changes() {
        let (mut app, ids) = kinds();
        app.session.execute("select.set", &json!({"ids": [ids["rect"].0]})).unwrap();
        let ctx = egui::Context::default();
        let before = circles_and_lit_buttons(&mut app, &ctx);
        assert_eq!(before.1, 0, "the Filter button is not lit");
        let st = app.session.active().unwrap();
        let (revision, open) = (st.revision, st.layers_open.generation());
        let r = app.run("ui.layersFilter", json!({"objects": ["text"]})).unwrap();
        assert_eq!(r, json!({"objects": ["text"], "layers": []}));
        assert_eq!(circles_and_lit_buttons(&mut app, &ctx), (2, 1), "Layer 1 and the type; the button is lit");
        app.run("ui.layersFilter", json!({"layers": ["hidden"]})).unwrap();
        assert_eq!(circles_and_lit_buttons(&mut app, &ctx), (0, 1), "no type is hidden");
        app.run("ui.layersFilter", json!({"objects": [], "layers": []})).unwrap();
        assert_eq!(circles_and_lit_buttons(&mut app, &ctx), before, "every row again");
        let st = app.session.active().unwrap();
        assert_eq!((st.revision, st.layers_open.generation(), st.selection.objects.clone()), (revision, open, vec![ids["rect"]]));
    }

    /// Clicks in the Filter button's popup tick a kind and a state, and a second click clears the
    /// kind, while the popup stays open; Clear All turns the filter off.
    #[test]
    fn the_filter_popup_ticks_kinds_and_states_and_clear_all_turns_the_filter_off() {
        let (mut app, _) = kinds();
        let ctx = egui::Context::default();
        // The Filter button: the 24 pt square lit while a filter is on.
        app.run("ui.layersFilter", json!({"layers": ["locked"]})).unwrap();
        let lit = Tokens::get(&ctx).tool_active;
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| show(&mut app, ui));
        out.textures_delta.clear();
        let filter = out
            .shapes
            .iter()
            .find_map(|c| match &c.shape {
                egui::Shape::Rect(r) if r.fill == lit && (r.rect.width() - 24.0).abs() < 0.01 => Some(r.rect.center()),
                _ => None,
            })
            .unwrap();
        app.run("ui.layersFilter", json!({"layers": []})).unwrap();
        click_with(&mut app, &ctx, filter, egui::Modifiers::NONE);
        // Where the popup draws `label` (after a frame for it to settle).
        let at = |app: &mut VectorcraftApp, label: &str| {
            super::super::tests_appearance::frame_events(&ctx, app, vec![], show);
            let texts = super::super::tests_appearance::frame_events(&ctx, app, vec![], show);
            texts.iter().find(|(t, _)| t == label).map(|(_, r)| r.center()).unwrap_or_else(|| panic!("{label} not in {texts:?}"))
        };
        let text = at(&mut app, "Text");
        click_with(&mut app, &ctx, text, egui::Modifiers::NONE);
        assert_eq!(app.ui.layers_filter, Filter { objects: vec![ObjectKind::Text], layers: vec![] });
        let hidden = at(&mut app, "Hidden");
        click_with(&mut app, &ctx, hidden, egui::Modifiers::NONE);
        assert_eq!(app.ui.layers_filter, Filter { objects: vec![ObjectKind::Text], layers: vec![RowState::Hidden] });
        click_with(&mut app, &ctx, text, egui::Modifiers::NONE);
        assert_eq!(app.ui.layers_filter, Filter { objects: vec![], layers: vec![RowState::Hidden] }, "a second click takes it out");
        let clear = at(&mut app, "Clear All");
        click_with(&mut app, &ctx, clear, egui::Modifiers::NONE);
        assert!(!app.ui.layers_filter.is_on());
    }

    /// `ui.layersFilter` reads its lists by name in the popup's order, keeps a list it is not given,
    /// and changes nothing when a list holds anything else.
    #[test]
    fn the_filter_command_reads_its_lists_by_name() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        let r = app.run("ui.layersFilter", json!({"objects": ["guides", "paths", "paths"], "layers": ["hidden"]})).unwrap();
        assert_eq!(r, json!({"objects": ["paths", "guides"], "layers": ["hidden"]}));
        let r = app.run("ui.layersFilter", json!({"layers": []})).unwrap();
        assert_eq!(r, json!({"objects": ["paths", "guides"], "layers": []}));
        for bad in [json!({"objects": ["paths", "layers"]}), json!({"objects": "paths"}), json!({"objects": [], "layers": ["visible"]})] {
            let e = app.run("ui.layersFilter", bad.clone()).unwrap_err();
            let key = if bad["layers"].is_null() { "objects" } else { "layers" };
            assert!(e.starts_with(&format!("`{key}` must be an array of these names: ")), "{bad}: {e}");
        }
        assert_eq!(app.run("ui.layersFilter", json!({})).unwrap(), json!({"objects": ["paths", "guides"], "layers": []}), "unchanged");
    }

    #[test]
    fn dragging_rows_moves_them_into_other_layers_and_alt_copies() {
        let (mut app, [layer, sub, g, _, a, c]) = nested();
        let ctx = egui::Context::default();
        let l2 = NodeId(app.session.execute("layer.new", &json!({"top": true, "name": "Top"})).unwrap()["id"].as_u64().unwrap());
        // Rows: Top, Layer 1, Sub, g, b, a, c.
        let (circles, _) = frame(&mut app, &ctx, vec![], false);
        assert_eq!(circles.len(), 7);
        let name = |i: usize| egui::pos2(150.0, circles[i].y);
        let parent = |app: &VectorcraftApp, id: NodeId| app.session.active().unwrap().doc.parent_of(id);
        // Rectangle a onto the middle of the Top layer's row: into it.
        drag(&mut app, &ctx, name(5), name(0), false);
        assert_eq!(parent(&app, a), Some(l2));
        // Rows: Top, a, Layer 1, Sub, g, b, c. The sublayer onto the upper edge of Layer 1's row:
        // above it, a top-level layer now.
        let (circles, _) = frame(&mut app, &ctx, vec![], false);
        assert_eq!(circles.len(), 7);
        drag(&mut app, &ctx, egui::pos2(150.0, circles[3].y), egui::pos2(150.0, circles[2].y - ROW_MEDIUM * 0.4), false);
        assert_eq!(parent(&app, sub), None);
        let d = &app.session.active().unwrap().doc;
        assert_eq!(d.layers.iter().map(|l| l.id).collect::<Vec<_>>(), vec![layer, sub, l2]);
        // Rows: Top, a, Sub, g, b, Layer 1, c. Alt-drag the group into Layer 1: a copy goes there,
        // the group stays.
        let (circles, _) = frame(&mut app, &ctx, vec![], false);
        let count = app.session.active().unwrap().doc.node_count();
        drag(&mut app, &ctx, egui::pos2(150.0, circles[3].y), egui::pos2(150.0, circles[5].y), true);
        assert_eq!(parent(&app, g), Some(sub));
        assert_eq!(app.session.active().unwrap().doc.node_count(), count + 2, "the group and its rectangle, copied");
        let _ = (layer, c);
    }

    /// The centres of the bottom bar's buttons, right to left from under the right end of its top
    /// divider: Delete, New Layer, New Sublayer, Make/Release Clipping Mask, Locate Object.
    fn bar_buttons(app: &mut VectorcraftApp, ctx: &egui::Context) -> Vec<egui::Pos2> {
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| show(app, ui));
        out.textures_delta.clear();
        let divider = Tokens::get(ctx).divider;
        let corner = out
            .shapes
            .iter()
            .rev()
            .find_map(|c| match &c.shape {
                egui::Shape::LineSegment { points, stroke } if stroke.color == divider => Some(points[1]),
                _ => None,
            })
            .unwrap();
        let step = 24.0 + ctx.global_style().spacing.item_spacing.x;
        (0..5).map(|k| corner + vec2(-12.0 - k as f32 * step, 15.0)).collect()
    }

    /// How many texts reading `s` the frame after `events` paints.
    fn texts(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<egui::Event>, s: &str) -> usize {
        let mut out = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| show(app, ui));
        out.textures_delta.clear();
        out.shapes.iter().filter(|c| matches!(&c.shape, egui::Shape::Text(t) if t.galley.text() == s)).count()
    }

    /// Drag the row at `a` to `b` and drop it there. Returns how many times its `name` was painted
    /// while it hung over `b`.
    fn drag_row(app: &mut VectorcraftApp, ctx: &egui::Context, a: egui::Pos2, b: egui::Pos2, name: &str) -> usize {
        frame(app, ctx, vec![egui::Event::PointerMoved(a)], false);
        frame(app, ctx, vec![button(a, true)], false);
        frame(app, ctx, vec![egui::Event::PointerMoved(a + vec2(0.0, 6.0))], false);
        let shown = texts(app, ctx, vec![egui::Event::PointerMoved(b)], name);
        frame(app, ctx, vec![button(b, false)], false);
        frame(app, ctx, vec![], false);
        shown
    }

    #[test]
    fn dropping_rows_on_new_layer_duplicates_them() {
        let (mut app, first, _) = two_rects();
        let second = app.session.execute("layer.new", &json!({"name": "Top"})).unwrap()["id"].as_u64().map(NodeId).unwrap();
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let layers = |app: &VectorcraftApp| app.session.active().unwrap().doc.layers.iter().map(|l| l.id).collect::<Vec<_>>();
        // Rows top down: Top, Layer 1 (expanded: its two rectangles).
        let (c, _) = frame(&mut app, &ctx, vec![], false);
        let buttons = bar_buttons(&mut app, &ctx);
        // Layer 1 dropped on New Layer: a copy of it right above it.
        let shown = drag_row(&mut app, &ctx, egui::pos2(150.0, c[1].y), buttons[1], "Layer 1");
        assert_eq!(shown, 2, "the row and its ghost under the pointer");
        let after = layers(&app);
        assert_eq!(after.len(), 3);
        assert_eq!((after[0], after[2]), (first, second));
        assert_eq!(app.session.active().unwrap().doc.node(after[1]).unwrap().children().map(Vec::len), Some(2));
        assert_eq!(texts(&mut app, &ctx, vec![], "Layer 1"), 1, "the ghost is gone after the drop");
        // One undo step takes the copy away.
        app.session.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(layers(&app), vec![first, second]);
        // The other bottom-bar buttons are no drop targets.
        drag_row(&mut app, &ctx, egui::pos2(150.0, c[0].y), buttons[2], "Top");
        assert_eq!(layers(&app), vec![first, second]);
    }

    #[test]
    fn the_selected_art_square_drags_the_selection_to_another_layer() {
        let (mut app, [_, sub, g, _, a, c]) = nested();
        let ctx = egui::Context::default();
        app.session.execute("select.set", &json!({"ids": [a.0, g.0]})).unwrap();
        let (circles, _) = frame(&mut app, &ctx, vec![], false);
        let square = |i: usize| circles[i] + vec2(TARGET_X - SQUARE_X, 0.0);
        // From the sublayer's (small) square onto Layer 1's row.
        drag(&mut app, &ctx, square(1), egui::pos2(150.0, circles[0].y), false);
        let d = &app.session.active().unwrap().doc;
        assert_eq!(d.node(d.layers[0].id).unwrap().children().unwrap().iter().map(|n| n.id).collect::<Vec<_>>(), vec![c, sub, a, g]);
        assert_eq!(app.session.active().unwrap().selection.objects.len(), 2, "still selected");
    }

    #[test]
    fn the_panel_menu_lists_every_operation() {
        let (mut app, _) = nested();
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let texts = super::super::tests_appearance::frame_events(&ctx, &mut app, vec![], menu);
        for label in [
            "New Layer…",
            "New Sublayer…",
            "Duplicate Selection",
            "Delete Selection",
            "Options for Selection…",
            "Make/Release Clipping Mask",
            "Enter Isolation Mode",
            "Locate Object",
            "Merge Selected",
            "Flatten Artwork",
            "Collect in New Layer",
            "Collect for Export",
            "Release to Layers (Sequence)",
            "Release to Layers (Build)",
            "Reverse Order",
            "Template",
            "Show All Layers",
            "Preview All Layers",
            "Unlock All Layers",
            "Paste Remembers Layers",
            "Panel Options…",
        ] {
            assert!(texts.iter().any(|(t, _)| t.trim() == label), "{label} missing from {texts:?}");
        }
    }

    /// With the dock collapsed, Layers shows in a floating flyout: its rows work there too.
    #[test]
    fn rows_work_in_the_floating_flyout() {
        let (mut app, [_, sub, ..]) = nested();
        app.ui.dock_collapsed = true;
        app.ui.open_panel = Some("layers".into());
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(1600.0, 1000.0));
        let mut time = 0.0;
        let mut frame = |app: &mut VectorcraftApp, events: Vec<egui::Event>| {
            time += 0.05;
            let input = egui::RawInput { time: Some(time), events, screen_rect: Some(screen), ..Default::default() };
            let mut out = ctx.run_ui(input, |ui| crate::dock::floating_panel(app, ui.ctx()));
            out.textures_delta.clear();
            let mut c: Vec<egui::Pos2> = out
                .shapes
                .iter()
                .filter_map(|c| match &c.shape {
                    egui::Shape::Circle(cs) if cs.radius == 5.0 => Some(cs.center),
                    _ => None,
                })
                .collect();
            c.sort_by(|a, b| a.y.total_cmp(&b.y));
            c
        };
        for _ in 0..3 {
            frame(&mut app, vec![]);
        }
        let circles = frame(&mut app, vec![]);
        assert_eq!(circles.len(), 6, "every row shows in the flyout");
        assert!(circles.iter().all(|c| screen.contains(*c)));
        let at = circles[1] - vec2(120.0, 0.0);
        frame(&mut app, vec![egui::Event::PointerMoved(at), button(at, true)]);
        frame(&mut app, vec![button(at, false)]);
        frame(&mut app, vec![]);
        assert_eq!(rows_of(&app), vec![sub]);
        assert_eq!(app.session.active().unwrap().current_layer(), Some(sub));
    }

    /// A layer of 2,000 groups of 10 paths with every row open: one frame lays out only the rows
    /// in view, and a row highlighted far below them is scrolled to.
    #[test]
    fn only_the_rows_in_view_are_laid_out() {
        use vectorcraft_doc::Appearance;
        use vectorcraft_geom::{Rect, shapes};
        let mut d = Document::new(1000.0, 1000.0);
        let mut groups = vec![];
        let mut needle = None;
        for g in 0..2000 {
            let kids = (0..10)
                .map(|k| {
                    let r = Rect::new(k as f64, g as f64 * 0.1, k as f64 + 1.0, g as f64 * 0.1 + 1.0);
                    let mut n = Node::path(d.alloc_id(), shapes::rectangle(r), Appearance::default());
                    if g == 0 && k == 0 {
                        n.name = Some("Needle".into());
                        needle = Some(n.id);
                    }
                    Arc::new(n)
                })
                .collect();
            groups.push(Arc::new(Node::group(d.alloc_id(), kids)));
        }
        *Arc::make_mut(&mut d.layers[0]).children_mut().unwrap() = groups;
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.add_document(d, None);
        expand(&mut app, &json!({})).unwrap();
        let ctx = egui::Context::default();
        // (shapes drawn, whether the needle's name is among them); a second passes per frame, so
        // scrolling ends.
        let mut time = 0.0;
        let mut frame = |app: &mut VectorcraftApp| {
            time += 1.0;
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(300.0, 800.0));
            let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), time: Some(time), ..Default::default() }, |ui| show(app, ui));
            out.textures_delta.clear();
            let found = out.shapes.iter().any(|c| matches!(&c.shape, egui::Shape::Text(t) if t.galley.text() == "Needle"));
            (out.shapes.len(), found)
        };
        let (shapes, found) = frame(&mut app);
        // 22,001 rows; about 30 fit.
        assert!(shapes < 1000 && !found, "{shapes} shapes");
        let needle = needle.unwrap();
        app.session.execute("layer.highlight", &json!({"ids": [needle.0], "mode": "set"})).unwrap();
        let found = (0..4).any(|_| frame(&mut app).1);
        assert_eq!(app.session.active().unwrap().highlighted_rows(), [needle]);
        assert!(found, "the highlighted row is scrolled into view");
    }

    /// Selecting art on the canvas scrolls its layer (now the active one) into view, but not away
    /// from a row highlighted in the panel.
    #[test]
    fn a_canvas_selection_reveals_its_active_layer() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        let run = |app: &mut VectorcraftApp, id: &str, p: serde_json::Value| app.session.execute(id, &p).unwrap();
        run(&mut app, "file.new", json!({"width": 100, "height": 100}));
        let mut rects = vec![run(&mut app, "shape.rectangle", json!({"x": 0, "y": 0, "width": 5, "height": 5}))["id"].as_u64().unwrap()];
        for _ in 0..59 {
            run(&mut app, "layer.new", json!({}));
            rects.push(run(&mut app, "shape.rectangle", json!({"x": 0, "y": 0, "width": 5, "height": 5}))["id"].as_u64().unwrap());
        }
        run(&mut app, "select.none", json!({}));
        let doc = app.session.active().unwrap().doc.clone();
        let name = |id: u64| doc.node(doc.layer_of(NodeId(id)).unwrap()).unwrap().display_name();
        let (bottom, top) = (name(rects[0]), name(rects[59]));
        let ctx = egui::Context::default();
        let mut time = 0.0;
        // Whether a row with this name is drawn after a few frames.
        let mut shown = |app: &mut VectorcraftApp, s: &str| {
            let mut found = false;
            for _ in 0..4 {
                time += 1.0;
                let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(300.0, 400.0));
                let mut out = ctx.run_ui(egui::RawInput { screen_rect: Some(screen), time: Some(time), ..Default::default() }, |ui| show(app, ui));
                out.textures_delta.clear();
                found = out.shapes.iter().any(|c| matches!(&c.shape, egui::Shape::Text(t) if t.galley.text() == s));
            }
            found
        };
        assert!(shown(&mut app, &top) && !shown(&mut app, &bottom));
        run(&mut app, "select.set", json!({"ids": [rects[0]]}));
        assert_eq!(app.session.active().unwrap().current_layer(), doc.layer_of(NodeId(rects[0])));
        assert!(shown(&mut app, &bottom), "the active layer is scrolled into view");
        // A row highlighted in the panel stays in view when the active layer changes.
        let top_layer = doc.layer_of(NodeId(rects[59])).unwrap();
        run(&mut app, "layer.highlight", json!({"ids": [top_layer.0], "mode": "set"}));
        assert!(shown(&mut app, &top));
        app.session.active_mut().unwrap().active_layer = doc.layer_of(NodeId(rects[0]));
        assert!(shown(&mut app, &top) && !shown(&mut app, &bottom), "the highlighted row keeps the view");
    }
}
