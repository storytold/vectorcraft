use super::rows::key;
use super::*;

/// Where a dragged row goes relative to the row it is dropped on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Place {
    Above,
    Below,
    Inside,
}

impl Place {
    pub(super) fn id(self) -> &'static str {
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
    let xml_id = app.session.prefs.identify_objects_by == "xmlId";
    let label = n.identify_label(xml_id);
    if xml_id { label } else { painted_name(n, &label, lang) }
}

/// Whether `n` gets rows for what it holds (a compound path's paths are one object).
pub(super) fn opens(n: &Node, opts: &PanelOptions) -> bool {
    match n.kind {
        NodeKind::Compound { .. } => false,
        _ => n.children().is_some_and(|c| c.iter().any(|c| !opts.layers_only || c.is_layer())),
    }
}

/// One row of the panel.
#[derive(Clone)]
pub(super) struct Row {
    pub(super) node: Arc<Node>,
    /// Nesting level (0: a top-level layer).
    pub(super) depth: usize,
    /// A clipping path: its group or layer clips.
    pub(super) clip_path: bool,
    /// It holds rows ([`opens`]), and shows them.
    pub(super) opens: bool,
    pub(super) open: bool,
    /// The colour of its layer (its own, for a layer).
    pub(super) colour: Color32,
    /// Drawn dimmed: hidden itself or in a hidden layer or group, or a template layer.
    pub(super) dim: bool,
    /// A ruler guide on the layer `node` (its index in the document's guides): the guide's row.
    pub(super) guide: Option<usize>,
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

    pub(super) fn label(self) -> &'static str {
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

    pub(super) fn label(self) -> &'static str {
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
pub(super) fn names<T: Copy + PartialEq>(all: &[(T, &'static str)], on: &[T]) -> Vec<&'static str> {
    all.iter().filter(|(t, _)| on.contains(t)).map(|(_, name)| *name).collect()
}

/// The parameter names of `on` with `t` added, or taken out when it is in `on`.
pub(super) fn toggled<T: Copy + PartialEq>(all: &[(T, &'static str)], on: &[T], t: T) -> Vec<&'static str> {
    all.iter().filter(|(x, _)| if *x == t { !on.contains(x) } else { on.contains(x) }).map(|(_, name)| *name).collect()
}

/// The list `key` of `p` read by the parameter names of `all`, in `all`'s order: `None` when `p`
/// leaves it out or sets it to null, and an error when it is anything other than an array of
/// those names.
pub(super) fn parse_names<T: Copy + PartialEq>(p: &Value, key: &str, all: &[(T, &'static str)]) -> Result<Option<Vec<T>>, String> {
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
pub(super) fn list_rows(doc: &Document, open: &OpenRows, opts: &PanelOptions, query: &str, filter: &Filter) -> Vec<Row> {
    let mask_layer = doc.mask_edit.map(|m| m.layer);
    let mut out = vec![];
    for l in doc.layers.iter().rev().filter(|l| mask_layer.is_none_or(|m| m == l.id)) {
        push_rows(l, 0, false, (Color32::PLACEHOLDER, true), (open, opts, query, filter, doc), &mut out);
    }
    out
}

/// Add the rows of `n` and those open inside it to `out`; `(colour, shown)` are those of what holds
/// it (the layer's colour, and whether it is visible).
pub(super) fn push_rows(
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
pub(super) fn rows_of_doc(ui: &Ui, st: &vectorcraft_engine::DocState, opts: &PanelOptions, query: &str, filter: &Filter) -> Arc<Vec<Row>> {
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
