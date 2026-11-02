//! The program's art and swatches (see [`super`] for the form).

use std::collections::{HashMap, HashSet};
use std::fmt::Write;
use std::sync::Arc;

use vectorcraft_color::cms::Cms;
use vectorcraft_color::swatch::{REGISTRATION, Swatch};
use vectorcraft_color::{BlendMode, Color, Paint};
use vectorcraft_doc::{AppearanceItem, Document, FillLayer, LAYER_COLORS, LayerColor, Node, NodeKind, StrokeLayer};
use vectorcraft_effects::stroke;
use vectorcraft_geom::{Affine, FillRule, PathData, Rect, Shape, SubPath};
use vectorcraft_pathops::BoolOp;

use crate::ai_ops::{self, average, unit};
use crate::ps;

/// How deep symbols, live objects and pattern tiles may nest in one another (deeper art is left
/// out). Groups and layers need no limit of their own: a document is never deeper than
/// [`vectorcraft_doc::MAX_DEPTH`].
const MAX_NEST: u32 = 8;
/// The side of the image whose pixels are averaged (larger ones are scaled down to it first).
const AVERAGE_SIDE: u32 = 64;
/// Most pixels an image may have to be decoded and averaged (more: mid gray).
const MAX_AVERAGED_PIXELS: u64 = 1 << 24;

pub(super) const TYPE: &str = "type is written as its outlines in the editing data for other apps";
pub(super) const GRADIENTS: &str = "gradients are written as their average color in the editing data for other apps";
const PATTERNS: &str = "pattern fills are written as their tiles cut to the shape in the editing data for other apps";
pub(super) const CLIPPING: &str =
    "clipping groups and clipping layers are written as their art cut to the clipping path in the editing data for other apps";
pub(super) const IMAGES: &str = "images are written as their outlines filled with their average color in the editing data for other apps";
pub(super) const LIVE: &str =
    "symbols, blends, envelopes, meshes, repeats, placed documents and intertwines are written as their art in the editing data for other apps";
const STROKES: &str = "strokes a plain line can't draw (inside or outside the path, variable width, dots, dashes fitted to corners, gradients, patterns, arrowheads) are written as filled outlines, and brushed strokes as their brush's art, in the editing data for other apps";
pub(super) const TRANSPARENCY: &str = "transparency (opacity, blending modes, opacity masks) is left out of the editing data for other apps";
pub(super) const EFFECTS: &str = "live effects other than geometry effects are left out of the editing data for other apps";
const SWATCHES: &str = "gradient and pattern swatches are left out of the editing data for other apps";
pub(super) const SYMBOL_LIMIT: &str = "symbol instances past 16,000,000 objects of symbol art in all are left out of the editing data for other apps";
const TEMPLATES: &str = "template layers are written as dimmed layers that don't print in the editing data for other apps";

pub(super) struct Writer<'a> {
    doc: &'a Document,
    /// The document → art space.
    to_art: Affine,
    cms: Arc<Cms>,
    pub(super) body: String,
    pub(super) warnings: Vec<String>,
    /// The lock and hidden state (`A`, `Xw`) the last operators set; `None` at a layer's start.
    state: Option<(bool, bool)>,
    /// The region the art being written is cut to (a clipping path's, in document space).
    clip: Option<PathData>,
    /// Symbols, live objects and pattern tiles being written inside one another.
    nest: u32,
    /// The XML ids objects are named with so far.
    ids: HashSet<String>,
    /// The number the next object named after each XML id tries first.
    next: HashMap<String, u64>,
    /// The document's brushes, read the first time a brushed stroke is written.
    brushes: Option<Vec<vectorcraft_brush::Brush>>,
    /// The objects of the symbol art written so far, within [`vectorcraft_doc::MAX_SYMBOL_NODES`].
    symbol_nodes: u64,
}

impl<'a> Writer<'a> {
    pub(super) fn new(doc: &'a Document, to_art: Affine) -> Self {
        Self {
            doc,
            to_art,
            cms: vectorcraft_color::cms::active(),
            body: String::new(),
            warnings: vec![],
            state: None,
            clip: None,
            nest: 0,
            ids: HashSet::new(),
            next: HashMap::new(),
            brushes: None,
            symbol_nodes: 0,
        }
    }

    /// Whether the art of a symbol, `art`, may be written: the objects of the symbol art written
    /// so far stay within [`vectorcraft_doc::MAX_SYMBOL_NODES`], as in the PDF part (instances of
    /// symbols that hold instances multiply). Past it the art is left out, with a warning.
    fn take_symbol(&mut self, art: &Node) -> bool {
        let total = self.symbol_nodes.saturating_add(art.count() as u64);
        if total > vectorcraft_doc::MAX_SYMBOL_NODES {
            self.warn(SYMBOL_LIMIT);
            return false;
        }
        self.symbol_nodes = total;
        true
    }

    fn warn(&mut self, w: &str) {
        if !self.warnings.iter().any(|x| x == w) {
            self.warnings.push(w.to_string());
        }
    }

    fn line(&mut self, s: &str) {
        self.body.push_str(s);
        self.body.push('\n');
    }

    // ---------- layers and objects ----------

    /// A top-level layer. A layer that baking its effects made a group of the layer and the art an
    /// effect adds (Crop Marks) is written as that layer holding the added art too, and any other
    /// object at the top level in a layer of its own.
    pub(super) fn layer(&mut self, n: &Node) {
        if n.is_layer() {
            return self.write_layer(n, &[]);
        }
        let members = n.children().map(Vec::as_slice).unwrap_or_default();
        self.transparency(n);
        match members.iter().find(|c| c.is_layer()) {
            Some(l) => {
                // The group took the layer's name. The added art is outside the layer's clipping.
                let layer = Node { name: l.name.clone().or_else(|| n.name.clone()), ..(**l).clone() };
                let added: Vec<Arc<Node>> = members.iter().filter(|c| !c.is_layer()).cloned().collect();
                self.write_layer(&layer, &added);
            }
            None => {
                let mut layer = Node::layer(n.id, n.name.as_deref().unwrap_or("Layer"), LayerColor::Preset(0));
                (layer.visible, layer.locked) = (n.visible, n.locked);
                self.write_layer(&layer, &[Arc::new(n.clone())]);
            }
        }
    }

    /// A layer or sublayer: `%AI5_BeginLayer`, its options `Lb`, its name `Ln` and as an XML id, its art, then
    /// `after` (outside its clipping path when it clips), `LB`, `%AI5_EndLayer--`.
    fn write_layer(&mut self, n: &Node, after: &[Arc<Node>]) {
        let NodeKind::Layer { color, template, printable, children, clip, preview, dim_images } = &n.kind else { return };
        let [r, g, b] = match color {
            LayerColor::Preset(i) => LAYER_COLORS.get(usize::from(*i)).map_or([79, 128, 255], |c| c.1),
            LayerColor::Custom(rgb) => *rgb,
        };
        let index = match color {
            LayerColor::Preset(i) if usize::from(*i) < LAYER_COLORS.len() => i64::from(*i),
            _ => -1,
        };
        if *template {
            self.warn(TEMPLATES);
        }
        self.transparency(n);
        let flag = |b: bool| u8::from(b);
        let dim = dim_images.unwrap_or(50);
        let name = n.name.as_deref().unwrap_or("Layer");
        let _ = writeln!(
            self.body,
            "%AI5_BeginLayer\n{} {} {} {} {} 0 1 {index} {r} {g} {b} 0 {dim} 0 Lb\n{} Ln",
            flag(n.visible),
            flag(*preview),
            flag(!n.locked),
            flag(*printable && !*template),
            flag(dim_images.is_some() || *template),
            ps::bytes(name.as_bytes()),
        );
        self.xml_id(if name.is_empty() { "Layer" } else { name });
        self.state = None;
        if *clip {
            self.warn(CLIPPING);
            self.clip_contents(children);
        } else {
            for c in children {
                self.node(c);
            }
        }
        for c in after {
            self.node(c);
        }
        self.line("LB\n%AI5_EndLayer--");
        // The reader's state carries over into the art that follows.
        self.state = None;
    }

    /// Set the lock and hidden state of the object that follows.
    fn state(&mut self, locked: bool, hidden: bool) {
        let (was_locked, was_hidden) = match self.state {
            Some((l, h)) => (Some(l), Some(h)),
            None => (None, None),
        };
        if was_locked != Some(locked) {
            self.line(if locked { "1 A" } else { "0 A" });
        }
        if was_hidden != Some(hidden) {
            self.line(if hidden { "1 Xw" } else { "0 Xw" });
        }
        self.state = Some((locked, hidden));
    }

    /// Warn when `n` has transparency or effects the data leaves out.
    fn transparency(&mut self, n: &Node) {
        if n.opacity < 1.0 || n.blend != BlendMode::Normal || n.extras().mask.as_ref().is_some_and(|m| !m.disabled) {
            self.warn(TRANSPARENCY);
        }
        let items = n.appearance.items.iter().filter(|i| i.visible()).flat_map(|i| i.effects());
        if n.appearance.effects.iter().chain(items).any(|e| e.visible) {
            self.warn(EFFECTS);
        }
    }

    /// One object, its state before it and its name after it.
    fn node(&mut self, n: &Node) {
        if let NodeKind::Layer { .. } = n.kind {
            return self.write_layer(n, &[]);
        }
        self.transparency(n);
        self.state(n.locked, !n.visible);
        let start = self.body.len();
        match &n.kind {
            NodeKind::Layer { .. } => {}
            NodeKind::Group { children, clip } => {
                self.line("u");
                if *clip {
                    self.warn(CLIPPING);
                    self.clip_contents(children);
                } else {
                    for c in children {
                        self.node(c);
                    }
                }
                self.line("U");
            }
            NodeKind::Path { path, rule, guide, .. } => {
                if *guide {
                    ai_ops::guide(&mut self.body, self.to_art, path, true);
                } else {
                    self.shape(n, path, *rule);
                }
            }
            NodeKind::Compound { children, rule } => {
                let mut subpaths = vec![];
                for p in children.iter().filter_map(|c| c.path_data()) {
                    subpaths.extend(p.subpaths.iter().cloned());
                }
                self.shape(n, &PathData::new(subpaths), *rule);
            }
            NodeKind::Text(_) => self.text(n),
            NodeKind::Image(im) => {
                let bounds = PathData::from_bezpath(&(im.xf * Rect::new(0.0, 0.0, f64::from(im.width), f64::from(im.height)).to_path(0.1)));
                let color = image_average(self.doc.image_blob(im).map(|b| b.bytes.as_slice()));
                self.warn(IMAGES);
                self.fill_object(&bounds, FillRule::NonZero, &FillLayer::new(Paint::solid(color)));
            }
            NodeKind::SymbolInstance { symbol, xf } => {
                self.warn(LIVE);
                self.line("u");
                if let Some(sym) = self.doc.symbols.iter().find(|s| &s.name == symbol)
                    && self.nest < MAX_NEST
                    && self.take_symbol(&sym.art)
                {
                    // Stained by the instance's fill, as the canvas draws it.
                    let mut art = vectorcraft_brush::instance_art(&sym.art, n);
                    art.transform(*xf, false);
                    art.visible = true;
                    art.locked = false;
                    art.name = None;
                    self.nest += 1;
                    self.node(&art);
                    self.nest -= 1;
                }
                self.line("U");
            }
            NodeKind::Blend { .. }
            | NodeKind::Envelope { .. }
            | NodeKind::Mesh(_)
            | NodeKind::Repeat(_)
            | NodeKind::PlacedDocument(_)
            | NodeKind::Intertwine { .. } => {
                self.warn(LIVE);
                self.line("u");
                if self.nest < MAX_NEST {
                    let g = vectorcraft_effects::expand_live_deep(Some(self.doc), n);
                    self.nest += 1;
                    for c in g.children().map(Vec::as_slice).unwrap_or_default() {
                        self.node(c);
                    }
                    self.nest -= 1;
                }
                self.line("U");
            }
        }
        if self.body.len() > start {
            self.name(n);
        }
    }

    /// `n`'s name, after it (see [`Self::xml_id`]).
    fn name(&mut self, n: &Node) {
        let Some(name) = n.name.as_deref().filter(|s| !s.is_empty()) else { return };
        self.xml_id(name);
    }

    /// An `/ArtDictionary` whose `AI10_ArtUID` is `name` as an XML id, numbered (`_` and 20
    /// digits, `_`) when a layer or another object has that id.
    fn xml_id(&mut self, name: &str) {
        let base = vectorcraft_doc::xml_id::from_object_name(name);
        let mut id = base.clone();
        let next = self.next.entry(base.clone()).or_insert(0);
        while self.ids.contains(&id) {
            *next += 1;
            id = format!("{base}_{next:020}_");
        }
        self.ids.insert(id.clone());
        let _ = writeln!(self.body, "%_/ArtDictionary :\n%_/XMLUID : {} ; (AI10_ArtUID) ,\n%_;\n%_", ps::bytes(id.as_bytes()));
    }

    /// A container's members, the first one's outline cutting the others: its fills painted
    /// behind them (cut to the region outside too), its strokes over them, not cut by it.
    fn clip_contents(&mut self, children: &[Arc<Node>]) {
        let Some((first, rest)) = children.split_first() else { return };
        // Nothing to clip by hides the clipped art.
        let Some((outline, rule)) = vectorcraft_effects::clip_outline(first) else { return };
        let paint = first.clip_paint();
        let outer = self.clip.clone();
        let region = self.cut(&PathData::from_bezpath(&outline), rule);
        self.clip = Some(region);
        if let Some(fill) = &paint.fill {
            self.node(fill);
        }
        for c in rest {
            self.node(c);
        }
        self.clip = outer;
        if let Some(stroke) = &paint.stroke {
            self.node(stroke);
        }
    }

    /// `path` (filled by `rule`) cut to the region the art is cut to, as a non-zero path.
    fn cut(&self, path: &PathData, rule: FillRule) -> PathData {
        match &self.clip {
            Some(clip) => vectorcraft_pathops::boolean(path, rule, clip, FillRule::NonZero, BoolOp::Intersect),
            None => vectorcraft_pathops::boolean(path, rule, path, rule, BoolOp::Intersect),
        }
    }

    // ---------- paths ----------

    /// A path's fills and strokes: one object with one fill and a plain stroke over it where it
    /// can be, else a group of an object for each (in paint order).
    fn shape(&mut self, n: &Node, path: &PathData, rule: FillRule) {
        if path.is_empty() {
            return;
        }
        let items: Vec<&AppearanceItem> = n
            .appearance
            .items
            .iter()
            .filter(|i| match i {
                AppearanceItem::Fill(f) => f.visible && !f.paint.is_none(),
                AppearanceItem::Stroke(s) => s.visible && !s.paint.is_none() && s.width > 0.0 && s.width.is_finite(),
            })
            .collect();
        if items.iter().any(|i| i.opacity() < 1.0 || i.blend() != BlendMode::Normal) {
            self.warn(TRANSPARENCY);
        }
        let solid = |p: &Paint| matches!(p, Paint::Solid { .. });
        match items.as_slice() {
            [] => self.path_object(path, rule, None, None),
            [AppearanceItem::Fill(f)] if solid(&f.paint) => self.path_object(path, rule, Some(f), None),
            [AppearanceItem::Stroke(s)] if direct_stroke(s) => self.path_object(path, rule, None, Some(s)),
            [AppearanceItem::Fill(f), AppearanceItem::Stroke(s)] if solid(&f.paint) && direct_stroke(s) => {
                self.path_object(path, rule, Some(f), Some(s))
            }
            [one] => self.item(path, rule, one),
            items => {
                self.line("u");
                for i in items {
                    self.item(path, rule, i);
                }
                self.line("U");
            }
        }
    }

    /// One fill or stroke of a path as an object of its own.
    fn item(&mut self, path: &PathData, rule: FillRule, item: &AppearanceItem) {
        match item {
            AppearanceItem::Fill(f) => self.fill_object(path, rule, f),
            AppearanceItem::Stroke(s) if direct_stroke(s) => self.path_object(path, rule, None, Some(s)),
            AppearanceItem::Stroke(s) => {
                self.warn(STROKES);
                if self.brush_art(path, s) {
                    return;
                }
                let outline = stroke::outline_region(path, rule, s);
                let fill = FillLayer { overprint: s.overprint, ..FillLayer::new(s.paint.clone()) };
                self.fill_object(&outline, FillRule::NonZero, &fill);
            }
        }
    }

    /// A brushed stroke `st` of `path` as a group of its brush's art. False when it has no brush
    /// the document knows (or brushes nest too deep).
    fn brush_art(&mut self, path: &PathData, st: &StrokeLayer) -> bool {
        if self.nest >= MAX_NEST {
            return false;
        }
        let doc = self.doc;
        let brushes = self.brushes.get_or_insert_with(|| vectorcraft_brush::library(doc));
        let Some(b) = st.brush.as_deref().and_then(|name| brushes.iter().find(|b| b.name == name)) else { return false };
        let art = vectorcraft_brush::stroke_pieces(doc, b, &path.to_bezpath(), st);
        let state = self.state;
        self.nest += 1;
        self.line("u");
        for piece in &art {
            self.node(piece);
        }
        if let Some((locked, hidden)) = state {
            self.state(locked, hidden);
        }
        self.line("U");
        self.nest -= 1;
        true
    }

    /// A fill as an object of its own: a gradient as its average color, a pattern as its tiles
    /// cut to the path.
    fn fill_object(&mut self, path: &PathData, rule: FillRule, f: &FillLayer) {
        match &f.paint {
            Paint::Solid { .. } => self.path_object(path, rule, Some(f), None),
            Paint::Gradient(g) => {
                self.warn(GRADIENTS);
                let flat = FillLayer { paint: Paint::solid(average(&g.gradient)), ..f.clone() };
                self.path_object(path, rule, Some(&flat), None);
            }
            Paint::Pattern { pattern, xf } => {
                self.warn(PATTERNS);
                let doc = self.doc;
                let Some(def) = doc.pattern(pattern).filter(|_| self.nest < MAX_NEST) else {
                    let flat = FillLayer { paint: Paint::solid(Color::gray(0.5)), ..f.clone() };
                    return self.path_object(path, rule, Some(&flat), None);
                };
                let tiles = def.instances_in(*xf, path.to_bezpath().bounding_box());
                let outer = self.clip.clone();
                let region = self.cut(path, rule);
                self.clip = Some(region);
                self.nest += 1;
                let state = self.state;
                self.line("u");
                for t in &tiles {
                    self.node(t);
                }
                if let Some((locked, hidden)) = state {
                    self.state(locked, hidden);
                }
                self.line("U");
                self.nest -= 1;
                self.clip = outer;
            }
            Paint::None => {}
        }
    }

    /// A path object painted with `fill` (a color) and `stroke` (a plain line of a color) over
    /// it; several subpaths make a compound path. Where the art is cut to a clipping path, the
    /// fill is the path's region cut to it and the stroke its outline cut to it, filled.
    fn path_object(&mut self, path: &PathData, rule: FillRule, fill: Option<&FillLayer>, stroke: Option<&StrokeLayer>) {
        if self.clip.is_none() {
            return self.plain_object(path, rule, fill, stroke);
        }
        let both = fill.is_some() && stroke.is_some();
        if both {
            self.line("u");
        }
        if fill.is_some() || stroke.is_none() {
            let region = self.cut(path, rule);
            self.plain_object(&region, FillRule::NonZero, fill, None);
        }
        if let Some(s) = stroke {
            let outline = stroke::outline_region(path, rule, s);
            let region = self.cut(&outline, FillRule::NonZero);
            let fill = FillLayer { overprint: s.overprint, ..FillLayer::new(s.paint.clone()) };
            self.plain_object(&region, FillRule::NonZero, Some(&fill), None);
        }
        if both {
            self.line("U");
        }
    }

    /// [`Self::path_object`] without cutting: the paint (`O`/`R` overprint, line style), `XR`,
    /// each subpath's points and painting operator (`*u` … `*U` around several).
    fn plain_object(&mut self, path: &PathData, rule: FillRule, fill: Option<&FillLayer>, stroke: Option<&StrokeLayer>) {
        let subpaths: Vec<&SubPath> = path.subpaths.iter().filter(|sp| !sp.anchors.is_empty()).collect();
        if subpaths.is_empty() {
            return;
        }
        if let Some(f) = fill {
            self.color(&f.paint, false);
            self.line(if f.overprint { "1 O" } else { "0 O" });
        }
        if let Some(s) = stroke {
            self.color(&s.paint, true);
            self.line(if s.overprint { "1 R" } else { "0 R" });
            ai_ops::line_style(&mut self.body, s);
        }
        self.line(if rule == FillRule::EvenOdd { "1 XR" } else { "0 XR" });
        let op = match (fill.is_some(), stroke.is_some()) {
            (true, true) => 'b',
            (true, false) => 'f',
            (false, true) => 's',
            (false, false) => 'n',
        };
        let several = subpaths.len() > 1;
        if several {
            self.line("*u");
        }
        for sp in subpaths {
            ai_ops::subpath(&mut self.body, self.to_art, sp);
            let op = if sp.closed { op } else { op.to_ascii_uppercase() };
            self.body.push(op);
            self.body.push('\n');
        }
        if several {
            self.line("*U");
        }
    }

    // ---------- type ----------

    /// Type `n` as a group of what Type › Create Outlines makes of it
    /// ([`vectorcraft_effects::outline_text`]): each run's glyphs with their underline and
    /// strikethrough, its color glyphs, and the object's own fills and strokes; then its inline
    /// graphics as their symbols' art where the layout places them, as the PDF page draws them.
    fn text(&mut self, n: &Node) {
        self.warn(TYPE);
        self.line("u");
        let doc = self.doc;
        let resolved = match &n.kind {
            NodeKind::Text(t) => Some(doc.inline_resolved(t).into_owned()),
            _ => None,
        };
        let node = match &resolved {
            Some(t) => Node { kind: NodeKind::Text(Box::new(t.clone())), ..n.clone() },
            None => n.clone(),
        };
        let state = self.state;
        let outlined = vectorcraft_effects::outline_text(&node);
        let pieces = outlined.as_ref().and_then(|o| o.children()).map(Vec::as_slice).unwrap_or_default();
        // The object's fills and strokes above the Characters row, when they paint, are the last
        // piece: the inline graphics go under it, as the PDF page draws them.
        let above = !node.appearance.split_contents().1.is_empty() && {
            let bare = Node { appearance: Default::default(), ..node.clone() };
            let glyphs = vectorcraft_effects::outline_text(&bare).and_then(|o| o.children().map(Vec::len)).unwrap_or_default();
            pieces.len() > glyphs
        };
        let (under, over) = pieces.split_at(pieces.len() - usize::from(above));
        for c in under {
            self.node(c);
        }
        if let Some(t) = resolved.as_ref().filter(|_| self.nest < MAX_NEST) {
            let layout = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), t);
            for ig in &layout.inlines {
                let Some(art) = t.runs.get(ig.run).and_then(|r| r.inline.as_ref()) else { continue };
                let Some(sym) = doc.symbols.iter().find(|x| x.name == art.symbol) else { continue };
                if !self.take_symbol(&sym.art) {
                    continue;
                }
                self.warn(LIVE);
                let mut art_node = (*sym.art).clone();
                // Strokes scale with the art, as on the canvas.
                art_node.transform(t.xf * ig.xf * doc.symbol_natural_xf(&art.symbol), true);
                (art_node.visible, art_node.locked, art_node.name) = (true, false, None);
                self.nest += 1;
                self.node(&art_node);
                self.nest -= 1;
            }
        }
        for c in over {
            self.node(c);
        }
        if let Some((locked, hidden)) = state {
            self.state(locked, hidden);
        }
        self.line("U");
    }

    // ---------- color ----------

    /// Set the fill (or with `stroke`, the stroke) color to solid paint `p`: a spot or global
    /// swatch by its name, Registration, else the color itself.
    fn color(&mut self, p: &Paint, stroke: bool) {
        let Paint::Solid { color, swatch, tint } = p else {
            return self.plain_color(Color::gray(1.0), stroke);
        };
        if swatch.as_deref() == Some(REGISTRATION) {
            let mut s = String::from("1 1 1 1 ");
            s.push_str(&ps::bytes(REGISTRATION.as_bytes()));
            s.push(' ');
            ps::push_num(&mut s, f64::from(1.0 - unit(*tint)));
            s.push_str(if stroke { " XS" } else { " Xs" });
            return self.line(&s);
        }
        let doc = self.doc;
        if let Some(name) = swatch
            && let Some(sw) = doc.swatch(name).filter(|s| s.global || s.spot)
            && let Paint::Solid { color: full, swatch: None, .. } = sw.paint
        {
            let mut s = String::new();
            self.named(&mut s, name, full, *tint, sw.spot, stroke);
            return self.body.push_str(&s);
        }
        self.plain_color(*color, stroke);
    }

    /// A color that names no swatch: `g` (gray), `k` (CMYK), else `c m y k r g b Xa` (RGB, and
    /// Lab as RGB), with its CMYK equivalent.
    fn plain_color(&mut self, color: Color, stroke: bool) {
        let s = &mut self.body;
        let op = |fill: &'static str, line: &'static str| if stroke { line } else { fill };
        match color {
            // Gray is ink coverage; the format's gray is lightness.
            Color::Gray { k } => {
                ps::push_num(s, f64::from(1.0 - unit(k)));
                let _ = writeln!(s, " {}", op("g", "G"));
            }
            Color::Cmyk { c, m, y, k } => {
                ps::push_nums(s, &[c, m, y, k].map(|v| f64::from(unit(v))));
                let _ = writeln!(s, "{}", op("k", "K"));
            }
            Color::Rgb { .. } | Color::Lab { .. } => {
                let cmyk = self.cms.to_cmyk(&color, self.cms.settings().intent).map(unit);
                ps::push_nums(s, &cmyk.map(f64::from));
                ps::push_nums(s, &color.to_rgb().map(|v| f64::from(unit(v))));
                let _ = writeln!(s, "{}", op("Xa", "XA"));
            }
        }
    }

    /// A spot (`spot`) or global process color `name` of full color `full` at `tint`, written to
    /// `s`: its CMYK equivalent, its RGB or Lab values, the name, how much lighter than the full
    /// color, and for any but a CMYK spot color (`x`) the model (0 CMYK, 1 RGB, 2 Lab) before
    /// `Xx` (spot) or `Xk` (global process).
    fn named(&self, s: &mut String, name: &str, full: Color, tint: f32, spot: bool, stroke: bool) {
        let cmyk = self.cms.to_cmyk(&self.doc.linked_color(full, spot), self.cms.settings().intent).map(unit);
        ps::push_nums(s, &cmyk.map(f64::from));
        let model = match full {
            Color::Rgb { r, g, b } => {
                ps::push_nums(s, &[r, g, b].map(|v| f64::from(unit(v))));
                Some(1)
            }
            Color::Lab { l, a, b } => {
                ps::push_nums(s, &[f64::from(l), f64::from(a), f64::from(b)]);
                Some(2)
            }
            _ if spot => None,
            _ => Some(0),
        };
        s.push_str(&ps::bytes(name.as_bytes()));
        s.push(' ');
        ps::push_num(s, f64::from(1.0 - unit(tint)));
        let op = match (model, spot) {
            (None, _) => "x",
            (Some(_), true) => "Xx",
            (Some(_), false) => "Xk",
        };
        if let Some(m) = model {
            let _ = write!(s, " {m}");
        }
        let op = if stroke { op.to_ascii_uppercase() } else { op.to_string() };
        let _ = writeln!(s, " {op}");
    }

    // ---------- swatches ----------

    /// The swatches: `%AI5_BeginPalette`, `0 0 Pb`, the Registration swatch, each color swatch's color and `(name) Pc`
    /// (the ungrouped ones first, then each color group's after `0 (name) count Pg`), `PB`,
    /// `%AI5_EndPalette`.
    pub(super) fn palette(&mut self, s: &mut String) {
        s.push_str("%AI5_BeginPalette\n0 0 Pb\n");
        self.registration_cell(s);
        let doc = self.doc;
        for sw in doc.swatches.iter() {
            self.swatch(s, sw);
        }
        for g in doc.swatch_groups.iter() {
            let colors: Vec<&Swatch> = g.swatches.iter().filter(|sw| writable(sw)).collect();
            let _ = writeln!(s, "0 {} {} Pg", ps::bytes(g.name.as_bytes()), colors.len());
            for sw in &g.swatches {
                self.swatch(s, sw);
            }
        }
        s.push_str("PB\n%AI5_EndPalette\n");
    }

    /// The Registration swatch, which every palette in the files users sent starts with: in a
    /// CMYK document `1 1 1 1 ([Registration]) 0 Xs`, in an RGB one black's CMYK equivalent,
    /// `0 0 0 ([Registration]) 0 1 Xz`; then `([Registration]) Pc`.
    fn registration_cell(&self, s: &mut String) {
        let name = ps::bytes(REGISTRATION.as_bytes());
        if self.doc.color_mode == vectorcraft_doc::ColorMode::Cmyk {
            let _ = writeln!(s, "1 1 1 1 {name} 0 Xs");
        } else {
            let black = Color::rgb(0.0, 0.0, 0.0);
            let cmyk = self.cms.to_cmyk(&black, self.cms.settings().intent).map(unit);
            ps::push_nums(s, &cmyk.map(f64::from));
            let _ = writeln!(s, "0 0 0 {name} 0 1 Xz");
        }
        let _ = writeln!(s, "{name} Pc");
    }

    /// One swatch of the palette: a color swatch's color and `(name) Pc`; gradient and pattern
    /// swatches are left out.
    fn swatch(&mut self, s: &mut String, sw: &Swatch) {
        if sw.name == REGISTRATION || matches!(sw.paint, Paint::None) {
            return;
        }
        if !writable(sw) {
            return self.warn(SWATCHES);
        }
        let Paint::Solid { color, swatch, .. } = &sw.paint else { return };
        if swatch.is_none() && (sw.global || sw.spot) {
            self.named(s, &sw.name, *color, 1.0, sw.spot, false);
        } else {
            let body = std::mem::take(&mut self.body);
            self.plain_color(*color, false);
            s.push_str(&std::mem::replace(&mut self.body, body));
        }
        let _ = writeln!(s, "{} Pc", ps::bytes(sw.name.as_bytes()));
    }

    /// The spot color swatches, with their full colors' CMYK equivalents, for the header.
    pub(super) fn spot_swatches(&self) -> Vec<(String, [f32; 4])> {
        let intent = self.cms.settings().intent;
        self.doc
            .swatches_iter()
            .filter(|sw| sw.spot)
            .filter_map(|sw| match sw.paint {
                Paint::Solid { color, swatch: None, .. } => {
                    Some((sw.name.clone(), self.cms.to_cmyk(&self.doc.linked_color(color, true), intent).map(unit)))
                }
                _ => None,
            })
            .collect()
    }
}

/// Is `sw` a color swatch the palette writes?
fn writable(sw: &Swatch) -> bool {
    matches!(sw.paint, Paint::Solid { .. }) && sw.name != REGISTRATION
}

/// Does a plain line of one color draw `s` (the line style operators can write it)?
fn direct_stroke(s: &StrokeLayer) -> bool {
    matches!(s.paint, Paint::Solid { .. }) && stroke::is_plain(s)
}

/// The image `bytes` decoded, when its header gives at most [`MAX_AVERAGED_PIXELS`] pixels; the
/// decoder may allocate at most 8 bytes a pixel.
fn decode_bounded(bytes: &[u8]) -> Option<image::DynamicImage> {
    let reader = || -> image::ImageResult<image::ImageReader<std::io::Cursor<&[u8]>>> {
        let mut r = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format()?;
        let mut limits = image::Limits::default();
        limits.max_alloc = Some(MAX_AVERAGED_PIXELS * 8);
        r.limits(limits);
        Ok(r)
    };
    let (w, h) = reader().and_then(|r| r.into_dimensions()).ok()?;
    if u64::from(w) * u64::from(h) > MAX_AVERAGED_PIXELS {
        return None;
    }
    reader().and_then(|r| r.decode()).ok()
}

/// The average color of the image `bytes` (scaled down first), transparent pixels over white;
/// mid gray when it can't be decoded or has more than [`MAX_AVERAGED_PIXELS`] pixels.
fn image_average(bytes: Option<&[u8]>) -> Color {
    let Some(img) = bytes.and_then(decode_bounded) else { return Color::gray(0.5) };
    let img = if img.width() > AVERAGE_SIDE || img.height() > AVERAGE_SIDE { img.thumbnail(AVERAGE_SIDE, AVERAGE_SIDE) } else { img };
    let rgba = img.to_rgba8();
    let mut sum = [0u64; 3];
    let mut n = 0u64;
    for px in rgba.pixels() {
        let [r, g, b, a] = px.0;
        let over = |c: u8| (u64::from(c) * u64::from(a) + 255 * (255 - u64::from(a))) / 255;
        sum[0] += over(r);
        sum[1] += over(g);
        sum[2] += over(b);
        n += 1;
    }
    if n == 0 {
        return Color::gray(0.5);
    }
    let c = |v: u64| (v as f32 / n as f32 / 255.0).clamp(0.0, 1.0);
    Color::rgb(c(sum[0]), c(sum[1]), c(sum[2]))
}
