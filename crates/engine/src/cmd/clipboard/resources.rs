//! The document resources copied objects use, and adding them to the document they're pasted into.
//!
//! Objects refer to resources by name (image blobs, symbols, patterns, swatches, character and
//! paragraph styles, brushes) and to graphic styles by id. Copy follows those references, also
//! through the symbols, patterns, styles and brush art it finds, and keeps what they lead to.
//!
//! Pasting matches each resource by name: an identical one in the document is used, a missing one
//! is added, and a different one of the same name is added under a free name, which the pasted
//! objects follow. Global and spot swatches are the exception: for a name the document gives
//! another colour the caller picks Merge (the objects take the document's swatch) or Add (the
//! pasted swatch comes in renamed), the Swatch Conflict dialog's question. Colours linked to a
//! swatch then show it as the document does: its colour at their tint, and a Lab spot colour as
//! the document's Spot Colors option says. Pasting back into the document the objects came from
//! uses its resources as they are now.

use std::cell::OnceCell;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::Arc;

use serde_json::{Map, Value, json};
use vectorcraft_brush::Brush;
use vectorcraft_color::{Color, Paint, Swatch};
use vectorcraft_doc::text::{KINSOKU_NAME_MAX, KINSOKU_SETS_MAX};
use vectorcraft_doc::{
    Appearance, AppearanceItem, Document, Effect, GraphicStyle, Kinsoku, KinsokuSet, Node, NodeKind, PatternDef, Symbol, SymbolExportType,
    SymbolNineSlice, SymbolRegistration, TextStyleDef,
};

use super::super::brushsym::{set_natural_size, stored_natural_size};
use super::super::typecmd::refresh_bounds;

use super::super::patterncmds::add_pattern;
use super::super::{EngineError, Result, bad, unique_name};
use super::Clipboard;

/// A resource kind objects refer to (graphic styles by id, the others by name).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Res {
    Image,
    Symbol,
    Pattern,
    Swatch,
    CharStyle,
    ParaStyle,
    Brush,
    GraphicStyle,
}

impl Res {
    fn id(self) -> &'static str {
        match self {
            Res::Image => "image",
            Res::Symbol => "symbol",
            Res::Pattern => "pattern",
            Res::Swatch => "swatch",
            Res::CharStyle => "charStyle",
            Res::ParaStyle => "paraStyle",
            Res::Brush => "brush",
            Res::GraphicStyle => "graphicStyle",
        }
    }
}

/// A reference from art to a resource.
enum Ref<'a> {
    Name(Res, &'a str),
    /// A colour linked to a swatch, at a tint.
    Swatch(&'a str, f32),
    /// A link to the graphic style with this id.
    Style(u32),
}

type Visit<'v> = &'v mut dyn FnMut(Ref<'_>);

fn paint_refs(p: &Paint, f: Visit) {
    match p {
        Paint::Solid { swatch: Some(s), tint, .. } => f(Ref::Swatch(s, *tint)),
        Paint::Gradient(g) => {
            // The gradient swatch it was applied from, and its stops' links.
            if let Some(s) = &g.swatch {
                f(Ref::Name(Res::Swatch, s));
            }
            g.gradient.stops.iter().filter_map(|st| Some((st.swatch.as_deref()?, st.tint))).for_each(|(s, t)| f(Ref::Swatch(s, t)));
        }
        Paint::Pattern { pattern, .. } => f(Ref::Name(Res::Pattern, pattern)),
        _ => {}
    }
}

fn appearance_refs(a: &Appearance, f: Visit) {
    // The graphics mapped onto the faces of its 3D effects name symbols.
    for s in vectorcraft_render::effects::appearance_graphic_symbols(a) {
        f(Ref::Name(Res::Symbol, &s));
    }
    for it in &a.items {
        paint_refs(it.paint(), f);
        if let AppearanceItem::Stroke(l) = it
            && let Some(b) = &l.brush
        {
            f(Ref::Name(Res::Brush, b));
        }
    }
}

/// The paints a character or paragraph style can set (by their serialized names).
const STYLE_PAINTS: [&str; 2] = ["fill", "stroke"];

fn style_paint(attrs: &Map<String, Value>, key: &str) -> Option<Paint> {
    serde_json::from_value(attrs.get(key)?.clone()).ok()
}

fn attrs_refs(attrs: &Map<String, Value>, f: Visit) {
    for p in STYLE_PAINTS.iter().filter_map(|k| style_paint(attrs, k)) {
        paint_refs(&p, f);
    }
}

/// Every reference in the subtree of `n` (opacity-mask art included).
fn node_refs(n: &Node, f: Visit) {
    n.walk(&mut |m| {
        appearance_refs(&m.appearance, f);
        if let Some(id) = m.extras().graphic_style {
            f(Ref::Style(id));
        }
        if let Some(mask) = &m.extras().mask {
            node_refs(&mask.art, f);
        }
        // A color-masked image's masked pixels go with it.
        if let Some(cm) = Document::color_mask_of(m) {
            f(Ref::Name(Res::Image, &cm.key));
        }
        // A graph keeps paints and style names for its series and labels, which a redraw gives them.
        if let Some(g) = m.extras().graph.as_deref() {
            vectorcraft_doc::swatches::graph_paints(m).for_each(|p| paint_refs(p, f));
            let l = &g.label_styles;
            for s in [&l.values, &l.categories, &l.legend].into_iter().flatten().filter_map(|st| st.style_name.as_ref()) {
                f(Ref::Name(Res::CharStyle, s));
            }
            for s in [&l.values_para, &l.categories_para, &l.legend_para].into_iter().flatten().filter_map(|pa| pa.style_name.as_ref()) {
                f(Ref::Name(Res::ParaStyle, s));
            }
        }
        match &m.kind {
            NodeKind::Text(t) => {
                for r in &t.runs {
                    paint_refs(&r.style.fill, f);
                    paint_refs(&r.style.stroke, f);
                    if let Some(s) = &r.style.style_name {
                        f(Ref::Name(Res::CharStyle, s));
                    }
                }
                for s in t.para_styles().filter_map(|pa| pa.style_name.as_ref()) {
                    f(Ref::Name(Res::ParaStyle, s));
                }
            }
            NodeKind::Image(im) => f(Ref::Name(Res::Image, &im.key)),
            NodeKind::PlacedDocument(pl) => f(Ref::Name(Res::Image, &pl.key)),
            NodeKind::SymbolInstance { symbol, .. } => f(Ref::Name(Res::Symbol, symbol)),
            _ => {}
        }
    });
}

/// The `(name, xml)` of `e` when it is an SVG filter effect.
fn filter_of(e: &Effect) -> Option<(String, String)> {
    let text = |k: &str| e.params.get(k).and_then(Value::as_str).map(str::to_string);
    if e.id != Appearance::SVG_FILTER {
        return None;
    }
    Some((text("name")?, text("xml")?))
}

/// The SVG filter effects of appearance `a` (its own and its fills' and strokes').
fn appearance_filters(a: &Appearance, out: &mut BTreeSet<(String, String)>) {
    let items = a.items.iter().flat_map(|i| i.effects().iter());
    out.extend(a.effects.iter().chain(items).filter_map(filter_of));
}

/// The SVG filter effects in the subtree of `n` (opacity-mask art included).
fn node_filters(n: &Node, out: &mut BTreeSet<(String, String)>) {
    n.walk(&mut |m| {
        appearance_filters(&m.appearance, out);
        if let Some(mask) = &m.extras().mask {
            node_filters(&mask.art, out);
        }
    });
}

/// The names the SVG filter effects of appearance `a` give (its own and its fills' and strokes').
fn appearance_filter_names(a: &Appearance, out: &mut BTreeSet<String>) {
    let items = a.items.iter().flat_map(|i| i.effects().iter());
    let named = a.effects.iter().chain(items).filter(|e| e.id == Appearance::SVG_FILTER);
    out.extend(named.filter_map(|e| e.params.get("name").and_then(Value::as_str)).map(str::to_string));
}

/// The names the SVG filter effects in the subtree of `n` give (opacity-mask art included).
fn node_filter_names(n: &Node, out: &mut BTreeSet<String>) {
    n.walk(&mut |m| {
        appearance_filter_names(&m.appearance, out);
        if let Some(mask) = &m.extras().mask {
            node_filter_names(&mask.art, out);
        }
    });
}

/// A global or spot colour of its own (not a tint of another swatch): what colours link to.
fn is_base(w: &Swatch) -> bool {
    (w.global || w.spot) && matches!(w.paint, Paint::Solid { swatch: None, .. })
}

/// The definitions references lead to: a document's, or the clipboard's own.
struct Defs<'a> {
    swatches: Vec<&'a Swatch>,
    symbols: &'a [Symbol],
    patterns: &'a [PatternDef],
    styles: &'a [GraphicStyle],
    char_styles: &'a [TextStyleDef],
    para_styles: &'a [TextStyleDef],
    brushes: Brushes<'a>,
    /// Where each name is defined, built the first time a kind is followed.
    index: DefIndex,
}

/// The positions of a kind's definitions by name.
type NameIndex = HashMap<String, Vec<usize>>;

/// [`Defs`]' definitions by name (graphic styles by id), each kind indexed the first time a
/// reference to it is followed: finding a name's definitions by scanning them all made a
/// document or library of N definitions that use each other cost N² comparisons.
#[derive(Default)]
struct DefIndex {
    swatches: OnceCell<NameIndex>,
    symbols: OnceCell<NameIndex>,
    patterns: OnceCell<NameIndex>,
    styles: OnceCell<HashMap<u32, Vec<usize>>>,
    char_styles: OnceCell<NameIndex>,
    para_styles: OnceCell<NameIndex>,
    brushes: OnceCell<NameIndex>,
}

/// The items of `list` named `name`, in order, `index` holding their positions (built from
/// `list` the first time).
fn named<'l, T>(index: &OnceCell<NameIndex>, list: &'l [T], name_of: impl Fn(&T) -> &str, name: &str) -> impl Iterator<Item = &'l T> {
    let index = index.get_or_init(|| {
        let mut by: NameIndex = HashMap::new();
        for (i, item) in list.iter().enumerate() {
            by.entry(name_of(item).to_string()).or_default().push(i);
        }
        by
    });
    let at: &[usize] = index.get(name).map_or(&[], Vec::as_slice);
    at.iter().filter_map(move |&i| list.get(i))
}

/// The brushes references lead to: a document's library, read the first time a brush is used
/// (`vectorcraft_brush::library` parses it), or the clipboard's own.
enum Brushes<'a> {
    Lazy(&'a Document, &'a OnceCell<Vec<Brush>>),
    Ready(&'a [Brush]),
}

impl<'a> Brushes<'a> {
    fn get(&self) -> &'a [Brush] {
        match *self {
            Brushes::Lazy(d, cell) => cell.get_or_init(|| vectorcraft_brush::library(d)),
            Brushes::Ready(list) => list,
        }
    }
}

impl<'a> Defs<'a> {
    /// `d`'s definitions; `brushes` holds its brush library once a brush is used.
    fn of(d: &'a Document, brushes: &'a OnceCell<Vec<Brush>>) -> Self {
        Self {
            swatches: d.swatches_iter().collect(),
            symbols: &d.symbols,
            patterns: &d.patterns,
            styles: &d.graphic_styles,
            char_styles: &d.char_styles,
            para_styles: &d.para_styles,
            brushes: Brushes::Lazy(d, brushes),
            index: DefIndex::default(),
        }
    }
}

/// What references lead out of.
enum Source<'a> {
    Node(&'a Node),
    Appearance(&'a Appearance),
    Attrs(&'a Map<String, Value>),
    Paint(&'a Paint),
}

/// The resources a set of objects uses, through the definitions they lead to.
#[derive(Default)]
pub(super) struct Used {
    names: BTreeMap<Res, BTreeSet<String>>,
    /// The tints (below 100 %) of the swatches used: (swatch, tint).
    tints: Vec<(String, f32)>,
    styles: BTreeSet<u32>,
}

impl Used {
    fn of<'a>(defs: &Defs<'a>, nodes: impl IntoIterator<Item = &'a Node>) -> Self {
        let mut used = Used::default();
        used.follow(defs, nodes.into_iter().map(Source::Node).collect());
        used
    }

    /// Every character style of `defs` (with `chars`) and every paragraph style (with `paras`), and
    /// the resources they use, styles of the other kind included.
    fn of_text_styles(defs: &Defs<'_>, chars: bool, paras: bool) -> Self {
        let mut used = Used::default();
        let mut todo = vec![];
        for s in defs.char_styles.iter().filter(|_| chars) {
            used.note(defs, Ref::Name(Res::CharStyle, &s.name), &mut todo);
        }
        for s in defs.para_styles.iter().filter(|_| paras) {
            used.note(defs, Ref::Name(Res::ParaStyle, &s.name), &mut todo);
        }
        used.follow(defs, todo);
        used
    }

    /// Note the references of `todo`'s sources and of the definitions they lead to.
    fn follow<'a>(&mut self, defs: &Defs<'a>, mut todo: Vec<Source<'a>>) {
        while let Some(src) = todo.pop() {
            let mut found = |r: Ref<'_>| self.note(defs, r, &mut todo);
            match src {
                Source::Node(n) => node_refs(n, &mut found),
                Source::Appearance(a) => appearance_refs(a, &mut found),
                Source::Attrs(a) => attrs_refs(a, &mut found),
                Source::Paint(p) => paint_refs(p, &mut found),
            }
        }
    }

    /// Note `r`; a definition seen for the first time joins `todo`.
    fn note<'a>(&mut self, defs: &Defs<'a>, r: Ref<'_>, todo: &mut Vec<Source<'a>>) {
        let (res, name) = match r {
            Ref::Style(id) => {
                if self.styles.insert(id) && id != 0 {
                    let index = defs.index.styles.get_or_init(|| {
                        let mut by: HashMap<u32, Vec<usize>> = HashMap::new();
                        for (i, g) in defs.styles.iter().enumerate() {
                            by.entry(g.id).or_default().push(i);
                        }
                        by
                    });
                    let at = index.get(&id).map_or(&[][..], Vec::as_slice);
                    todo.extend(at.iter().filter_map(|&i| defs.styles.get(i)).map(|g| Source::Appearance(&g.appearance)));
                }
                return;
            }
            Ref::Swatch(name, tint) => {
                if tint < 1.0 && !self.has_tint(name, tint) {
                    self.tints.push((name.to_string(), tint));
                }
                (Res::Swatch, name)
            }
            Ref::Name(res, name) => (res, name),
        };
        let set = self.names.entry(res).or_default();
        if set.contains(name) {
            return;
        }
        set.insert(name.to_string());
        let ix = &defs.index;
        match res {
            // A tint swatch's base, a gradient swatch's linked stops.
            Res::Swatch => todo.extend(named(&ix.swatches, &defs.swatches, |w| &w.name, name).map(|w| Source::Paint(&w.paint))),
            Res::Symbol => todo.extend(named(&ix.symbols, defs.symbols, |s| &s.name, name).map(|s| Source::Node(&s.art))),
            Res::Pattern => todo.extend(named(&ix.patterns, defs.patterns, |p| &p.name, name).flat_map(|p| p.art.iter().map(|a| Source::Node(a)))),
            Res::CharStyle => todo.extend(named(&ix.char_styles, defs.char_styles, |s| &s.name, name).map(|s| Source::Attrs(&s.attrs))),
            Res::ParaStyle => todo.extend(named(&ix.para_styles, defs.para_styles, |s| &s.name, name).map(|s| Source::Attrs(&s.attrs))),
            // A scatter or art brush's art, a pattern brush's tiles.
            Res::Brush => todo.extend(named(&ix.brushes, defs.brushes.get(), |b| &b.name, name).flat_map(|b| b.art().into_iter().map(Source::Node))),
            _ => {}
        }
    }

    fn has(&self, r: Res, name: &str) -> bool {
        self.names.get(&r).is_some_and(|s| s.contains(name))
    }
    fn has_tint(&self, name: &str, tint: f32) -> bool {
        self.tints.iter().any(|(n, t)| n == name && *t == tint)
    }
    /// Does the clipboard carry swatch `w`: a used global or spot colour or gradient swatch, or a
    /// tint swatch of a tint used? (Never the built-in Registration swatch.)
    fn carries(&self, w: &Swatch) -> bool {
        let named = is_base(w) || matches!(w.paint, Paint::Gradient(_));
        !w.is_reserved() && (named && self.has(Res::Swatch, &w.name) || w.tint_of().is_some_and(|(b, t)| self.has_tint(b, t)))
    }
}

/// What a preview of some art shows besides the art itself ([`art_resources`]).
pub struct ArtResources {
    /// Changes when any resource the art uses changes, and only then.
    pub key: u64,
    /// The art and image bytes counted in `key` by their address, held weakly: their allocations
    /// stay theirs (no other art can take an address) for as long as the key is kept.
    _kept: (Vec<std::sync::Weak<Node>>, Vec<std::sync::Weak<Vec<u8>>>),
}

/// The resources `art` uses in `doc`, through the definitions they lead to as Copy follows them
/// ([`Used`]): images, symbols, patterns (tile type, overlap and spacing too), swatches (linked
/// colours, tints, gradient swatches; colour groups' included), character and paragraph styles,
/// brushes and graphic styles, and the document's Spot Colors option. Art and image bytes count
/// by identity, the other definitions by their saved form: working the key out walks `art` and
/// the definitions it leads to, and never reads an image's bytes.
pub fn art_resources<'a>(doc: &'a Document, art: impl IntoIterator<Item = &'a Node>) -> ArtResources {
    use std::hash::{Hash, Hasher};
    let library = OnceCell::new();
    let defs = Defs::of(doc, &library);
    // The art's references shortened to the brush library's life.
    let mut nodes: Vec<&Node> = vec![];
    for n in art {
        nodes.push(n);
    }
    let used = Used::of(&defs, nodes);
    let mut h = std::collections::hash_map::DefaultHasher::new();
    let mut kept = (vec![], vec![]);
    fn saved(h: &mut impl Hasher, v: &impl serde::Serialize) {
        serde_json::to_vec(v).unwrap_or_default().hash(h);
    }
    fn node(h: &mut impl Hasher, kept: &mut Vec<std::sync::Weak<Node>>, a: &Arc<Node>) {
        (Arc::as_ptr(a) as usize).hash(h);
        kept.push(Arc::downgrade(a));
    }
    for (res, names) in &used.names {
        for name in names {
            (*res as u8, name).hash(&mut h);
            match res {
                Res::Image => {
                    if let Some(b) = doc.images.get(name) {
                        b.mime.hash(&mut h);
                        for bytes in std::iter::once(&b.bytes).chain(&b.proxy) {
                            (Arc::as_ptr(bytes) as usize).hash(&mut h);
                            kept.1.push(Arc::downgrade(bytes));
                        }
                    }
                }
                Res::Symbol => doc.symbols.iter().filter(|s| s.name == *name).for_each(|s| node(&mut h, &mut kept.0, &s.art)),
                Res::Pattern => {
                    for p in doc.patterns.iter().filter(|p| p.name == *name) {
                        saved(&mut h, &PatternDef { art: vec![], ..p.clone() });
                        p.art.iter().for_each(|a| node(&mut h, &mut kept.0, a));
                    }
                }
                Res::CharStyle => doc.char_styles.iter().filter(|s| s.name == *name).for_each(|s| saved(&mut h, s)),
                Res::ParaStyle => doc.para_styles.iter().filter(|s| s.name == *name).for_each(|s| saved(&mut h, s)),
                Res::Brush => defs.brushes.get().iter().filter(|b| b.name == *name).for_each(|b| saved(&mut h, b)),
                // Swatches below (tints count too); graphic styles go by id.
                Res::Swatch | Res::GraphicStyle => {}
            }
        }
    }
    for w in doc.swatches_iter().filter(|w| used.carries(w) || used.has(Res::Swatch, &w.name)) {
        saved(&mut h, w);
    }
    for g in doc.graphic_styles.iter().filter(|g| g.id != 0 && used.styles.contains(&g.id)) {
        saved(&mut h, g);
    }
    if used.names.contains_key(&Res::Swatch) {
        doc.spot_use_lab.hash(&mut h);
    }
    ArtResources { key: h.finish(), _kept: kept }
}

/// A resource list entry known by its name.
trait Named: Clone {
    fn name(&self) -> &str;
    fn set_name(&mut self, name: String);
}

macro_rules! named {
    ($($t:ty),+) => {$(
        impl Named for $t {
            fn name(&self) -> &str {
                &self.name
            }
            fn set_name(&mut self, name: String) {
                self.name = name;
            }
        }
    )+};
}
named!(Symbol, TextStyleDef, Brush);

/// Where a pasted resource ended up in the document.
enum Placed {
    /// The document's own of that name is used.
    Existing,
    /// The document's own numbered copy ("Name 2", from an earlier paste) is used.
    Reused(String),
    Added,
    /// Added under a free name.
    Renamed(String),
}

impl Placed {
    /// Added as `name`, which `from` was free or not.
    fn added(from: &str, name: String) -> Self {
        if name == from { Placed::Added } else { Placed::Renamed(name) }
    }
    /// The name of the definition the paste added (`None` when the document's own is used).
    fn new_name(&self, from: &str) -> Option<String> {
        match self {
            Placed::Existing | Placed::Reused(_) => None,
            Placed::Added => Some(from.to_string()),
            Placed::Renamed(n) => Some(n.clone()),
        }
    }
}

/// Is `name` `base` or a numbered copy of it ("base 2")?
fn copy_of(base: &str, name: &str) -> bool {
    name == base || name.strip_prefix(base).and_then(|r| r.strip_prefix(' ')).is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

/// The document's own resource for pasted resource `name`, among the document's `names`: the
/// one of that name when `same` holds for it (or, pasting back into the source document, at all),
/// else a numbered copy of it (added by an earlier paste) that is the same. `None`: add it.
fn existing<'a>(name: &str, names: impl IntoIterator<Item = &'a str>, same_doc: bool, same: impl Fn(&str) -> bool) -> Option<Placed> {
    let mut found: Vec<&str> = names.into_iter().filter(|n| copy_of(name, n)).collect();
    // The name itself first.
    found.sort_by_key(|n| *n != name);
    if same_doc && found.first() == Some(&name) {
        return Some(Placed::Existing);
    }
    found.into_iter().find(|n| same(n)).map(|n| if n == name { Placed::Existing } else { Placed::Reused(n.to_string()) })
}

/// Find `item` in `list` ([`existing`], `same` comparing it under the entry's name) or add it
/// under a free name.
fn place<T: Named>(list: &mut Vec<T>, item: &T, same_doc: bool, same: impl Fn(&T, &T) -> bool) -> Placed {
    let same_as = |n: &str| {
        let mut c = item.clone();
        c.set_name(n.to_string());
        list.iter().find(|x| x.name() == n).is_some_and(|x| same(x, &c))
    };
    if let Some(p) = existing(item.name(), list.iter().map(Named::name), same_doc, same_as) {
        return p;
    }
    let name = unique_name(item.name(), |n| list.iter().any(|x| x.name() == n));
    let mut c = item.clone();
    c.set_name(name.clone());
    list.push(c);
    Placed::added(item.name(), name)
}

/// Merge or Add, for a swatch whose name the document gives another colour.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum SwatchChoice {
    #[default]
    Merge,
    Add,
}

/// The `swatchConflict` parameter: one choice for every conflict, or one per swatch name (the
/// others merge).
#[derive(Clone, Debug, Default)]
pub(crate) struct SwatchChoices {
    all: SwatchChoice,
    by_name: BTreeMap<String, SwatchChoice>,
}

impl SwatchChoices {
    pub(crate) fn parse(cmd: &str, p: &Value) -> Result<Self> {
        let one = |v: &Value| match v.as_str() {
            Some("merge") => Ok(SwatchChoice::Merge),
            Some("add") => Ok(SwatchChoice::Add),
            _ => Err(bad(cmd, format!("swatchConflict must be \"merge\" or \"add\", not {v}"))),
        };
        match p.get("swatchConflict") {
            None | Some(Value::Null) => Ok(Self::default()),
            Some(Value::Object(m)) => {
                Ok(Self { all: SwatchChoice::Merge, by_name: m.iter().map(|(k, v)| Ok((k.clone(), one(v)?))).collect::<Result<_>>()? })
            }
            Some(v) => Ok(Self { all: one(v)?, by_name: BTreeMap::new() }),
        }
    }
    fn of(&self, name: &str) -> SwatchChoice {
        self.by_name.get(name).copied().unwrap_or(self.all)
    }
}

/// How pasted art is relinked to the document's resources.
#[derive(Clone, Debug, Default)]
struct Relink {
    renames: BTreeMap<Res, BTreeMap<String, String>>,
    /// An SVG filter effect's `(name, xml)` on the clipboard → its `(name, xml)` in the document
    /// ([`Clipboard::filter_links`]).
    filters: BTreeMap<(String, String), (String, String)>,
    /// Graphic style id on the clipboard → its id in the document. Links to other ids are dropped.
    styles: BTreeMap<u32, u32>,
    /// Swatch (its name in the document) → the colour linked colours show at 100 %.
    colors: BTreeMap<String, Color>,
    /// A custom Kinsoku set the pasted paragraphs and paragraph styles hold → the document's set
    /// they take, or none (Hard) when the document has no room for it ([`Clipboard::import_kinsoku_sets`]).
    kinsoku: HashMap<KinsokuSet, Option<Arc<KinsokuSet>>>,
}

impl Relink {
    fn name(&self, r: Res, s: &mut String) {
        if let Some(n) = self.renames.get(&r).and_then(|m| m.get(s.as_str())) {
            *s = n.clone();
        }
    }
    /// `name` as the document knows it.
    fn renamed(&self, r: Res, name: &str) -> String {
        let mut n = name.to_string();
        self.name(r, &mut n);
        n
    }
    fn paint(&self, p: &mut Paint) {
        match p {
            Paint::Pattern { pattern, .. } => self.name(Res::Pattern, pattern),
            Paint::Gradient(g) => {
                if let Some(s) = &mut g.swatch {
                    self.name(Res::Swatch, s);
                }
            }
            _ => {}
        }
        p.map_links(&mut |c, link, tint| {
            if let Some(l) = link {
                self.name(Res::Swatch, l);
                if let Some(base) = self.colors.get(l.as_str()) {
                    *c = base.tinted(*tint);
                }
            }
            false
        });
    }
    fn appearance(&self, a: &mut Appearance) {
        if !self.filters.is_empty() {
            let items = a.items.iter_mut().flat_map(|i| i.effects_mut().iter_mut());
            for e in a.effects.iter_mut().chain(items) {
                if let Some((name, xml)) = filter_of(e).and_then(|k| self.filters.get(&k)) {
                    e.params["name"] = json!(name);
                    e.params["xml"] = json!(xml);
                }
            }
        }
        // A 3D effect's mapped graphics follow their symbols' names in the document. Each name is
        // looked up once, so renames that chain (A to B, B to C) do not apply twice.
        let items = a.items.iter_mut().flat_map(|i| i.effects_mut().iter_mut());
        for e in a.effects.iter_mut().chain(items).filter(|e| vectorcraft_render::effects::takes_graphics(&e.id)) {
            let list = e.params.get_mut(vectorcraft_render::effects::GRAPHICS).and_then(Value::as_array_mut);
            for g in list.into_iter().flatten() {
                if let Some(Value::String(s)) = g.get_mut("symbol") {
                    self.name(Res::Symbol, s);
                }
            }
        }
        for it in &mut a.items {
            self.paint(it.paint_mut());
            if let AppearanceItem::Stroke(l) = it
                && let Some(b) = &mut l.brush
            {
                self.name(Res::Brush, b);
            }
        }
    }
    fn attrs(&self, a: &mut Map<String, Value>) {
        if let Some(to) =
            a.get("kinsoku_set").and_then(|v| serde_json::from_value::<KinsokuSet>(v.clone()).ok()).and_then(|set| self.kinsoku.get(&set))
        {
            match to {
                Some(set) => a.insert("kinsoku_set".into(), json!(set)),
                None => {
                    a.insert("kinsoku".into(), json!(Kinsoku::Hard));
                    a.insert("kinsoku_set".into(), Value::Null)
                }
            };
        }
        for k in STYLE_PAINTS {
            let Some(before) = style_paint(a, k) else { continue };
            let mut p = before.clone();
            self.paint(&mut p);
            if p != before
                && let Ok(v) = serde_json::to_value(&p)
            {
                a.insert(k.to_string(), v);
            }
        }
    }
    /// Relink a node tree (the mirror of [`node_refs`]).
    fn node(&self, n: &mut Node) {
        self.appearance(&mut n.appearance);
        n.extras_mut().graphic_style = n.extras().graphic_style.and_then(|id| self.styles.get(&id).copied());
        if let Some(m) = n.extras_if_any().and_then(|e| e.mask.as_mut()) {
            self.node(Arc::make_mut(&mut m.art));
        }
        if let Some(cm) = n.extras_if_any().and_then(|e| e.color_mask.as_deref_mut()) {
            self.name(Res::Image, &mut cm.key);
            self.name(Res::Image, &mut cm.from);
        }
        if let Some(g) = n.extras_if_any().and_then(|e| e.graph.as_deref_mut()) {
            for s in &mut g.series_paints {
                s.fill.iter_mut().chain(s.stroke.iter_mut()).for_each(|p| self.paint(p));
            }
            let l = &mut g.label_styles;
            for st in [&mut l.values, &mut l.categories, &mut l.legend].into_iter().flatten() {
                self.paint(&mut st.fill);
                self.paint(&mut st.stroke);
                if let Some(s) = &mut st.style_name {
                    self.name(Res::CharStyle, s);
                }
            }
            for pa in [&mut l.values_para, &mut l.categories_para, &mut l.legend_para].into_iter().flatten() {
                if let Some(s) = &mut pa.style_name {
                    self.name(Res::ParaStyle, s);
                }
                // A graph draws its labels again from the paragraph attributes it keeps for them.
                if let Some(to) = pa.kinsoku_set.as_deref().and_then(|set| self.kinsoku.get(set)) {
                    pa.kinsoku_set = to.clone();
                }
            }
            for r in [&mut l.values_rows, &mut l.categories_rows, &mut l.legend_rows].into_iter().flatten() {
                r.paint.fill.iter_mut().chain(r.paint.stroke.iter_mut()).for_each(|p| self.paint(p));
            }
        }
        match &mut n.kind {
            NodeKind::Text(t) => {
                for r in &mut t.runs {
                    self.paint(&mut r.style.fill);
                    self.paint(&mut r.style.stroke);
                    if let Some(s) = &mut r.style.style_name {
                        self.name(Res::CharStyle, s);
                    }
                }
                // Lines break anew when a paragraph's set takes other characters or Hard.
                let mut relaid = false;
                for pa in t.para_styles_mut() {
                    if let Some(s) = &mut pa.style_name {
                        self.name(Res::ParaStyle, s);
                    }
                    if let Some(set) = pa.kinsoku_set.as_deref()
                        && let Some(to) = self.kinsoku.get(set)
                    {
                        relaid |= to.as_deref().is_none_or(|to| kinsoku_chars(to) != kinsoku_chars(set));
                        pa.kinsoku_set = to.clone();
                    }
                }
                if relaid {
                    refresh_bounds(t);
                }
            }
            NodeKind::Image(im) => self.name(Res::Image, &mut im.key),
            NodeKind::PlacedDocument(pl) => self.name(Res::Image, &mut pl.key),
            NodeKind::SymbolInstance { symbol, .. } => self.name(Res::Symbol, symbol),
            _ => {}
        }
        for c in n.children_mut().into_iter().flatten() {
            self.node(Arc::make_mut(c));
        }
    }
}

/// The characters and sequences of Kinsoku set `s`, without its name.
fn kinsoku_chars(s: &KinsokuSet) -> (&[char], &[char], &[char], &[String]) {
    (&s.cannot_start, &s.cannot_end, &s.hanging, &s.inseparable)
}

/// Is `ours` (a document definition's art) the art of `theirs` (a pasted one's) once that is
/// relinked by `r`? Graphic style links don't count: they are relinked last.
fn same_art(r: &Relink, ours: &[Arc<Node>], theirs: &[Arc<Node>]) -> bool {
    let unlinked = Relink::default();
    let relinked = |r: &Relink, n: &Node| {
        let mut n = n.clone();
        r.node(&mut n);
        n
    };
    ours.len() == theirs.len() && ours.iter().zip(theirs).all(|(a, b)| relinked(&unlinked, a) == relinked(r, b))
}

/// Is `ours` (a document brush) brush `theirs` (a pasted one) once the art of that is relinked by
/// `r`? Graphic style links don't count, as in [`same_art`].
fn same_brush(r: &Relink, ours: &Brush, theirs: &Brush) -> bool {
    let relinked = |r: &Relink, b: &Brush| {
        let mut b = b.clone();
        b.art_mut().into_iter().for_each(|a| r.node(a));
        b
    };
    relinked(&Relink::default(), ours) == relinked(r, theirs)
}

/// A symbol's Symbol Options, which its document keeps by symbol name: copied with the symbol and
/// given to it under the name pasting adds it as.
#[derive(Clone, Debug, PartialEq)]
pub struct SymbolOptions {
    /// Natural size `(w, h)` (`None`: the document records none).
    pub size: Option<(f64, f64)>,
    pub registration: SymbolRegistration,
    pub export_type: SymbolExportType,
    /// 9-slice guides (`None`: 9-slice scaling off).
    pub nine_slice: Option<SymbolNineSlice>,
}

impl SymbolOptions {
    /// The options `d` keeps for symbol `name`.
    fn of(d: &Document, name: &str) -> Self {
        Self {
            size: stored_natural_size(d, name),
            registration: d.symbol_registration(name),
            export_type: d.symbol_export_type(name),
            nine_slice: d.symbol_nine_slice(name),
        }
    }

    /// Make them the options of `d`'s symbol `name`.
    fn give(&self, d: &mut Document, name: &str) {
        set_natural_size(d, name, self.size);
        d.set_symbol_registration(name, self.registration);
        d.set_symbol_export_type(name, self.export_type);
        d.set_symbol_nine_slice(name, self.nine_slice);
    }
}

/// A character or paragraph style, symbol (with the Symbol Options copied with it), pattern or
/// brush to place
/// ([`Clipboard::import_used`]).
#[derive(Clone, Copy)]
enum Def<'c> {
    Style(Res, &'c TextStyleDef),
    Symbol(&'c Symbol, Option<&'c SymbolOptions>),
    Pattern(&'c PatternDef),
    Brush(&'c Brush),
}

impl<'c> Def<'c> {
    fn key(&self) -> (Res, &'c str) {
        match self {
            Def::Style(r, s) => (*r, &s.name),
            Def::Symbol(s, _) => (Res::Symbol, &s.name),
            Def::Pattern(p) => (Res::Pattern, &p.name),
            Def::Brush(b) => (Res::Brush, &b.name),
        }
    }

    /// Call `f` with each resource it refers to by name.
    fn each_use(&self, f: &mut dyn FnMut(Res, &str)) {
        let mut g = |r: Ref<'_>| {
            if let Ref::Name(res, n) = r {
                f(res, n);
            }
        };
        match self {
            Def::Style(_, s) => attrs_refs(&s.attrs, &mut g),
            Def::Symbol(s, _) => node_refs(&s.art, &mut g),
            Def::Pattern(p) => p.art.iter().for_each(|a| node_refs(a, &mut g)),
            Def::Brush(b) => b.art().into_iter().for_each(|a| node_refs(a, &mut g)),
        }
    }

    /// Is it the definition `d` has under `name`, once relinked by `r`? (`brushes`: the brush
    /// library of `d`.)
    fn same_in(&self, d: &Document, brushes: &Brushes<'_>, name: &str, r: &Relink) -> bool {
        match self {
            Def::Style(res, s) => {
                let theirs = if *res == Res::CharStyle { &d.char_styles } else { &d.para_styles };
                let mut attrs = s.attrs.clone();
                r.attrs(&mut attrs);
                theirs.iter().any(|x| x.name == name && x.attrs == attrs)
            }
            Def::Symbol(s, options) => d.symbols.iter().any(|x| {
                x.name == name
                    && options.is_none_or(|o| SymbolOptions::of(d, name) == *o)
                    && same_art(r, std::slice::from_ref(&x.art), std::slice::from_ref(&s.art))
            }),
            Def::Pattern(p) => {
                let bare = |p: &PatternDef| PatternDef { name: String::new(), art: vec![], ..p.clone() };
                d.pattern(name).is_some_and(|x| bare(x) == bare(p) && same_art(r, &x.art, &p.art))
            }
            Def::Brush(b) => brushes.get().iter().any(|x| x.name == name && same_brush(r, x, b)),
        }
    }

    /// The names of its kind in `d` (`brushes`: its brush library) that are its name or a numbered
    /// copy of it.
    fn names_in(&self, d: &Document, brushes: &Brushes<'_>) -> Vec<String> {
        let (res, name) = self.key();
        let names: Vec<String> = match res {
            Res::CharStyle => d.char_styles.iter().map(|s| s.name.clone()).collect(),
            Res::ParaStyle => d.para_styles.iter().map(|s| s.name.clone()).collect(),
            Res::Symbol => d.symbols.iter().map(|s| s.name.clone()).collect(),
            Res::Brush => brushes.get().iter().map(|b| b.name.clone()).collect(),
            _ => d.patterns.iter().map(|p| p.name.clone()).collect(),
        };
        names.into_iter().filter(|n| copy_of(name, n)).collect()
    }
}

/// `defs` in rounds: each round takes those that use none of the others left, in their order.
/// → (those, the rest, which use each other in a loop or use one that does, in their order)
fn use_order(defs: Vec<Def<'_>>) -> (Vec<Def<'_>>, Vec<Def<'_>>) {
    let uses = uses_of(&defs);
    // The round of each definition; `None` for one in a loop or one that uses such a definition.
    let mut round: Vec<Option<usize>> = vec![None; defs.len()];
    // A group outside a loop holds one definition, and comes after the groups it uses.
    for (group, looped) in components(&uses) {
        for v in group.into_iter().filter(|_| !looped) {
            let r = uses[v].iter().try_fold(0, |r, &w| round.get(w).copied().flatten().map(|k| r.max(k + 1)));
            round[v] = r;
        }
    }
    let mut ready: Vec<(usize, usize)> = round.iter().enumerate().filter_map(|(i, r)| r.map(|r| (r, i))).collect();
    ready.sort_unstable();
    let order = ready.into_iter().filter_map(|(_, i)| defs.get(i).copied()).collect();
    let rest = defs.iter().zip(&round).filter(|(_, r)| r.is_none()).map(|(def, _)| *def).collect();
    (order, rest)
}

/// The positions in `defs` of the definitions each one uses, itself included.
fn uses_of(defs: &[Def<'_>]) -> Vec<Vec<usize>> {
    let mut at: BTreeMap<(Res, &str), Vec<usize>> = BTreeMap::new();
    for (i, def) in defs.iter().enumerate() {
        at.entry(def.key()).or_default().push(i);
    }
    defs.iter()
        .map(|def| {
            let mut to = vec![];
            def.each_use(&mut |res, n| to.extend(at.get(&(res, n)).into_iter().flatten()));
            to.sort_unstable();
            to.dedup();
            to
        })
        .collect()
}

/// `rest` ([`use_order`]'s definitions that use each other in a loop or use one that does) split
/// into its loops and the definitions outside a loop, each group after the groups it uses:
/// (positions in `rest`, in their order; true for a loop, definitions that use each other or one
/// that uses itself).
fn loop_groups(rest: &[Def<'_>]) -> Vec<(Vec<usize>, bool)> {
    components(&uses_of(rest))
}

/// The strongly connected components of the graph `uses` (each node's successors), each after
/// those it reaches: (nodes, in their order; true for a loop, two or more nodes or one that uses
/// itself).
fn components(uses: &[Vec<usize>]) -> Vec<(Vec<usize>, bool)> {
    // Tarjan's algorithm, with an explicit stack of calls.
    const NONE: usize = usize::MAX;
    let (mut index, mut low, mut on) = (vec![NONE; uses.len()], vec![0; uses.len()], vec![false; uses.len()]);
    let (mut stack, mut next, mut groups) = (vec![], 0, vec![]);
    for root in 0..uses.len() {
        if index[root] != NONE {
            continue;
        }
        let mut calls: Vec<(usize, usize)> = vec![(root, 0)];
        (index[root], low[root], on[root]) = (next, next, true);
        next += 1;
        stack.push(root);
        while let Some(&(v, k)) = calls.last() {
            if let Some(&w) = uses[v].get(k) {
                if let Some(top) = calls.last_mut() {
                    top.1 += 1;
                }
                if index[w] == NONE {
                    (index[w], low[w], on[w]) = (next, next, true);
                    next += 1;
                    stack.push(w);
                    calls.push((w, 0));
                } else if on[w] {
                    low[v] = low[v].min(index[w]);
                }
                continue;
            }
            calls.pop();
            if let Some(&(u, _)) = calls.last() {
                low[u] = low[u].min(low[v]);
            }
            if low[v] == index[v] {
                let mut group = vec![];
                while let Some(w) = stack.pop() {
                    on[w] = false;
                    group.push(w);
                    if w == v {
                        break;
                    }
                }
                group.sort_unstable();
                let looped = group.len() > 1 || uses[v].contains(&v);
                groups.push((group, looped));
            }
        }
    }
    groups
}

/// The names in `d` that `looped` (a loop of definitions that use each other, [`loop_groups`])
/// all have at once, each the same once relinked by `r` with those names: each one's name or a
/// numbered copy of it. The names are picked one definition at a time, in order, and each
/// definition is compared as soon as the names of those it uses are picked, so a pick that does
/// not fit is dropped with every combination that follows from it; at most 65,536 comparisons are
/// made. `None` when no combination fits, or none was found within that many comparisons. The brush
/// library of `d` is read once, the first time a brush is compared.
fn names_of_loop(d: &Document, looped: &[Def<'_>], r: &Relink) -> Option<Vec<String>> {
    let library = OnceCell::new();
    let brushes = Brushes::Lazy(d, &library);
    let names: Vec<Vec<String>> = looped.iter().map(|def| def.names_in(d, &brushes)).collect();
    if names.is_empty() || names.iter().any(Vec::is_empty) {
        return None;
    }
    // The definitions compared once the name at each position is picked.
    let mut compare_at: Vec<Vec<usize>> = vec![vec![]; looped.len()];
    for (i, uses) in uses_of(looped).iter().enumerate() {
        compare_at[uses.iter().copied().fold(i, usize::max)].push(i);
    }
    let (mut pick, mut guess, mut budget, mut k) = (vec![0usize; looped.len()], r.clone(), 1usize << 16, 0);
    loop {
        let (res, own) = looped[k].key();
        guess.renames.entry(res).or_default().insert(own.to_string(), names[k][pick[k]].clone());
        let fits = compare_at[k].iter().all(|&i| {
            budget = budget.saturating_sub(1);
            looped[i].same_in(d, &brushes, &names[i][pick[i]], &guess)
        });
        if fits && k + 1 == looped.len() {
            return Some(names.iter().zip(&pick).map(|(l, &p)| l[p].clone()).collect());
        }
        if budget == 0 {
            return None;
        }
        if fits {
            k += 1;
            pick[k] = 0;
            continue;
        }
        // The next name at this position, or back to the position before.
        loop {
            pick[k] += 1;
            if pick[k] < names[k].len() {
                break;
            }
            k = k.checked_sub(1)?;
        }
    }
}

/// What pasting did to the document's resources (part of the paste result).
#[derive(Debug, Default)]
pub(crate) struct Imported {
    relink: Relink,
    /// Whether relinking can change a pasted copy: a resource has another name or color in the
    /// document, or the objects link a graphic style.
    relinks: bool,
    /// Resources added to the document (renamed ones included).
    pub added: usize,
    /// Swatch conflicts resolved by merging.
    pub merged: usize,
    /// `{kind, from, to}` for every resource the pasted objects now know under another name: added
    /// renamed, or a copy added renamed by an earlier paste.
    pub renamed: Vec<Value>,
}

impl Imported {
    fn placed(&mut self, r: Res, from: &str, p: &Placed) {
        if matches!(p, Placed::Added | Placed::Renamed(_)) {
            self.added += 1;
        }
        if let Placed::Reused(to) | Placed::Renamed(to) = p {
            self.renamed.push(json!({"kind": r.id(), "from": from, "to": to}));
            self.relink.renames.entry(r).or_default().insert(from.to_string(), to.clone());
        }
    }
    /// Point a pasted copy at the document's resources.
    pub(crate) fn apply(&self, n: &mut Node) {
        if self.relinks {
            self.relink.node(n);
        }
    }
}

impl Clipboard {
    /// `nodes` (from `src`) and the resources of `src` they use.
    pub(crate) fn gather(src: &Document, nodes: Vec<Node>) -> Self {
        let library = OnceCell::new();
        let used = Used::of(&Defs::of(src, &library), &nodes);
        Self::with_used(src, &used, &library, nodes)
    }

    /// `nodes` and the resources of `src` that `used` names (`library`: the brush library of `src`,
    /// read when a brush was used).
    fn with_used(src: &Document, used: &Used, library: &OnceCell<Vec<Brush>>, nodes: Vec<Node>) -> Self {
        let pick = |r: Res, list: &[TextStyleDef]| list.iter().filter(|s| used.has(r, &s.name)).cloned().collect::<Vec<_>>();
        // Read when a brush was used.
        let brushes = match (used.names.get(&Res::Brush), library.get()) {
            (Some(names), Some(library)) => library.iter().filter(|b| names.contains(&b.name)).cloned().collect(),
            _ => vec![],
        };
        Self {
            images: src.images.iter().filter(|(k, _)| used.has(Res::Image, k)).map(|(k, b)| (k.clone(), b.clone())).collect(),
            symbols: src.symbols.iter().filter(|s| used.has(Res::Symbol, &s.name)).cloned().collect(),
            symbol_options: src
                .symbols
                .iter()
                .filter(|s| used.has(Res::Symbol, &s.name))
                .map(|s| (s.name.clone(), SymbolOptions::of(src, &s.name)))
                .collect(),
            patterns: src.patterns.iter().filter(|p| used.has(Res::Pattern, &p.name)).cloned().collect(),
            swatches: src.swatches_iter().filter(|w| used.carries(w)).cloned().collect(),
            graphic_styles: src.graphic_styles.iter().filter(|g| g.id != 0 && used.styles.contains(&g.id)).cloned().collect(),
            char_styles: pick(Res::CharStyle, &src.char_styles),
            para_styles: pick(Res::ParaStyle, &src.para_styles),
            brushes,
            nodes,
            ..Default::default()
        }
    }

    /// The resources the objects use now (Paste without Formatting leaves text styles behind).
    pub(super) fn used(&self) -> Used {
        let defs = Defs {
            swatches: self.swatches.iter().collect(),
            symbols: &self.symbols,
            patterns: &self.patterns,
            styles: &self.graphic_styles,
            char_styles: &self.char_styles,
            para_styles: &self.para_styles,
            brushes: Brushes::Ready(&self.brushes),
            index: DefIndex::default(),
        };
        Used::of(&defs, &self.nodes)
    }

    /// The used global and spot colours whose name `d` gives another one: (pasted, document).
    pub(super) fn conflicts<'a, 'd>(&'a self, d: &'d Document, used: &Used) -> Vec<(&'a Swatch, &'d Swatch)> {
        self.swatches
            .iter()
            .filter(|w| is_base(w) && used.has(Res::Swatch, &w.name))
            .filter_map(|w| d.swatch(&w.name).filter(|t| is_base(t) && (t.paint != w.paint || t.spot != w.spot)).map(|t| (w, t)))
            .collect()
    }

    /// Add the resources the objects use to `d` (inside the paste's undo step) and work out how
    /// pasted copies relink to them. Each style, symbol, pattern and brush is placed after those it
    /// uses, as [`import_text_styles`] places them, so that a style that paints with a pattern added
    /// under a free name ("Dots 2") is compared and added with that name. An error, with `d`
    /// unchanged, when a pasted SVG filter finds no free name ([`Self::filter_links`]).
    pub(crate) fn import_into(&self, d: &mut Document, choices: &SwatchChoices, same_doc: bool) -> Result<Imported> {
        self.import_used(d, &self.used(), choices, same_doc)
    }

    /// [`Self::import_into`] for the resources `used` names. Each style, symbol, pattern and brush
    /// comes after the styles, symbols, patterns and brushes it uses ([`use_order`]), so that it is
    /// compared and relinked with their names here, and each loop of definitions that use each
    /// other is matched on its own ([`loop_groups`]). Pasted back into its own document
    /// (`same_doc`), each definition of a loop is the document's of its name.
    fn import_used(&self, d: &mut Document, used: &Used, choices: &SwatchChoices, same_doc: bool) -> Result<Imported> {
        let mut out = Imported::default();
        out.relink.filters = self.filter_links(d, same_doc)?;
        self.import_images(d, used, same_doc, &mut out);
        self.import_swatches(d, used, choices, same_doc, &mut out);
        self.import_kinsoku_sets(d, used, same_doc, &mut out);
        let mut defs: Vec<Def<'_>> = vec![];
        for (r, list) in [(Res::CharStyle, &self.char_styles), (Res::ParaStyle, &self.para_styles)] {
            defs.extend(list.iter().filter(|s| used.has(r, &s.name)).map(|s| Def::Style(r, s)));
        }
        defs.extend(self.symbols.iter().filter(|s| used.has(Res::Symbol, &s.name)).map(|s| Def::Symbol(s, self.symbol_options.get(&s.name))));
        defs.extend(self.patterns.iter().filter(|p| used.has(Res::Pattern, &p.name)).map(Def::Pattern));
        defs.extend(self.brushes.iter().filter(|b| used.has(Res::Brush, &b.name)).map(Def::Brush));
        // Groups to place in order; true for a loop of definitions that use each other.
        let (order, rest) = use_order(defs);
        let loops = loop_groups(&rest).into_iter().map(|(at, looped)| (at.into_iter().filter_map(|i| rest.get(i).copied()).collect(), looped));
        let groups: Vec<(Vec<Def<'_>>, bool)> = std::iter::once((order, false)).chain(loops).collect();
        // Styles (their colors relinked first), symbols, patterns and brushes (compared once
        // relinked, symbols with their options too). The styles, symbols, patterns and brushes
        // added are relinked at the end, when every new name and graphic style id is known: a loop
        // of uses may place one before a definition it uses.
        let (mut added_symbols, mut added_patterns, mut added_brushes, mut added_styles) = (HashSet::new(), HashSet::new(), HashSet::new(), vec![]);
        // Each definition, whether it is in a loop, and the loop it starts.
        let steps = groups
            .iter()
            .flat_map(|(group, looped)| group.iter().enumerate().map(move |(i, def)| (*def, *looped, (*looped && i == 0).then_some(group))));
        // In a loop the document does not have all at once, every definition uses one that is
        // added under a free name, so each one is added too.
        let mut fresh = false;
        // The names in the document of the loop being placed, when it has them all at once.
        let mut matched: BTreeMap<(Res, String), String> = BTreeMap::new();
        // The document's brush library, read the first time a brush is placed and stored again
        // after each brush added; nothing else in the loop changes it.
        let mut library: Option<Vec<Brush>> = None;
        for (def, looped, starts) in steps {
            fresh &= looped;
            if !looped {
                matched.clear();
            }
            // A loop of uses that the document has all at once is known under those names before
            // its definitions are compared. In its own document, each definition is the
            // document's of its name, as outside a loop.
            if let Some(group) = starts.filter(|_| !same_doc) {
                matched.clear();
                match names_of_loop(d, group, &out.relink) {
                    Some(names) => {
                        for (def, name) in group.iter().zip(names) {
                            let (res, own) = def.key();
                            if own != name {
                                out.relink.renames.entry(res).or_default().insert(own.to_string(), name.clone());
                            }
                            matched.insert((res, own.to_string()), name);
                        }
                        fresh = false;
                    }
                    None => fresh = true,
                }
            }
            // A definition of such a loop is the document's under the name found for it.
            let (res, own) = def.key();
            if let Some(name) = matched.get(&(res, own.to_string())) {
                let p = if name == own { Placed::Existing } else { Placed::Reused(name.clone()) };
                out.placed(res, own, &p);
                continue;
            }
            match def {
                Def::Style(r, def) => {
                    let mut s = def.clone();
                    out.relink.attrs(&mut s.attrs);
                    let theirs = if r == Res::CharStyle { &mut d.char_styles } else { &mut d.para_styles };
                    let p = place(theirs, &s, same_doc, |a, b| !fresh && a == b);
                    if let Some(name) = p.new_name(&def.name) {
                        added_styles.push((r, name, def));
                    }
                    out.placed(r, &def.name, &p);
                }
                Def::Symbol(def, options) => {
                    let theirs: HashMap<String, SymbolOptions> = match options {
                        Some(_) => d
                            .symbols
                            .iter()
                            .filter(|s| copy_of(&def.name, &s.name))
                            .map(|s| (s.name.clone(), SymbolOptions::of(d, &s.name)))
                            .collect(),
                        None => HashMap::new(),
                    };
                    let r = &out.relink;
                    let p = place(&mut d.symbols, def, same_doc, |a, b| {
                        !fresh
                            && options.is_none_or(|o| theirs.get(&a.name) == Some(o))
                            && same_art(r, std::slice::from_ref(&a.art), std::slice::from_ref(&b.art))
                    });
                    if let Some(name) = p.new_name(&def.name) {
                        if let Some(o) = options {
                            o.give(d, &name);
                        }
                        added_symbols.insert(name);
                    }
                    out.placed(Res::Symbol, &def.name, &p);
                }
                Def::Pattern(def) => {
                    // Tile options alike, and the art once relinked.
                    let bare = |p: &PatternDef| PatternDef { name: String::new(), art: vec![], ..p.clone() };
                    let same = |n: &str| !fresh && d.pattern(n).is_some_and(|x| bare(x) == bare(def) && same_art(&out.relink, &x.art, &def.art));
                    let p = existing(&def.name, d.patterns.iter().map(|p| p.name.as_str()), same_doc, same).unwrap_or_else(|| {
                        // Its swatch comes along under the same name.
                        let name = unique_name(&def.name, |n| d.pattern(n).is_some() || d.swatch_name_taken(n));
                        add_pattern(d, PatternDef { name: name.clone(), ..def.clone() });
                        Placed::added(&def.name, name)
                    });
                    added_patterns.extend(p.new_name(&def.name));
                    out.placed(Res::Pattern, &def.name, &p);
                }
                Def::Brush(b) => {
                    let lib = library.get_or_insert_with(|| vectorcraft_brush::library(d));
                    let r = &out.relink;
                    let p = place(lib, b, same_doc, |ours, theirs| !fresh && same_brush(r, ours, theirs));
                    if let Some(name) = p.new_name(&b.name) {
                        vectorcraft_brush::store(d, lib);
                        added_brushes.insert(name);
                    }
                    out.placed(Res::Brush, &b.name, &p);
                }
            }
        }
        self.import_graphic_styles(d, used, same_doc, &mut out);
        for (r, name, def) in added_styles {
            let theirs = if r == Res::CharStyle { &mut d.char_styles } else { &mut d.para_styles };
            if let Some(s) = theirs.iter_mut().find(|s| s.name == name) {
                s.attrs = def.attrs.clone();
                out.relink.attrs(&mut s.attrs);
            }
        }
        for s in d.symbols.iter_mut().filter(|s| added_symbols.contains(&s.name)) {
            out.relink.node(Arc::make_mut(&mut s.art));
        }
        for p in d.patterns.iter_mut().filter(|p| added_patterns.contains(&p.name)) {
            for a in &mut p.art {
                out.relink.node(Arc::make_mut(a));
            }
        }
        if !added_brushes.is_empty() {
            let mut lib = vectorcraft_brush::library(d);
            for b in lib.iter_mut().filter(|b| added_brushes.contains(&b.name)) {
                b.art_mut().into_iter().for_each(|a| out.relink.node(a));
            }
            vectorcraft_brush::store(d, &lib);
        }
        out.relinks = !(out.relink.renames.is_empty()
            && out.relink.filters.is_empty()
            && out.relink.styles.is_empty()
            && out.relink.colors.is_empty()
            && out.relink.kinsoku.is_empty()
            && used.styles.is_empty());
        Ok(out)
    }

    /// Add the custom Kinsoku sets that the pasted paragraphs and the used paragraph styles hold
    /// to `d`'s sets, and record in [`Relink::kinsoku`] which set of `d` each one takes. A set
    /// takes `d`'s set of its name (in any case) when that has the same characters, or, pasted
    /// back into its own document, in any case; else a numbered copy of it ("Tight 2") with the
    /// same characters. Any other set is added under a free name, so editing `d`'s set of that
    /// name leaves the pasted paragraphs alone. When `d` already holds [`KINSOKU_SETS_MAX`] sets,
    /// or no free name fits [`KINSOKU_NAME_MAX`], the paragraphs and styles take Hard.
    fn import_kinsoku_sets(&self, d: &mut Document, used: &Used, same_doc: bool, out: &mut Imported) {
        let mut found: Vec<KinsokuSet> = vec![];
        let mut seen: HashSet<KinsokuSet> = HashSet::new();
        let mut note = |set: &KinsokuSet| {
            if !seen.contains(set) {
                seen.insert(set.clone());
                found.push(set.clone());
            }
        };
        let mut nodes = |n: &Node| {
            // Opacity-mask art included.
            vectorcraft_doc::swatches::walk_with_masks(n, &mut |m| {
                if let NodeKind::Text(t) = &m.kind {
                    t.para_styles().filter_map(|p| p.kinsoku_set.as_deref()).for_each(&mut note);
                }
                // The paragraph attributes a graph keeps for its labels.
                if let Some(spec) = m.extras().graph.as_deref() {
                    let l = &spec.label_styles;
                    [&l.values_para, &l.categories_para, &l.legend_para]
                        .into_iter()
                        .flatten()
                        .filter_map(|p| p.kinsoku_set.as_deref())
                        .for_each(&mut note);
                }
            });
        };
        self.nodes.iter().for_each(&mut nodes);
        self.symbols.iter().filter(|s| used.has(Res::Symbol, &s.name)).for_each(|s| nodes(s.art.as_ref()));
        self.patterns.iter().filter(|p| used.has(Res::Pattern, &p.name)).flat_map(|p| p.art.iter()).for_each(|a| nodes(a.as_ref()));
        self.brushes.iter().filter(|b| used.has(Res::Brush, &b.name)).flat_map(Brush::art).for_each(&mut nodes);
        let styles = self.para_styles.iter().filter(|s| used.has(Res::ParaStyle, &s.name));
        for set in styles.filter_map(|s| s.attrs.get("kinsoku_set")).filter_map(|v| serde_json::from_value::<KinsokuSet>(v.clone()).ok()) {
            note(&set);
        }
        for set in found {
            let lower = set.name.to_lowercase();
            let mut copies: Vec<&KinsokuSet> = d.kinsoku_sets.iter().filter(|x| copy_of(&lower, &x.name.to_lowercase())).collect();
            // The set of its name first.
            copies.sort_by_key(|x| !KinsokuSet::same_name(&x.name, &set.name));
            let own = copies.first().copied().filter(|x| same_doc && KinsokuSet::same_name(&x.name, &set.name));
            let to = match own.or_else(|| copies.iter().copied().find(|x| kinsoku_chars(x) == kinsoku_chars(&set))) {
                Some(x) => {
                    if !KinsokuSet::same_name(&x.name, &set.name) {
                        out.renamed.push(json!({"kind": "kinsokuSet", "from": set.name, "to": x.name}));
                    }
                    Some(Arc::new(x.clone()))
                }
                None => {
                    let name = unique_name(&set.name, |n| KinsokuSet::named(&d.kinsoku_sets, n).is_some());
                    (d.kinsoku_sets.len() < KINSOKU_SETS_MAX && name.chars().count() <= KINSOKU_NAME_MAX).then(|| {
                        let added = KinsokuSet { name: name.clone(), ..set.clone() };
                        d.kinsoku_sets.push(added.clone());
                        out.added += 1;
                        if name != set.name {
                            out.renamed.push(json!({"kind": "kinsokuSet", "from": set.name, "to": name}));
                        }
                        Arc::new(added)
                    })
                }
            };
            if to.as_deref() != Some(&set) {
                out.relink.kinsoku.insert(set, to);
            }
        }
    }

    /// How the pasted SVG filter effects relink ([`Relink::filters`]). Each names the document's
    /// filter of its name; one that draws another definition than that filter takes the document's
    /// filter when pasted back into the document it came from, and elsewhere keeps its own under a
    /// name no filter or effect of the document gives, so editing or deleting the document's filter leaves it alone. When that name would
    /// make the filter too long to draw, the free name is a shorter start of its name ("Shif"), else
    /// a free letter, digit or other printable ASCII character; the filter's id is always its name. An error when no free name fits.
    fn filter_links(&self, d: &Document, same_doc: bool) -> Result<BTreeMap<(String, String), (String, String)>> {
        let mut found = BTreeSet::new();
        for n in &self.nodes {
            node_filters(n, &mut found);
        }
        for s in &self.symbols {
            node_filters(&s.art, &mut found);
        }
        for a in self.patterns.iter().flat_map(|p| p.art.iter()) {
            node_filters(a, &mut found);
        }
        for a in self.brushes.iter().flat_map(Brush::art) {
            node_filters(a, &mut found);
        }
        for g in &self.graphic_styles {
            appearance_filters(&g.appearance, &mut found);
        }
        let mut taken: BTreeSet<String> = d.svg_filters.iter().map(|f| f.name.clone()).chain(found.iter().map(|(n, _)| n.clone())).collect();
        let mut effects_read = false;
        let mut links = BTreeMap::new();
        for (name, xml) in found {
            let Some(held) = d.svg_filters.iter().find(|f| f.name == name).filter(|f| f.xml != xml.trim()) else { continue };
            let to = if same_doc {
                (name.clone(), held.xml.clone())
            } else {
                // The names the document's effects give are taken too, those whose filter was
                // deleted included: defining a filter of that name later changes every effect that
                // names it. They are read at the first rename only.
                if !effects_read {
                    effects_read = true;
                    for root in d.art_roots() {
                        node_filter_names(root, &mut taken);
                    }
                    for a in vectorcraft_brush::library(d).iter().flat_map(Brush::art) {
                        node_filter_names(a, &mut taken);
                    }
                    for g in &d.graphic_styles {
                        appearance_filter_names(&g.appearance, &mut taken);
                    }
                }
                // Free names made from the starts of the name, longest first, then free letters and
                // digits, then other single printable ASCII characters: the first under which the
                // filter still draws.
                let mut ends: Vec<usize> = name.char_indices().map(|(i, _)| i).skip(1).collect();
                ends.push(name.len());
                let starts = ends.iter().rev().filter_map(|&end| name.get(..end).map(str::trim_end)).filter(|b| !b.is_empty());
                let shorter = starts.map(|b| unique_name(b, |n| taken.contains(n)));
                let single = ('a'..='z')
                    .chain('0'..='9')
                    .chain('A'..='Z')
                    .chain(('!'..='~').filter(|c| !c.is_ascii_alphanumeric()))
                    .map(String::from)
                    .filter(|n| !taken.contains(n));
                let renamed = shorter.chain(single).find_map(|new| {
                    let drawn = vectorcraft_render::effects::svg_filter::with_id(&xml, &new)
                        .filter(|x| vectorcraft_render::effects::svg_filter::check(x).is_ok_and(|id| id == new))?;
                    Some((new, drawn))
                });
                let Some((new, drawn)) = renamed else {
                    return Err(EngineError::Other(format!(
                        "the pasted SVG filter {name:?} draws differently from the document's filter of that name, and no free name fits it"
                    )));
                };
                taken.insert(new.clone());
                (new, drawn)
            };
            links.insert((name, xml), to);
        }
        Ok(links)
    }

    /// Image blobs: a different blob under a key the document uses gets a new key.
    fn import_images(&self, d: &mut Document, used: &Used, same_doc: bool, out: &mut Imported) {
        for (key, blob) in self.images.iter().filter(|(k, _)| used.has(Res::Image, k)) {
            let p = existing(key, d.images.keys().map(String::as_str), same_doc, |k| d.images.get(k) == Some(blob)).unwrap_or_else(|| {
                let k = unique_name(key, |n| d.images.contains_key(n));
                d.images.insert(k.clone(), blob.clone());
                Placed::added(key, k)
            });
            out.placed(Res::Image, key, &p);
        }
    }

    /// Global and spot colours (a name conflict merges or adds as `choices` say), then the tint
    /// swatches of the tints used and the gradient swatches.
    fn import_swatches(&self, d: &mut Document, used: &Used, choices: &SwatchChoices, same_doc: bool, out: &mut Imported) {
        let conflicts: BTreeSet<String> =
            if same_doc { BTreeSet::new() } else { self.conflicts(d, used).into_iter().map(|(w, _)| w.name.clone()).collect() };
        let bases: Vec<&Swatch> = self.swatches.iter().filter(|w| is_base(w) && used.has(Res::Swatch, &w.name)).collect();
        for w in &bases {
            let p = if conflicts.contains(&w.name) && choices.of(&w.name) == SwatchChoice::Merge {
                out.merged += 1;
                Placed::Existing
            } else {
                add_swatch(d, w, same_doc)
            };
            out.placed(Res::Swatch, &w.name, &p);
        }
        // Linked colours show the document's swatch of their link.
        for w in &bases {
            let name = out.relink.renamed(Res::Swatch, &w.name);
            if let Some(c) = d.global_color(&name) {
                out.relink.colors.insert(name, c);
            }
        }
        // Then the tint and gradient swatches, linked like the objects.
        for def in self.swatches.iter().filter(|w| !is_base(w) && used.carries(w)) {
            let mut w = def.clone();
            out.relink.paint(&mut w.paint);
            let p = add_swatch(d, &w, same_doc);
            out.placed(Res::Swatch, &def.name, &p);
        }
    }

    /// Graphic styles: in the source document by id, elsewhere by name and look.
    fn import_graphic_styles(&self, d: &mut Document, used: &Used, same_doc: bool, out: &mut Imported) {
        for g in self.graphic_styles.iter().filter(|g| used.styles.contains(&g.id)) {
            let mut new = g.clone();
            out.relink.appearance(&mut new.appearance);
            let found = match d.graphic_styles.iter().position(|x| same_doc && x.id == g.id) {
                Some(i) => Some(i),
                None => {
                    let same = |n: &str| d.graphic_style(n).is_some_and(|x| x.same_look(&new));
                    let p = existing(&g.name, d.graphic_styles.iter().map(|x| x.name.as_str()), same_doc, same);
                    // Objects follow the style's id, not its name.
                    p.and_then(|p| d.graphic_style_index(if let Placed::Reused(n) = &p { n } else { &g.name }))
                }
            };
            let i = match found {
                Some(i) => i,
                None => {
                    let name = unique_name(&g.name, |n| d.graphic_style(n).is_some());
                    out.placed(Res::GraphicStyle, &g.name, &Placed::added(&g.name, name.clone()));
                    let id = d.next_graphic_style_id();
                    d.graphic_styles.push(GraphicStyle { name, id, ..new });
                    d.graphic_styles.len() - 1
                }
            };
            out.relink.styles.insert(g.id, d.graphic_style_id(i));
        }
    }
}

/// Where a loaded text style is in the document ([`import_text_styles`]).
pub(crate) struct LoadedStyle {
    /// A paragraph style (else a character style).
    pub para: bool,
    /// Its name in the document.
    pub name: String,
    /// It was added (else the document's own was the same).
    pub added: bool,
}

/// Add every character style of `src` (with `chars`) and every paragraph style (with `paras`) to
/// `d`, with the swatches, patterns and what their art uses (styles of the other kind included)
/// that the styles' fill and stroke lead to, as Paste adds those of pasted type
/// ([`Clipboard::import_into`]; a global or spot color whose name `d` gives another color is
/// `d`'s). Each style, symbol and pattern is placed after those it uses, so that a style follows
/// a pattern added under a free name and a pattern follows a style renamed on the way in. A style
/// `d` has under its name, or a numbered copy of it ("Body 2"), that is the same once relinked is
/// used; any other is added under a free name. → each style of `src` placed, in order, character
/// styles first. An error, as [`Clipboard::import_into`] gives.
pub(crate) fn import_text_styles(src: &Document, d: &mut Document, chars: bool, paras: bool) -> Result<Vec<LoadedStyle>> {
    let library = OnceCell::new();
    let used = Used::of_text_styles(&Defs::of(src, &library), chars, paras);
    let clip = Clipboard::with_used(src, &used, &library, vec![]);
    let before = [&d.char_styles, &d.para_styles].map(|l| l.iter().map(|s| s.name.clone()).collect::<BTreeSet<_>>());
    let out = clip.import_used(d, &used, &SwatchChoices::default(), false)?;
    let mut placed: Vec<LoadedStyle> = vec![];
    for (para, r, list) in [(false, Res::CharStyle, &clip.char_styles), (true, Res::ParaStyle, &clip.para_styles)] {
        for s in list {
            let name = out.relink.renamed(r, &s.name);
            let again = placed.iter().any(|p| p.para == para && p.name == name);
            let added = !again && !before[usize::from(para)].contains(&name);
            placed.push(LoadedStyle { para, name, added });
        }
    }
    Ok(placed)
}

/// Swatch `w` in `d` ([`existing`]), else added under a free name (swatches and colour groups
/// share one namespace).
fn add_swatch(d: &mut Document, w: &Swatch, same_doc: bool) -> Placed {
    let same = |n: &str| d.swatch(n).is_some_and(|t| *t == Swatch { name: n.to_string(), ..w.clone() });
    existing(&w.name, d.swatches_iter().map(|t| t.name.as_str()), same_doc, same).unwrap_or_else(|| {
        let name = d.free_swatch_name(&w.name);
        d.swatches.push(Swatch { name: name.clone(), ..w.clone() });
        Placed::added(&w.name, name)
    })
}
