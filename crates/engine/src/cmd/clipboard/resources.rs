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
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use serde_json::{Map, Value, json};
use vectorcraft_brush::Brush;
use vectorcraft_color::{Color, Paint, Swatch};
use vectorcraft_doc::{Appearance, AppearanceItem, Document, GraphicStyle, Node, NodeKind, PatternDef, Symbol, TextStyleDef};

use super::super::patterncmds::add_pattern;
use super::super::{Result, bad, unique_name};
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
        let mut todo: Vec<Source<'a>> = nodes.into_iter().map(Source::Node).collect();
        while let Some(src) = todo.pop() {
            let mut found = |r: Ref<'_>| used.note(defs, r, &mut todo);
            match src {
                Source::Node(n) => node_refs(n, &mut found),
                Source::Appearance(a) => appearance_refs(a, &mut found),
                Source::Attrs(a) => attrs_refs(a, &mut found),
                Source::Paint(p) => paint_refs(p, &mut found),
            }
        }
        used
    }

    /// Note `r`; a definition seen for the first time joins `todo`.
    fn note<'a>(&mut self, defs: &Defs<'a>, r: Ref<'_>, todo: &mut Vec<Source<'a>>) {
        let (res, name) = match r {
            Ref::Style(id) => {
                if self.styles.insert(id) {
                    todo.extend(defs.styles.iter().filter(|g| g.id != 0 && g.id == id).map(|g| Source::Appearance(&g.appearance)));
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
        match res {
            // A tint swatch's base, a gradient swatch's linked stops.
            Res::Swatch => todo.extend(defs.swatches.iter().filter(|w| w.name == name).map(|w| Source::Paint(&w.paint))),
            Res::Symbol => todo.extend(defs.symbols.iter().filter(|s| s.name == name).map(|s| Source::Node(&s.art))),
            Res::Pattern => todo.extend(defs.patterns.iter().filter(|p| p.name == name).flat_map(|p| p.art.iter().map(|a| Source::Node(a)))),
            Res::CharStyle => todo.extend(defs.char_styles.iter().filter(|s| s.name == name).map(|s| Source::Attrs(&s.attrs))),
            Res::ParaStyle => todo.extend(defs.para_styles.iter().filter(|s| s.name == name).map(|s| Source::Attrs(&s.attrs))),
            // A scatter or art brush's art, a pattern brush's tiles.
            Res::Brush => todo.extend(defs.brushes.get().iter().filter(|b| b.name == name).flat_map(|b| b.art().into_iter().map(Source::Node))),
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
#[derive(Debug, Default)]
struct Relink {
    renames: BTreeMap<Res, BTreeMap<String, String>>,
    /// Graphic style id on the clipboard → its id in the document. Links to other ids are dropped.
    styles: BTreeMap<u32, u32>,
    /// Swatch (its name in the document) → the colour linked colours show at 100 %.
    colors: BTreeMap<String, Color>,
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
        match &mut n.kind {
            NodeKind::Text(t) => {
                for r in &mut t.runs {
                    self.paint(&mut r.style.fill);
                    self.paint(&mut r.style.stroke);
                    if let Some(s) = &mut r.style.style_name {
                        self.name(Res::CharStyle, s);
                    }
                }
                for pa in t.para_styles_mut() {
                    if let Some(s) = &mut pa.style_name {
                        self.name(Res::ParaStyle, s);
                    }
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

/// Does brush `b`'s art show symbols or patterns?
fn shows_definitions(b: &Brush) -> bool {
    let mut found = false;
    for a in b.art() {
        node_refs(a, &mut |r| found |= matches!(r, Ref::Name(Res::Symbol | Res::Pattern, _)));
    }
    found
}

/// What pasting did to the document's resources (part of the paste result).
#[derive(Debug, Default)]
pub(crate) struct Imported {
    relink: Relink,
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
        self.relink.node(n);
    }
}

impl Clipboard {
    /// `nodes` (from `src`) and the resources of `src` they use.
    pub(crate) fn gather(src: &Document, nodes: Vec<Node>) -> Self {
        let library = OnceCell::new();
        let used = Used::of(&Defs::of(src, &library), &nodes);
        let pick = |r: Res, list: &[TextStyleDef]| list.iter().filter(|s| used.has(r, &s.name)).cloned().collect::<Vec<_>>();
        // Read when a brush was used.
        let brushes = match (used.names.get(&Res::Brush), library.get()) {
            (Some(names), Some(library)) => library.iter().filter(|b| names.contains(&b.name)).cloned().collect(),
            _ => vec![],
        };
        Self {
            images: src.images.iter().filter(|(k, _)| used.has(Res::Image, k)).map(|(k, b)| (k.clone(), b.clone())).collect(),
            symbols: src.symbols.iter().filter(|s| used.has(Res::Symbol, &s.name)).cloned().collect(),
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
    /// pasted copies relink to them.
    pub(crate) fn import_into(&self, d: &mut Document, choices: &SwatchChoices, same_doc: bool) -> Imported {
        let used = self.used();
        let mut out = Imported::default();
        self.import_images(d, &used, same_doc, &mut out);
        self.import_swatches(d, &used, choices, same_doc, &mut out);
        // Brushes whose art shows no symbols or patterns, before the symbols and patterns whose art
        // they may brush; the others once those are in.
        let mut added_brushes = self.import_brushes(d, &used, same_doc, false, &mut out);
        // Character and paragraph styles (their colours relinked first).
        for (r, mine, theirs) in [(Res::CharStyle, &self.char_styles, &mut d.char_styles), (Res::ParaStyle, &self.para_styles, &mut d.para_styles)] {
            for def in mine.iter().filter(|s| used.has(r, &s.name)) {
                let mut s = def.clone();
                out.relink.attrs(&mut s.attrs);
                let p = place(theirs, &s, same_doc, PartialEq::eq);
                out.placed(r, &def.name, &p);
            }
        }
        // Symbols and patterns, compared once relinked. Those added are relinked at the end, when
        // every new name and graphic style id is known.
        let mut added_symbols = vec![];
        for def in self.symbols.iter().filter(|s| used.has(Res::Symbol, &s.name)) {
            let r = &out.relink;
            let p = place(&mut d.symbols, def, same_doc, |a, b| same_art(r, std::slice::from_ref(&a.art), std::slice::from_ref(&b.art)));
            added_symbols.extend(p.new_name(&def.name));
            out.placed(Res::Symbol, &def.name, &p);
        }
        let mut added_patterns = vec![];
        for def in self.patterns.iter().filter(|p| used.has(Res::Pattern, &p.name)) {
            // Tile options alike, and the art once relinked.
            let bare = |p: &PatternDef| PatternDef { name: String::new(), art: vec![], ..p.clone() };
            let same = |n: &str| d.pattern(n).is_some_and(|x| bare(x) == bare(def) && same_art(&out.relink, &x.art, &def.art));
            let p = existing(&def.name, d.patterns.iter().map(|p| p.name.as_str()), same_doc, same).unwrap_or_else(|| {
                // Its swatch comes along under the same name.
                let name = unique_name(&def.name, |n| d.pattern(n).is_some() || d.swatch_name_taken(n));
                add_pattern(d, PatternDef { name: name.clone(), ..def.clone() });
                Placed::added(&def.name, name)
            });
            added_patterns.extend(p.new_name(&def.name));
            out.placed(Res::Pattern, &def.name, &p);
        }
        added_brushes.extend(self.import_brushes(d, &used, same_doc, true, &mut out));
        self.import_graphic_styles(d, &used, same_doc, &mut out);
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
        out
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

    /// The brushes whose art shows symbols or patterns (`late`), or the others: each one the
    /// document has (its art compared once relinked) is used, the others are added. → the names of
    /// those added, whose art is relinked at the end.
    fn import_brushes(&self, d: &mut Document, used: &Used, same_doc: bool, late: bool, out: &mut Imported) -> Vec<String> {
        let brushes: Vec<&Brush> = self.brushes.iter().filter(|b| used.has(Res::Brush, &b.name) && shows_definitions(b) == late).collect();
        if brushes.is_empty() {
            return vec![];
        }
        let mut lib = vectorcraft_brush::library(d);
        let mut added = vec![];
        for b in brushes {
            let r = &out.relink;
            let p = place(&mut lib, b, same_doc, |ours, theirs| same_brush(r, ours, theirs));
            added.extend(p.new_name(&b.name));
            out.placed(Res::Brush, &b.name, &p);
        }
        if !added.is_empty() {
            vectorcraft_brush::store(d, &lib);
        }
        added
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
