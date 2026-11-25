use std::borrow::Cow;
use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash as _, Hasher as _};
use std::ops::Deref;

use craft::keymap::Mods;
pub(super) use craft::layers::ui::filter::{FilterObject, FilterState};
use craft::layers::ui::filter::{LayerFilter, SmartSource};
use craft::layers::ui::panel::MenuTarget;
use craft::layers::ui::tree::{
    LayerInfo, LayerKind, LayerPart, LayerProperties, LayerRowKey, LayerSelection, LayerTree, LayerView, SubRowGroup, SubRows,
};
use craft::ui::contract::{Entry, PlainData, ReorderMove};
use craft::ui::list::LabelUnderline;
use craft::ui::style::DataColor;
use vectorcraft_doc::Guide;
use vectorcraft_engine::{DocState, OpenRows};

use super::{Arc, Color32, Document, LiveShape, Node, NodeId, NodeKind, PanelOptions, Value, VectorcraftApp};

/// The name painted for a row in `lang`: a generated `<Kind>` name is translated, anything else is
/// user data (an unnamed text object shows its text, which can look like `<Path>`; only an empty
/// one is called `<Text>`).
pub(super) fn painted_name(n: &Node, name: &str, lang: crate::i18n::Lang) -> String {
    let generated = n.name.is_none() && !matches!(&n.kind, NodeKind::Text(t) if t.runs.iter().any(|r| !r.text.is_empty()));
    if generated && let Some(inner) = name.strip_prefix('<').and_then(|s| s.strip_suffix('>')) {
        return format!("<{}>", crate::i18n::tr(lang, inner));
    }
    name.to_string()
}

/// The name the panel shows for `n` in `lang`: its XML ID while Identify Objects By is XML ID,
/// otherwise [`painted_name`].
pub(crate) fn shown_name(app: &VectorcraftApp, n: &Node, lang: crate::i18n::Lang) -> String {
    listed_name(n, by_xml_id(app), lang)
}

/// Whether Identify Objects By is XML ID.
fn by_xml_id(app: &VectorcraftApp) -> bool {
    app.session.prefs.identify_objects_by == "xmlId"
}

/// [`shown_name`] with Identify Objects By read as `xml_id`.
fn listed_name(n: &Node, xml_id: bool, lang: crate::i18n::Lang) -> String {
    let label = n.identify_label(xml_id);
    if xml_id { label } else { painted_name(n, &label, lang) }
}

/// Whether `n` gets rows for what it holds (a compound path's paths are one object).
pub(super) fn opens(n: &Node, opts: &PanelOptions) -> bool {
    match n.kind {
        NodeKind::Compound { .. } => false,
        // Ruler guide nodes are not object rows (a layer's guide rows are counted by its caller).
        _ => n.children().is_some_and(|c| c.iter().any(|c| !c.is_ruler_guide() && (!opts.layers_only || c.is_layer()))),
    }
}

/// A filter value with a control-channel name (`ui.layersFilter`'s parameters).
pub(super) trait FilterName: Copy + PartialEq {
    /// The parameter name.
    fn param(self) -> &'static str;
}

impl FilterName for FilterObject {
    fn param(self) -> &'static str {
        self.name()
    }
}

impl FilterName for FilterState {
    fn param(self) -> &'static str {
        self.name()
    }
}

// PLAN: the sheet drops `names`; document-panels-w8's `set_filter` answers with the ticked names through it.
/// The parameter names of the ones of `all` in `on`, in `all`'s order.
pub(super) fn names<T: FilterName>(all: &[T], on: &[T]) -> Vec<&'static str> {
    all.iter().filter(|t| on.contains(t)).map(|t| t.param()).collect()
}

/// The list `key` of `p` read by the parameter names of `all`, in `all`'s order: `None` when `p`
/// leaves it out or sets it to null, and an error when it is anything other than an array of
/// those names.
pub(super) fn parse_names<T: FilterName>(p: &Value, key: &str, all: &[T]) -> Result<Option<Vec<T>>, String> {
    let given = match p.get(key) {
        None | Some(Value::Null) => return Ok(None),
        Some(v) => {
            v.as_array().and_then(|a| a.iter().map(|name| all.iter().find(|t| Some(t.param()) == name.as_str()).copied()).collect::<Option<Vec<T>>>())
        }
    };
    let Some(given) = given else {
        let names: Vec<&str> = all.iter().map(|t| t.param()).collect();
        return Err(format!("`{key}` must be an array of these names: {}", names.join(", ")));
    };
    Ok(Some(all.iter().copied().filter(|t| given.contains(t)).collect()))
}

/// The isolated node `id`, which heads the rows in isolation mode, with whether it is its parent's
/// clipping path, and the color of the layer holding it and whether the layers and groups around it
/// are visible. None when `id` is not in `doc`.
pub(super) fn isolated_root(doc: &Document, id: NodeId) -> Option<(&Arc<Node>, bool, (Color32, bool))> {
    let path = doc.index_path(id)?;
    let (first, rest) = path.split_first()?;
    let mut cur = doc.layers.get(*first)?;
    let (mut clip_path, mut colour, mut shown) = (false, Color32::PLACEHOLDER, true);
    for &i in rest {
        if let NodeKind::Layer { color, .. } = &cur.kind {
            let [r, g, b] = color.rgb();
            colour = Color32::from_rgb(r, g, b);
        }
        shown &= cur.visible;
        clip_path = i == 0 && cur.clips();
        cur = cur.children()?.get(i)?;
    }
    Some((cur, clip_path, (colour, shown)))
}

/// Whether a row's name is italic: a layer whose Layer Options › Print is off does not print, and
/// the panel shows that in its name, as the reference application does (a template layer's Print is
/// off too).
fn prints_italic(n: &Node) -> bool {
    matches!(n.kind, NodeKind::Layer { printable: false, .. })
}

/// What the row of `n` is: the kinds the Objects filter tells apart. Paths take plain paths, the
/// paths whose corners Live Corners cut and compound paths; Shapes take live rectangles, ellipses,
/// polygons and lines; a path made into a guide is neither.
fn kind_of(n: &Node) -> LayerKind {
    match &n.kind {
        NodeKind::Layer { .. } => LayerKind::Layer,
        NodeKind::Group { .. } => LayerKind::Group,
        NodeKind::Path { guide: true, .. } => LayerKind::Object,
        NodeKind::Path { live, .. } if live.as_deref().is_some_and(|l| !matches!(l, LiveShape::Path { .. })) => LayerKind::Shape,
        NodeKind::Path { .. } | NodeKind::Compound { .. } => LayerKind::Path,
        NodeKind::Text(_) => LayerKind::Text,
        NodeKind::Image(_) => LayerKind::Image,
        _ => LayerKind::Object,
    }
}

/// A row of the Layers panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PlainData)]
pub enum RowId {
    /// A layer, sublayer, group or object.
    Node(#[plain_data(assume = "a node id is an integer")] NodeId),
    /// A ruler guide, by its index in the document's guides ([`Document::ruler_guides`]).
    Guide(usize),
    /// The spine of a blend, below the blend's key objects.
    Spine(#[plain_data(assume = "a node id is an integer")] NodeId),
}

/// The active document as the Layers panel lists it: the tree the panel reads. The app keeps it and
/// lists it again only when [`LayerRows::key`] changes.
#[derive(Clone, Debug, PlainData)]
pub(super) struct LayerRows {
    #[plain_data(assume = "a document is never changed behind its Arc: an edit makes a new one")]
    doc: Arc<Document>,
    opts: PanelOptions,
    /// Identify Objects By is XML ID.
    xml_id: bool,
    #[plain_data(assume = "a language is an index")]
    lang: crate::i18n::Lang,
    /// The rows open.
    #[plain_data(assume = "a set of node ids and a change count")]
    open: OpenRows,
    /// The highlighted rows.
    #[plain_data(assume = "node ids are integers")]
    highlighted: Vec<NodeId>,
    /// The current layer.
    #[plain_data(assume = "a node id is an integer")]
    current: Option<NodeId>,
    /// The selected ruler guides, by index.
    guides: Vec<usize>,
    /// The document's ruler guides ([`Document::ruler_guides`]).
    #[plain_data(assume = "a guide is numbers and ids")]
    ruler: Vec<Guide>,
    /// The index of each ruler guide node among them.
    #[plain_data(assume = "node ids are integers")]
    guide_rows: HashMap<NodeId, usize>,
    /// The selected objects.
    #[plain_data(assume = "node ids are integers")]
    selected: HashSet<NodeId>,
    /// The layers and sublayers whose visible, unlocked art ([`Node::layer_art`]) is all selected
    /// and not empty.
    #[plain_data(assume = "node ids are integers")]
    full: HashSet<NodeId>,
    /// The selected blends, in selection order.
    #[plain_data(assume = "node ids are integers")]
    blends: Vec<NodeId>,
    /// In isolation mode, the isolated node: the top row.
    #[plain_data(assume = "a node id is an integer")]
    root: Option<NodeId>,
}

/// [`LayerRows`] with the editing state of their document, for the panel's host code in one pass.
pub(super) struct DocLayers<'a> {
    st: &'a DocState,
    rows: Cow<'a, LayerRows>,
}

impl<'a> DocLayers<'a> {
    /// The rows of `st`, the document `app` shows, listed now.
    pub(super) fn new(app: &VectorcraftApp, st: &'a DocState) -> Self {
        Self { st, rows: Cow::Owned(LayerRows::new(app, st)) }
    }

    /// The rows of `st` as the app keeps them.
    pub(super) fn of(st: &'a DocState, rows: &'a LayerRows) -> Self {
        Self { st, rows: Cow::Borrowed(rows) }
    }

    /// The document's state.
    pub(super) fn state(&self) -> &'a DocState {
        self.st
    }
}

impl Deref for DocLayers<'_> {
    type Target = LayerRows;

    fn deref(&self) -> &LayerRows {
        &self.rows
    }
}

impl LayerRows {
    /// The rows of `st`, the document `app` shows.
    pub(super) fn new(app: &VectorcraftApp, st: &DocState) -> Self {
        let doc = &st.doc;
        let ruler = doc.ruler_guides();
        let guide_rows = ruler.iter().enumerate().filter_map(|(i, g)| Some((g.id?, i))).collect();
        let blends = st.selection.objects.iter().copied().filter(|id| doc.node(*id).is_some_and(|n| matches!(n.kind, NodeKind::Blend { .. })));
        let selected: HashSet<NodeId> = st.selection.objects.iter().copied().collect();
        Self {
            doc: doc.clone(),
            opts: app.ui.layers_panel.clone(),
            xml_id: by_xml_id(app),
            lang: crate::i18n::current(),
            open: st.layers_open.clone(),
            highlighted: st.highlighted_rows(),
            current: st.current_layer(),
            guides: vectorcraft_engine::cmd::guide_indexes(doc, &st.selection.guides),
            ruler,
            guide_rows,
            full: super::rows::full_layers(doc, &selected),
            selected,
            blends: blends.collect(),
            root: st.isolation.and_then(|i| isolated_root(doc, i)).map(|(n, ..)| n.id),
        }
    }

    /// What the rows of `st` are listed from: the document and its revision (which every edit and
    /// selection change moves), the open rows, isolation mode, the highlighted rows and the current
    /// layer, the Panel Options, Identify Objects By and the language.
    pub(super) fn key(app: &VectorcraftApp, st: &DocState) -> u64 {
        let mut hasher = DefaultHasher::new();
        let opts = &app.ui.layers_panel;
        (
            st.uid,
            st.revision,
            st.layers_open.generation(),
            st.isolation,
            &st.layer_rows,
            st.active_layer,
            opts,
            by_xml_id(app),
            crate::i18n::current(),
        )
            .hash(&mut hasher);
        hasher.finish()
    }

    /// The document.
    pub(super) fn doc(&self) -> &Arc<Document> {
        &self.doc
    }

    /// In isolation mode, the isolated node, which heads the rows.
    pub(super) fn root(&self) -> Option<NodeId> {
        self.root
    }

    /// Show Layers Only.
    pub(super) fn layers_only(&self) -> bool {
        self.opts.layers_only
    }

    /// The selected ruler guides, by index.
    pub(super) fn guides(&self) -> &[usize] {
        &self.guides
    }

    /// Whether object `id` is selected.
    pub(super) fn is_selected(&self, id: NodeId) -> bool {
        self.selected.contains(&id)
    }

    /// Whether `id` is a layer or sublayer whose visible, unlocked art is all selected and not
    /// empty.
    pub(super) fn is_full(&self, id: NodeId) -> bool {
        self.full.contains(&id)
    }

    /// The node ids a row menu's command acts on: every highlighted row when the menu's row is one of
    /// them, else the row's own node.
    pub(super) fn targets(&self, target: &MenuTarget<RowId>) -> Vec<u64> {
        let Some(LayerRowKey::Layer(row)) = &target.row else { return vec![] };
        let Some(id) = self.node_of(row) else { return vec![] };
        if self.highlighted.contains(&id) { self.highlighted.iter().map(|r| r.0).collect() } else { vec![id.0] }
    }

    /// The node row `row` stands for: a ruler guide's node, a spine's blend.
    pub(super) fn node_of(&self, row: &RowId) -> Option<NodeId> {
        match *row {
            RowId::Node(id) | RowId::Spine(id) => Some(id),
            RowId::Guide(i) => self.ruler.get(i).and_then(|g| g.id),
        }
    }

    /// The colour of the layer holding `id`.
    pub(super) fn color(&self, id: NodeId) -> DataColor {
        let [r, g, b] = self.doc.layer_color(id);
        DataColor::new(Color32::from_rgb(r, g, b))
    }

    /// Whether `id` and everything around it is visible.
    fn shown(&self, id: NodeId) -> bool {
        self.doc.ancestry(id).is_some_and(|a| a.iter().all(|i| self.doc.node(*i).is_some_and(|n| n.visible)))
    }

    /// Whether rows `moving` (objects only when `objects`) may land inside `group`: never into one of
    /// them or a row inside it, in isolation mode only inside the isolated art, and only where the
    /// group takes them ([`super::rows::takes`]).
    pub(super) fn lands_in(&self, group: NodeId, moving: &[NodeId], objects: bool) -> bool {
        let Some(g) = self.doc.node(group) else { return false };
        let around = self.doc.ancestry(group).unwrap_or_default();
        !moving.iter().any(|m| around.contains(m))
            && self.root.is_none_or(|r| around.contains(&r))
            && super::rows::takes(g, objects, moving, &self.doc)
    }

    /// The ruler guide rows of `n`, under a layer before its art, the highest `guide.list` index first.
    fn guide_rows_of(&self, n: &Node) -> Vec<RowId> {
        if !n.is_layer() || self.opts.layers_only {
            return vec![];
        }
        let mut found: Vec<usize> =
            n.children().into_iter().flatten().filter(|c| c.is_ruler_guide()).filter_map(|c| self.guide_rows.get(&c.id).copied()).collect();
        found.sort_unstable_by(|a, b| b.cmp(a));
        found.into_iter().map(RowId::Guide).collect()
    }

    /// The rows inside `n`, top down: its ruler guides, its art (only layers with Show Layers Only on;
    /// ruler guide nodes are guide rows, not object rows), then a blend's spine, as Release leaves it.
    fn rows_in(&self, n: &Node) -> Vec<RowId> {
        let mut rows = self.guide_rows_of(n);
        if !opens(n, &self.opts) {
            return rows;
        }
        for c in n.children().into_iter().flatten().rev() {
            if !c.is_ruler_guide() && (!self.opts.layers_only || c.is_layer()) {
                rows.push(RowId::Node(c.id));
            }
        }
        if matches!(n.kind, NodeKind::Blend { .. }) {
            rows.push(RowId::Spine(n.id));
        }
        rows
    }

    fn node_info(&self, n: &Node) -> LayerInfo {
        let name = if self.doc.mask_edit.is_some_and(|m| m.layer == n.id) {
            tl!("<Opacity Mask>").to_string()
        } else {
            listed_name(n, self.xml_id, self.lang)
        };
        let mut info = LayerInfo::new(name, kind_of(n));
        info.visible = n.visible;
        info.dimmed = !self.shown(n.id) || n.is_template();
        info.locks.all = n.locked;
        info.color = Some(self.color(n.id));
        info.view = if n.is_template() {
            LayerView::Template
        } else if matches!(n.kind, NodeKind::Layer { preview: false, .. }) {
            LayerView::Outline
        } else {
            LayerView::Preview
        };
        info.italic = prints_italic(n);
        // A clipping path's name is underlined, a masked object's with a dashed line.
        let parent = self.doc.parent_of(n.id).and_then(|p| self.doc.node(p));
        let clip_path = parent.is_some_and(|p| p.clips() && p.children().and_then(|c| c.first()).is_some_and(|c| c.id == n.id));
        info.underline = if clip_path {
            LabelUnderline::Solid
        } else if n.extras().mask.is_some() {
            LabelUnderline::Dashed
        } else {
            LabelUnderline::None
        };
        info.thumbnail = self.opts.thumb(n);
        // An effect, shown or hidden, on the object itself or on one of its fills or strokes.
        info.has_effects = !n.appearance.effects.is_empty() || n.appearance.items.iter().any(|i| !i.effects().is_empty());
        let linked = match &n.kind {
            NodeKind::PlacedDocument(_) => true,
            NodeKind::Image(im) => im.link.is_some(),
            _ => false,
        };
        info.smart = linked.then_some(SmartSource::Linked);
        info
    }

    fn guide_info(&self, i: usize) -> LayerInfo {
        let g = self.ruler.get(i);
        let vertical = g.is_some_and(|g| g.vertical);
        let name = format!("<{}>", if vertical { tl!("Vertical Guide") } else { tl!("Horizontal Guide") });
        let mut info = LayerInfo::new(name, LayerKind::Guide);
        let node = g.and_then(|g| g.id).and_then(|id| self.doc.node(id));
        info.visible = node.is_none_or(|n| n.visible);
        info.locks.all = node.is_some_and(|n| n.locked);
        if let Some(layer) = g.and_then(|g| g.layer) {
            info.dimmed = !self.shown(layer) || !info.visible;
            info.color = Some(self.color(layer));
        }
        // PLAN: dp.L12's horizontal and vertical guide marks need app icons the icon set lacks; the row keeps the kind's ruler glyph.
        info.renamable = false;
        info.thumbnail = false;
        info
    }

    fn spine_info(&self, blend: NodeId) -> LayerInfo {
        let mut info = LayerInfo::new(format!("<{}>", tl!("Path")), LayerKind::Spine);
        info.dimmed = !self.shown(blend);
        info.color = Some(self.color(blend));
        info.renamable = false;
        info.thumbnail = false;
        info
    }
}

impl LayerTree for LayerRows {
    type Id = RowId;

    /// The top rows are the layers (only the opacity mask's while one is edited), or in isolation
    /// mode the isolated layer, sublayer, group or object.
    fn children(&self, parent: Option<&RowId>) -> Vec<RowId> {
        match parent {
            None => match self.root {
                Some(root) => vec![RowId::Node(root)],
                None => {
                    let mask_layer = self.doc.mask_edit.map(|m| m.layer);
                    self.doc.layers.iter().rev().filter(|l| mask_layer.is_none_or(|m| m == l.id)).map(|l| RowId::Node(l.id)).collect()
                }
            },
            Some(RowId::Node(id)) => self.doc.node(*id).map(|n| self.rows_in(n)).unwrap_or_default(),
            Some(RowId::Guide(_) | RowId::Spine(_)) => vec![],
        }
    }

    fn parent(&self, id: &RowId) -> Option<RowId> {
        match *id {
            RowId::Node(id) if Some(id) == self.root => None,
            RowId::Node(id) => self.doc.parent_of(id).map(RowId::Node),
            RowId::Guide(i) => self.ruler.get(i).and_then(|g| g.layer).map(RowId::Node),
            RowId::Spine(blend) => Some(RowId::Node(blend)),
        }
    }

    fn is_open(&self, id: &RowId) -> bool {
        matches!(id, RowId::Node(id) if self.open.contains(*id))
    }

    fn effects_open(&self, _: &RowId) -> bool {
        false
    }

    fn info(&self, id: &RowId) -> LayerInfo {
        match *id {
            RowId::Node(id) => match self.doc.node(id) {
                Some(n) => self.node_info(n),
                None => LayerInfo::new(String::new(), LayerKind::Object),
            },
            RowId::Guide(i) => self.guide_info(i),
            RowId::Spine(blend) => self.spine_info(blend),
        }
    }

    /// The filter's rule, and the app's own: a path made into a guide is a guide to it; the ruler
    /// guide rows stay only with no search and no row state ticked, while Guides is ticked or no
    /// object kind is; a spine is no object of its own and leaves while the filter is on.
    fn listed(&self, id: &RowId, filter: &LayerFilter) -> bool {
        match *id {
            RowId::Node(node) => match self.doc.node(node) {
                Some(n) if matches!(n.kind, NodeKind::Path { guide: true, .. }) => {
                    let mut info = self.node_info(n);
                    info.kind = LayerKind::Guide;
                    filter.shows(&info)
                }
                _ => filter.lists(self, id),
            },
            RowId::Guide(_) => {
                filter.name.trim().is_empty()
                    && filter.states.is_empty()
                    && (filter.objects.is_empty() || filter.objects.contains(&FilterObject::Guides))
            }
            RowId::Spine(_) => false,
        }
    }

    /// The highlighted rows, else the current layer's; the selected ruler guides; the spines of the
    /// selected blends.
    fn selection(&self) -> LayerSelection<RowId> {
        let mut rows: Vec<RowId> = self.highlighted.iter().copied().map(RowId::Node).collect();
        let current = self.current.map(RowId::Node);
        if rows.is_empty() {
            rows.extend(current);
        }
        rows.extend(self.guides.iter().map(|i| RowId::Guide(*i)));
        rows.extend(self.blends.iter().map(|id| RowId::Spine(*id)));
        LayerSelection::new(rows, current, LayerPart::Pixels)
    }

    fn sub_rows(&self, _: &RowId, _: SubRowGroup) -> Option<SubRows> {
        None
    }

    fn properties(&self) -> Option<LayerProperties> {
        None
    }

    fn blend_modes(&self, _: bool, _: &str) -> Vec<Entry<String>> {
        Vec::new()
    }

    /// Rows land in a layer or group that takes them ([`LayerRows::lands_in`]): ruler guides only on
    /// a layer (into a group, on the layer holding it), never at the top level; nothing outside the
    /// isolated art; a spine never moves.
    fn accepts_drop(&self, mv: &ReorderMove<LayerRowKey<RowId>>, _: Mods) -> bool {
        let rows: Vec<RowId> = mv
            .keys
            .iter()
            .filter_map(|key| match key {
                LayerRowKey::Layer(row) => Some(*row),
                _ => None,
            })
            .collect();
        let guides = !rows.is_empty() && rows.iter().all(|r| matches!(r, RowId::Guide(_)));
        let moving: Vec<NodeId> = rows
            .iter()
            .filter_map(|r| match r {
                RowId::Node(id) => Some(*id),
                _ => None,
            })
            .collect();
        if rows.len() != mv.keys.len() || (!guides && moving.len() != rows.len()) {
            return false;
        }
        let objects = !guides && !moving.iter().any(|id| self.doc.node(*id).is_some_and(Node::is_layer));
        match &mv.parent {
            None => self.root.is_none() && !guides,
            Some(LayerRowKey::Layer(RowId::Node(group))) => {
                let into = match self.doc.node(*group) {
                    Some(n) if guides && !n.is_layer() => self.doc.layer_containing(*group),
                    _ => Some(*group),
                };
                into.is_some_and(|into| self.lands_in(into, &moving, objects))
            }
            Some(_) => false,
        }
    }
}
