//! Drawing items to a document: the scale and position the options ask for, one layer per DXF
//! layer that holds art (or one in all), named blocks as symbols (anonymous ones, such as
//! dimensions, as groups), and colours, lineweights, linetypes and transparency resolved through
//! layers and blocks.

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use kurbo::BezPath;
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, AppearanceItem, CharStyle, Dash, Document, LAYER_COLORS, LayerColor, LineCap, Node, NodeKind, Symbol, TextObject};
use vectorcraft_geom::{Affine, FillRule, PathData, Point, Rect};

use super::ImportOptions;
use super::entity::{Alpha, Col, Converter, Geom, InsertItem, Item, Lw, Props, Span, TextItem};
use super::reader::{Drawing, LayoutDef};
use crate::{CAP_HEIGHT, MAX_NEST};

/// Points per millimetre.
const PT_PER_MM: f64 = 72.0 / 25.4;
/// The stroke weight of lineweight 0 (CAD apps draw it one device pixel wide), in points.
const THINNEST: f64 = 0.25;
/// The most objects an import makes.
const MAX_NODES: usize = 2_000_000;
/// How far from the origin art may lie (points): the canvas.
pub(crate) const MAX_EXTENT: f64 = 4.0e6;

const NESTED: &str = "blocks nested more than 8 deep, or inside themselves, are left out";
const XREFS: &str = "blocks referencing other drawings (external references) are left out";
const TOO_MANY: &str = "the drawing has more than 2,000,000 objects: the rest are left out";

/// What a block's art takes from the insert that places it: its colour, lineweight, linetype
/// and opacity where the art says "by block", and its layer for art on layer 0.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
struct Inherit {
    color: Option<[u8; 3]>,
    lineweight: Option<i32>,
    linetype: Option<String>,
    alpha: Option<u8>,
    layer: Option<String>,
}

pub(crate) struct Build<'d, 'a> {
    d: &'d Drawing<'a>,
    o: &'d ImportOptions,
    conv: Converter,
    doc: Document,
    /// Block items by upper-case name.
    blocks: HashMap<String, Rc<Vec<Item>>>,
    bounds: HashMap<String, Option<Rect>>,
    sensitive: HashMap<String, bool>,
    /// Symbol names by block and what it inherits (`None`: the block has no art).
    symbols: HashMap<(String, Inherit), Option<String>>,
    /// Blocks being drawn, outermost first.
    stack: Vec<String>,
    /// Document space per drawing unit, and its symbols' (no translation).
    map: Affine,
    symbol_map: Affine,
    /// Lineweights are multiplied by this.
    lw_scale: f64,
    nodes: usize,
    warnings: Vec<String>,
}

/// The unit of `$INSUNITS` code `code` in points (0, unitless: none).
pub(crate) fn insunits_points(code: i64) -> Option<f64> {
    let m = 72.0 / 0.0254;
    Some(match code {
        1 => 72.0,
        2 => 864.0,
        3 => 72.0 * 63_360.0,
        4 => PT_PER_MM,
        5 => PT_PER_MM * 10.0,
        6 => m,
        7 => m * 1e3,
        8 => 72e-6,
        9 => 72e-3,
        10 => 2592.0,
        11 => m * 1e-10,
        12 => m * 1e-9,
        13 => m * 1e-6,
        14 => m * 0.1,
        15 => m * 10.0,
        16 => m * 100.0,
        17 => m * 1e9,
        18 => m * 1.495_978_707e11,
        19 => m * 9.460_730_472_580_8e15,
        20 => m * 3.085_677_581e16,
        21 => m * 1200.0 / 3937.0,
        _ => return None,
    })
}

impl<'d, 'a> Build<'d, 'a> {
    pub fn new(d: &'d Drawing<'a>, o: &'d ImportOptions) -> Self {
        Self {
            d,
            o,
            conv: Converter::default(),
            doc: Document::new(1.0, 1.0),
            blocks: HashMap::new(),
            bounds: HashMap::new(),
            sensitive: HashMap::new(),
            symbols: HashMap::new(),
            stack: vec![],
            map: Affine::IDENTITY,
            symbol_map: Affine::IDENTITY,
            lw_scale: 1.0,
            nodes: 0,
            warnings: vec![],
        }
    }

    fn warn(&mut self, w: &str) {
        if !self.warnings.iter().any(|x| x == w) {
            self.warnings.push(w.to_string());
        }
    }

    /// Import `layout`; `natural` and `ratio` are points per drawing unit in the drawing's own
    /// unit and at the scale asked for (fitting computes its own).
    pub fn run(mut self, layout: &LayoutDef, natural: f64, ratio: f64) -> Result<(Document, Vec<String>), String> {
        let items = self.conv.items(&self.d.layout_records(layout));
        let bounds = items.iter().fold(None, |acc, it| vectorcraft_geom::union_opt(acc, self.item_bounds(it, 0)));
        let (k, size, offset) = self.placement(bounds, natural, ratio)?;
        self.map = Affine::new([k, 0.0, 0.0, -k, offset.x, offset.y]);
        self.symbol_map = Affine::new([k, 0.0, 0.0, -k, 0.0, 0.0]);
        self.lw_scale = if self.o.scale_lineweights { k / natural } else { 1.0 };
        let layers = self.layers(&items);
        let Self { mut doc, conv, warnings: notes, .. } = self;
        if !layers.is_empty() {
            doc.layers = layers;
        }
        if let Some(a) = doc.artboards.first_mut() {
            a.rect = Rect::from_origin_size(Point::ZERO, size);
        }
        doc.fix_next_id();
        let mut warnings = conv.warnings;
        warnings.extend(notes);
        if !conv.skipped.is_empty() {
            let list: Vec<String> = conv.skipped.iter().map(|(k, n)| format!("{n} {k}")).collect();
            warnings.push(format!("entities of these types are left out: {}", list.join(", ")));
        }
        if bounds.is_none() {
            warnings.push(format!("the {} layout has no art", layout.name));
        }
        Ok((doc, warnings))
    }

    /// Points per drawing unit, the artboard's size, and where the drawing's origin goes.
    fn placement(&self, bounds: Option<Rect>, natural: f64, ratio: f64) -> Result<(f64, vectorcraft_geom::Size, Point), String> {
        let b = bounds.unwrap_or(Rect::ZERO);
        let k = if self.o.fit {
            let (w, h) = self.fit_box(b);
            let ratio = |a: f64, b: f64| (b > 1e-12).then(|| a / b);
            [ratio(w, b.width()), ratio(h, b.height())].into_iter().flatten().reduce(f64::min).unwrap_or(natural)
        } else {
            ratio
        };
        if !(k.is_finite() && k > 0.0) {
            return Err("the scale is not a usable number".into());
        }
        let size = if self.o.fit {
            let (w, h) = self.fit_box(b);
            vectorcraft_geom::Size::new(w, h)
        } else {
            vectorcraft_geom::Size::new((b.width() * k).max(1.0), (b.height() * k).max(1.0))
        };
        // Centred: the art's centre on the artboard's; else the drawing's origin (fitted art: its
        // bottom-left corner) on the artboard's bottom-left corner.
        let offset = if self.o.center {
            Point::new(size.width / 2.0 - b.center().x * k, size.height / 2.0 + b.center().y * k)
        } else if self.o.fit {
            Point::new(-b.x0 * k, size.height + b.y0 * k)
        } else {
            Point::new(0.0, size.height)
        };
        let corners = [Point::new(b.x0, b.y0), Point::new(b.x1, b.y1)].map(|p| Point::new(p.x * k + offset.x, -p.y * k + offset.y));
        let fits = corners.iter().all(|p| p.x.abs() <= MAX_EXTENT && p.y.abs() <= MAX_EXTENT) && size.width.max(size.height) <= MAX_EXTENT;
        if !fits {
            let (w, h) = (b.width() * k, b.height() * k);
            return Err(format!(
                "at this scale the drawing is {w:.0} × {h:.0} pt, more than the canvas holds: choose a smaller scale or fit it to the artboard"
            ));
        }
        Ok((k, size, offset))
    }

    /// The box Fit to Artboard fills, turned to the art's orientation.
    fn fit_box(&self, b: Rect) -> (f64, f64) {
        let (w, h) = self.o.fit_to;
        if (b.width() > b.height()) == (w > h) { (w, h) } else { (h, w) }
    }

    // ---------- blocks ----------

    /// A block's items (read once).
    fn block(&mut self, name: &str) -> Option<Rc<Vec<Item>>> {
        if let Some(b) = self.blocks.get(name) {
            return Some(b.clone());
        }
        let def = self.d.blocks.get(name)?;
        if def.xref {
            self.warn(XREFS);
            return None;
        }
        let items = Rc::new(self.conv.items(&def.records));
        self.blocks.insert(name.to_string(), items.clone());
        Some(items)
    }

    /// Enter block `name` `depth` deep; false (with a warning) when too deep or inside itself.
    fn enter(&mut self, name: &str, depth: u32) -> bool {
        if depth >= MAX_NEST || self.stack.iter().any(|s| s == name) {
            self.warn(NESTED);
            return false;
        }
        self.stack.push(name.to_string());
        true
    }

    /// A block's bounds in its coordinates less its base point.
    fn block_bounds(&mut self, name: &str, depth: u32) -> Option<Rect> {
        if let Some(b) = self.bounds.get(name) {
            return *b;
        }
        let items = self.block(name)?;
        if !self.enter(name, depth) {
            return None;
        }
        let b = items.iter().fold(None, |acc, it| vectorcraft_geom::union_opt(acc, self.item_bounds(it, depth + 1)));
        self.stack.pop();
        let base = self.d.blocks.get(name).map_or(Point::ZERO, |b| b.base);
        let b = b.map(|r| r - base.to_vec2());
        self.bounds.insert(name.to_string(), b);
        b
    }

    /// An item's bounds in world coordinates (type: an estimate from its height and length).
    fn item_bounds(&mut self, it: &Item, depth: u32) -> Option<Rect> {
        match &it.geom {
            Geom::Path { bp, .. } => super::curve::bounds(bp),
            Geom::Text(t) => {
                let lines = t.text.lines().count().max(1) as f64;
                let longest = t.text.lines().map(|l| l.chars().count()).max().unwrap_or(0) as f64;
                let w = longest * t.height * 0.9 * t.width;
                let x0 = match t.justify {
                    vectorcraft_doc::Justify::Center => -w / 2.0,
                    vectorcraft_doc::Justify::Right => -w,
                    _ => 0.0,
                };
                let below = (lines - 1.0) * t.leading.unwrap_or(0.0) + t.height * 0.3;
                Some(t.xf.transform_rect_bbox(Rect::new(x0, -below, x0 + w, t.height)))
            }
            Geom::Insert(ins) => {
                let b = self.block_bounds(&ins.block, depth);
                let cells = b.into_iter().flat_map(|b| ins.cells.iter().map(move |m| m.transform_rect_bbox(b)));
                let attribs: Vec<Option<Rect>> = ins.attribs.iter().map(|a| self.item_bounds(a, depth)).collect();
                cells.map(Some).chain(attribs).fold(None, vectorcraft_geom::union_opt)
            }
        }
        .filter(|r| [r.x0, r.y0, r.x1, r.y1].iter().all(|v| v.is_finite()))
    }

    /// Does a block's look depend on its insert (art "by block", or on layer 0 "by layer")?
    fn is_sensitive(&mut self, name: &str, depth: u32) -> bool {
        if let Some(s) = self.sensitive.get(name) {
            return *s;
        }
        let Some(items) = self.block(name) else { return false };
        if !self.enter(name, depth) {
            return false;
        }
        let mut s = false;
        for it in items.iter() {
            let p = &it.props;
            let on_zero = p.layer == "0";
            s |= p.color == Col::ByBlock || p.lineweight == Lw::ByBlock || p.linetype == "BYBLOCK" || p.alpha == Alpha::ByBlock;
            s |= on_zero && (p.color == Col::ByLayer || p.lineweight == Lw::ByLayer || p.linetype == "BYLAYER");
            if let Geom::Insert(ins) = &it.geom {
                s |= !ins.attribs.is_empty() || self.is_sensitive(&ins.block, depth + 1);
            }
            if s {
                break;
            }
        }
        self.stack.pop();
        self.sensitive.insert(name.to_string(), s);
        s
    }

    // ---------- properties ----------

    /// The layer art on layer 0 takes from its block, else its own.
    fn layer_name<'p>(&self, p: &'p Props, inh: &'p Inherit) -> &'p str {
        match &inh.layer {
            Some(l) if p.layer == "0" => l,
            _ => &p.layer,
        }
    }

    fn rgb(&self, p: &Props, inh: &Inherit) -> [u8; 3] {
        match p.color {
            Col::Rgb(c) => c,
            Col::ByBlock => inh.color.unwrap_or([0; 3]),
            Col::ByLayer => self.d.layer(self.layer_name(p, inh)).map_or([0; 3], |l| l.rgb),
        }
    }

    /// The lineweight in hundredths of a millimetre.
    fn lineweight(&self, p: &Props, inh: &Inherit) -> i32 {
        let default = self.d.header.lwdefault;
        match p.lineweight {
            Lw::Hundredths(v) => v,
            Lw::Default => default,
            Lw::ByBlock => inh.lineweight.unwrap_or(default),
            Lw::ByLayer => self.d.layer(self.layer_name(p, inh)).map(|l| l.lineweight).filter(|v| *v >= 0).unwrap_or(default),
        }
    }

    /// The linetype name (upper case) after "by layer" and "by block".
    fn linetype(&self, p: &Props, inh: &Inherit) -> String {
        match p.linetype.as_str() {
            "BYLAYER" => self.d.layer(self.layer_name(p, inh)).map_or_else(|| "CONTINUOUS".into(), |l| l.linetype.clone()),
            "BYBLOCK" => inh.linetype.clone().unwrap_or_else(|| "CONTINUOUS".into()),
            other => other.to_string(),
        }
    }

    fn opacity(&self, p: &Props, inh: &Inherit) -> f32 {
        match p.alpha {
            Alpha::Opacity(a) => a,
            Alpha::ByBlock => inh.alpha.map_or(1.0, |a| f32::from(a) / 255.0),
            Alpha::ByLayer => 1.0,
        }
    }

    /// What a block placed by an insert with `p` inherits.
    fn inherit(&self, p: &Props, inh: &Inherit) -> Inherit {
        Inherit {
            color: Some(self.rgb(p, inh)),
            lineweight: Some(self.lineweight(p, inh)),
            linetype: Some(self.linetype(p, inh)),
            alpha: Some((self.opacity(p, inh) * 255.0).round() as u8),
            layer: Some(self.layer_name(p, inh).to_string()),
        }
    }

    /// A dash pattern in points for `linetype` drawn `scale` points per drawing unit.
    fn dash(&self, linetype: &str, p: &Props, scale: f64) -> Option<(Dash, bool)> {
        let pattern = self.d.linetypes.get(linetype)?;
        let k = self.d.header.ltscale * p.ltscale * scale;
        // Dashes (positive), gaps (negative) and dots (0), merged into dash, gap, dash, gap…
        let mut out: Vec<f64> = vec![];
        for v in pattern {
            let dash = *v >= 0.0;
            let len = v.abs() * k;
            match out.len().is_multiple_of(2) {
                expects_dash if expects_dash == dash => out.push(len),
                _ if out.is_empty() => out.extend([0.0, len]),
                _ => {
                    if let Some(last) = out.last_mut() {
                        *last += len;
                    }
                }
            }
        }
        if !out.len().is_multiple_of(2) {
            out.push(0.0);
        }
        let dots = out.iter().step_by(2).any(|d| *d <= 1e-9);
        let d = Dash { pattern: out, offset: 0.0, align_corners: false };
        (d.is_dashed() && d.pattern.iter().skip(1).step_by(2).any(|g| *g > 1e-9) && d.pattern.iter().all(|v| v.is_finite())).then_some((d, dots))
    }

    // ---------- art ----------

    /// One doc layer per DXF layer holding art (in the order art first uses them), or one for all.
    fn layers(&mut self, items: &[Item]) -> Vec<Arc<Node>> {
        let mut layers: Vec<(String, Vec<Arc<Node>>)> = vec![];
        let mut index: HashMap<String, usize> = HashMap::new();
        let top = Inherit::default();
        let map = self.map;
        for it in items {
            let Some(node) = self.item(it, map, &top, 0) else { continue };
            let name = if self.o.merge_layers { "" } else { it.props.layer.as_str() };
            let i = *index.entry(name.to_uppercase()).or_insert_with(|| {
                layers.push((name.to_string(), vec![]));
                layers.len() - 1
            });
            if let Some((_, children)) = layers.get_mut(i) {
                children.push(Arc::new(node));
            }
        }
        layers
            .into_iter()
            .enumerate()
            .map(|(i, (name, children))| {
                let id = self.doc.alloc_id();
                let color = LayerColor::Preset((i % LAYER_COLORS.len()) as u8);
                let label = if name.is_empty() { "Layer 1" } else { &name };
                let mut l = Node::layer(id, label, color);
                if let Some(def) = self.d.layer(&name) {
                    l.visible = !def.hidden;
                    l.locked = def.locked;
                    if let NodeKind::Layer { printable, .. } = &mut l.kind {
                        *printable = def.plot;
                    }
                }
                if let Some(c) = l.children_mut() {
                    *c = children;
                }
                Arc::new(l)
            })
            .collect()
    }

    /// One item as an object, drawn by `map` (world coordinates to its parent's space).
    fn item(&mut self, it: &Item, map: Affine, inh: &Inherit, depth: u32) -> Option<Node> {
        if self.nodes >= MAX_NODES {
            self.warn(TOO_MANY);
            return None;
        }
        let mut node = match &it.geom {
            Geom::Path { bp, fill, width } => self.path(bp, *fill, *width, &it.props, map, inh)?,
            Geom::Text(t) => self.text(t, &it.props, map, inh)?,
            Geom::Insert(ins) => return self.insert(ins, &it.props, map, inh, depth),
        };
        node.opacity = self.opacity(&it.props, inh);
        self.nodes += 1;
        Some(node)
    }

    fn path(&mut self, bp: &BezPath, fill: bool, width: f64, p: &Props, map: Affine, inh: &Inherit) -> Option<Node> {
        let mut bp = bp.clone();
        bp.apply_affine(map);
        super::curve::bounds(&bp)?;
        let paint = Paint::solid(rgb_color(self.rgb(p, inh)));
        let scale = map.determinant().abs().sqrt();
        let appearance = if fill {
            Appearance::basic(paint, Paint::None, 0.0)
        } else {
            let lw = self.lineweight(p, inh);
            let w = if width > 0.0 {
                width * scale
            } else if lw == 0 {
                THINNEST
            } else {
                f64::from(lw) / 100.0 * PT_PER_MM * self.lw_scale
            };
            let mut ap = Appearance::basic(Paint::None, paint, w);
            if let Some(AppearanceItem::Stroke(s)) = ap.items.last_mut()
                && let Some((dash, dots)) = self.dash(&self.linetype(p, inh), p, scale)
            {
                s.dash = Some(dash);
                if dots {
                    s.cap = LineCap::Round;
                }
            }
            ap
        };
        let rule = if fill { FillRule::EvenOdd } else { FillRule::NonZero };
        let path = PathData::from_bezpath(&bp);
        let kind = |path: PathData| NodeKind::Path { path, rule, live: None, clipping: false, guide: false };
        let mut n = if path.subpaths.len() > 1 {
            // Several loops (a hatch with islands) paint as one compound path.
            let children = path
                .subpaths
                .into_iter()
                .map(|sp| {
                    let mut c = Node::new(self.doc.alloc_id(), kind(PathData::single(sp)));
                    c.appearance = appearance.clone();
                    Arc::new(c)
                })
                .collect();
            Node::new(self.doc.alloc_id(), NodeKind::Compound { children, rule })
        } else {
            Node::new(self.doc.alloc_id(), kind(path))
        };
        n.appearance = appearance;
        Some(n)
    }

    fn text(&mut self, t: &TextItem, p: &Props, map: Affine, inh: &Inherit) -> Option<Node> {
        // Text space is y-down points; the item's is y-up drawing units.
        let m = map * t.xf * Affine::FLIP_Y;
        let s = m.determinant().abs().sqrt();
        if !(s > 1e-12 && s.is_finite()) {
            return None;
        }
        let family = t.family.clone().or_else(|| self.d.styles.get(&t.style).filter(|f| !f.is_empty()).cloned());
        let default = CharStyle::default();
        let style = CharStyle {
            font_family: family.unwrap_or(default.font_family.clone()),
            size: t.height / CAP_HEIGHT * s,
            h_scale: t.width * 100.0,
            leading: t.leading.map(|l| l * s),
            fill: Paint::solid(rgb_color(self.rgb(p, inh))),
            ..default
        };
        let mut obj = TextObject::point(Point::ZERO, &t.text, style);
        if let Some(span) = t.span {
            reach(&mut obj, span, s);
        }
        obj.xf = m * Affine::scale(1.0 / s);
        obj.para.justify = t.justify;
        Some(Node::new(self.doc.alloc_id(), NodeKind::Text(Box::new(obj))))
    }

    /// An insert: a symbol instance per copy (anonymous blocks: a group of their art), grouped
    /// with its attributes.
    fn insert(&mut self, ins: &InsertItem, p: &Props, map: Affine, inh: &Inherit, depth: u32) -> Option<Node> {
        let child = self.inherit(p, inh);
        let base = self.d.blocks.get(&ins.block).map_or(Point::ZERO, |b| b.base);
        let unbase = Affine::translate(-base.to_vec2());
        let anonymous = ins.block.starts_with('*');
        let symbol = if anonymous { None } else { self.symbol(&ins.block, &child, depth) };
        let mut parts = vec![];
        for cell in &ins.cells {
            let node = match &symbol {
                Some(name) => {
                    let xf = map * *cell * self.symbol_map.inverse();
                    self.nodes += 1;
                    Some(Node::new(self.doc.alloc_id(), NodeKind::SymbolInstance { symbol: name.clone(), xf }))
                }
                None if anonymous => self.art(&ins.block, map * *cell * unbase, &child, depth),
                None => None,
            };
            parts.extend(node);
        }
        for a in &ins.attribs {
            parts.extend(self.item(a, map, &child, depth));
        }
        let mut node = match parts.len() {
            0 => return None,
            1 => parts.pop()?,
            _ => Node::group(self.doc.alloc_id(), parts.into_iter().map(Arc::new).collect()),
        };
        node.opacity = self.opacity(p, inh);
        Some(node)
    }

    /// The art of block `name` drawn by `map`, as one object.
    fn art(&mut self, name: &str, map: Affine, inh: &Inherit, depth: u32) -> Option<Node> {
        let items = self.block(name)?;
        if !self.enter(name, depth) {
            return None;
        }
        let mut nodes = vec![];
        for it in items.iter() {
            // Art on a hidden layer stays hidden inside blocks.
            if self.d.layer(self.layer_name(&it.props, inh)).is_some_and(|l| l.hidden) && it.props.layer != "0" {
                continue;
            }
            nodes.extend(self.item(it, map, inh, depth + 1));
        }
        self.stack.pop();
        match nodes.len() {
            0 => None,
            1 => nodes.pop(),
            _ => Some(Node::group(self.doc.alloc_id(), nodes.into_iter().map(Arc::new).collect())),
        }
    }

    /// The symbol for block `name` placed with `inh` (made once per look).
    fn symbol(&mut self, name: &str, inh: &Inherit, depth: u32) -> Option<String> {
        let inh = if self.is_sensitive(name, depth) { inh.clone() } else { Inherit::default() };
        let key = (name.to_string(), inh);
        if let Some(s) = self.symbols.get(&key) {
            return s.clone();
        }
        let base = self.d.blocks.get(name).map_or(Point::ZERO, |b| b.base);
        let map = self.symbol_map * Affine::translate(-base.to_vec2());
        let art = self.art(name, map, &key.1, depth);
        let made = art.map(|art| {
            let label = self.d.blocks.get(name).map_or(name, |b| b.name.as_str());
            let symbol = unique(label, |n| self.doc.symbols.iter().any(|s| s.name == n));
            self.doc.symbols.push(Symbol { name: symbol.clone(), art: Arc::new(art) });
            symbol
        });
        self.symbols.insert(key, made.clone());
        made
    }
}

fn rgb_color([r, g, b]: [u8; 3]) -> Color {
    Color::rgb8(r, g, b)
}

/// Make Fit or Aligned text `t` (in points, `s` per drawing unit) reach its second point: its
/// line measured in its font, Fit text is stretched or squeezed to the length (its horizontal
/// scale), Aligned text scaled as a whole (its size).
fn reach(t: &mut TextObject, span: Span, s: f64) {
    let (Span::Fit(length) | Span::Aligned(length)) = span;
    let target = length * s;
    let layout = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), t);
    let Some(width) = layout.lines.first().map(|l| l.x1 - l.x0) else { return };
    if !(width > 1e-9 && target > 1e-9 && target.is_finite()) {
        return;
    }
    let k = target / width;
    for run in &mut t.runs {
        match span {
            Span::Fit(_) => run.style.h_scale *= k,
            Span::Aligned(_) => run.style.size *= k,
        }
    }
}

/// `base`, or `base 2`, `base 3`… when taken.
fn unique(base: &str, taken: impl Fn(&str) -> bool) -> String {
    if !taken(base) {
        return base.to_string();
    }
    (2..).map(|i| format!("{base} {i}")).find(|n| !taken(n)).unwrap_or_else(|| base.to_string())
}
