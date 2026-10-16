//! The drawing VectorCraft imports: pages, layers and paths in y-down points, with transforms
//! already baked in. One FreeHand point is one document point (1/72 inch).

use std::collections::BTreeMap;

use crate::Error;
use crate::container::{self, Limits};
use crate::records::{ColorRec, FillRec, GradNote, IDENTITY, Matrix, Node3, Prop, RawFile, Rec, ShapeRec, StrokeRec};

/// Red, green, blue and alpha, each 0…1.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rgba {
    pub r: f64,
    pub g: f64,
    pub b: f64,
    pub a: f64,
}

/// One gradient stop.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Stop {
    pub offset: f64,
    pub color: Rgba,
}

/// A fill already resolved to a colour or a gradient.
#[derive(Debug, Clone, PartialEq)]
pub enum Fill {
    None,
    Solid(Rgba),
    /// `angle` is degrees for a y-down page: 0 points to +x, positive angles point toward the top.
    Linear {
        angle: f64,
        stops: Vec<Stop>,
    },
    /// Centre as a fraction of the shape bounds, `cx` from the left and `cy` from the top.
    Radial {
        cx: f64,
        cy: f64,
        stops: Vec<Stop>,
    },
}

/// A solid stroke. Caps and joins are not stored in the records this reader trusts, so callers
/// use butt caps and a miter join.
#[derive(Debug, Clone, PartialEq)]
pub struct Stroke {
    pub color: Rgba,
    pub width: f64,
    pub dash: Vec<f64>,
}

/// One cubic segment. The three points are control 1, control 2 and the end, in absolute points.
#[derive(Debug, Clone, PartialEq)]
pub struct Subpath {
    pub start: [f64; 2],
    pub cubics: Vec<[[f64; 2]; 3]>,
    pub closed: bool,
}

/// A path with its paint. A compound is the same shape with more than one subpath.
#[derive(Debug, Clone, PartialEq)]
pub struct Path {
    pub subpaths: Vec<Subpath>,
    pub even_odd: bool,
    pub fill: Fill,
    pub stroke: Option<Stroke>,
}

/// One run of characters that share a font, a size and a colour.
#[derive(Debug, Clone, PartialEq)]
pub struct TextRun {
    pub text: String,
    pub font: String,
    pub size: f64,
    pub color: Rgba,
}

/// A text object. `x` and `y` are the top-left of the box in y-down points.
#[derive(Debug, Clone, PartialEq)]
pub struct Text {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub runs: Vec<TextRun>,
}

/// One object on a layer, back to front within its parent.
#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    Group(Vec<Item>),
    ClipGroup { clip: Path, children: Vec<Item> },
    Compound(Path),
    Path(Path),
    Text(Text),
}

/// A layer. Visibility 3 is shown. Any other value is kept, with its objects, and marked hidden.
/// The file does not document a lock bit, so `locked` stays false.
#[derive(Debug, Clone, PartialEq)]
pub struct Layer {
    pub name: String,
    pub visible: bool,
    pub locked: bool,
    pub items: Vec<Item>,
}

/// One page, already placed in y-down document points. Pages keep their relative positions.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Page {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// A FreeHand drawing converted for a y-down editor.
#[derive(Debug, Clone, PartialEq)]
pub struct Document {
    pub pages: Vec<Page>,
    pub layers: Vec<Layer>,
    pub warnings: Vec<String>,
}

const KAPPA: f64 = 0.5522847498307936;
const DEPTH: u8 = 64;

/// Read a FreeHand drawing. `limits` caps the sizes an untrusted file may ask for.
pub fn read(bytes: &[u8], limits: Limits) -> Result<Document, Error> {
    let raw = container::open(bytes, &limits)?;
    let _ = raw.version;
    Ok(build(&raw, &limits))
}

fn build(raw: &RawFile, limits: &Limits) -> Document {
    let mut b = Builder {
        raw,
        depth_cap: u8::try_from(limits.max_depth).unwrap_or(DEPTH).min(DEPTH),
        notes: BTreeMap::new(),
        fill_name: find_str(raw, "fill"),
        stroke_name: find_str(raw, "stroke"),
        contents_name: find_str(raw, "contents"),
        stack: Vec::new(),
    };
    let (pages, matrix) = b.pages();
    let mut layers = Vec::new();
    for id in b.list(b.block_layers()) {
        if let Some(layer) = b.layer(id, matrix) {
            layers.push(layer);
        }
    }
    let warnings = b.notes.into_iter().map(|(k, n)| format!("{k} ({n})")).collect();
    Document { pages, layers, warnings }
}

fn find_str(raw: &RawFile, text: &str) -> u32 {
    raw.recs.iter().position(|r| matches!(r, Rec::Str(s) if s == text)).and_then(|i| u32::try_from(i + 1).ok()).unwrap_or(0)
}

struct Builder<'a> {
    raw: &'a RawFile,
    depth_cap: u8,
    notes: BTreeMap<String, usize>,
    fill_name: u32,
    stroke_name: u32,
    contents_name: u32,
    stack: Vec<u32>,
}

impl Builder<'_> {
    fn note(&mut self, msg: &str) {
        *self.notes.entry(msg.to_string()).or_insert(0) += 1;
    }

    fn block_layers(&self) -> u32 {
        if let Some(Rec::Block { layers }) = self.raw.get(self.raw.block_id) {
            return *layers;
        }
        self.raw
            .recs
            .iter()
            .find_map(|r| match r {
                Rec::Block { layers } => Some(*layers),
                _ => None,
            })
            .unwrap_or(0)
    }

    fn list(&self, id: u32) -> Vec<u32> {
        match self.raw.get(id) {
            Some(Rec::List { ids, .. }) => ids.clone(),
            _ => Vec::new(),
        }
    }

    fn pages(&mut self) -> (Vec<Page>, Matrix) {
        let found = self.page_rects();
        let rects = if found.is_empty() { self.fallback_page() } else { found };
        let origin_x = rects.iter().map(|r| r[0]).fold(f64::INFINITY, f64::min);
        let origin_top = rects.iter().map(|r| r[1] + r[3]).fold(f64::NEG_INFINITY, f64::max);
        if !origin_x.is_finite() || !origin_top.is_finite() {
            self.note("no page size was stored; a US Letter page was used");
            return (vec![Page { x: 0.0, y: 0.0, width: 612.0, height: 792.0 }], [1.0, 0.0, 0.0, 0.0, -1.0, 792.0]);
        }
        let pages = rects
            .iter()
            .filter(|r| r[2].is_finite() && r[3].is_finite() && r[2] > 0.0 && r[3] > 0.0)
            .map(|r| Page { x: r[0] - origin_x, y: origin_top - (r[1] + r[3]), width: r[2], height: r[3] })
            .collect::<Vec<_>>();
        let matrix = [1.0, 0.0, -origin_x, 0.0, -1.0, origin_top];
        if pages.is_empty() {
            self.note("no page size was stored; a US Letter page was used");
            (vec![Page { x: 0.0, y: 0.0, width: 612.0, height: 792.0 }], [1.0, 0.0, 0.0, 0.0, -1.0, 792.0])
        } else {
            (pages, matrix)
        }
    }

    /// Page rectangles in FreeHand y-up points: min x, min y, width, height.
    fn page_rects(&self) -> Vec<[f64; 4]> {
        let Some(Rec::Pairs { pairs, .. }) = self.raw.get(self.raw.props_id) else { return Vec::new() };
        let mut out = Vec::new();
        for (name, value) in pairs {
            if self.raw.text(*name) != "pages" {
                continue;
            }
            for id in self.list(*value) {
                if let Some(Rec::Props(props)) = self.raw.get(id)
                    && let Some(r) = page_rect(props)
                {
                    out.push(r);
                }
            }
        }
        out
    }

    fn fallback_page(&mut self) -> Vec<[f64; 4]> {
        if let Some((w, h)) = self.raw.fallback {
            self.note("the page size came from the file trailer, not a page record");
            vec![[0.0, 0.0, w, h]]
        } else {
            self.note("no page size was stored; a US Letter page was used");
            vec![[0.0, 0.0, 612.0, 792.0]]
        }
    }

    fn layer(&mut self, id: u32, matrix: Matrix) -> Option<Layer> {
        let Rec::Layer { elements, name, visibility } = self.raw.get(id)? else { return None };
        let (elements, name, visibility) = (*elements, *name, *visibility);
        let visible = visibility == 3;
        let items = self.children(elements, matrix);
        let label = self.raw.text(name);
        Some(Layer { name: if label.is_empty() { "Layer".into() } else { label.to_string() }, visible, locked: false, items })
    }

    fn children(&mut self, list: u32, matrix: Matrix) -> Vec<Item> {
        let ids = self.list(list);
        let mut out = Vec::new();
        for id in ids {
            out.extend(self.one(id, matrix));
        }
        out
    }

    fn one(&mut self, id: u32, matrix: Matrix) -> Vec<Item> {
        if id == 0 {
            return Vec::new();
        }
        if self.stack.len() >= usize::from(self.depth_cap) || self.stack.contains(&id) {
            self.note("cyclic or deeply nested objects were left out");
            return Vec::new();
        }
        self.stack.push(id);
        let items = self.dispatch(id, matrix);
        self.stack.pop();
        items
    }

    fn dispatch(&mut self, id: u32, matrix: Matrix) -> Vec<Item> {
        match self.step(id) {
            Step::Group { elements, xform, clip } => {
                let next = mul(matrix, self.xform(xform));
                let kids = self.children(elements, next);
                if clip {
                    self.clip(kids)
                } else {
                    // An empty group is still a group. FreeHand stores some with a child list
                    // whose count is zero; dropping them flattened the layer.
                    vec![Item::Group(kids)]
                }
            }
            Step::Composite { style, elements } => self.compound(style, elements, matrix).into_iter().collect(),
            Step::Path { style, closed, even_odd } => self.path_item(id, style, closed, even_odd, matrix).into_iter().collect(),
            Step::Shape(shape) => self.shape_item(&shape, matrix).into_iter().collect(),
            Step::Symbol { class, matrix: local } => self.symbol(class, local, matrix),
            Step::Blend(lists) => {
                self.note("blend steps were drawn as stored, not regenerated");
                let mut out = Vec::new();
                for list in lists {
                    out.extend(self.children(list, matrix));
                }
                out
            }
            Step::Text(text) => self.text_item(&text, matrix).into_iter().collect(),
            Step::Image => {
                self.note("placed images were left out");
                Vec::new()
            }
            Step::Guides => {
                self.note("guide objects were left out");
                Vec::new()
            }
            Step::Other(label) => {
                self.note(&format!("unsupported objects ({label}) were left out"));
                Vec::new()
            }
            Step::Missing => Vec::new(),
        }
    }

    fn step(&self, id: u32) -> Step {
        match self.raw.get(id) {
            Some(Rec::Group { style, elements, xform, clip, .. }) => {
                let _ = style;
                Step::Group { elements: *elements, xform: *xform, clip: *clip }
            }
            Some(Rec::Composite { style, elements, .. }) => Step::Composite { style: *style, elements: *elements },
            Some(Rec::Path { style, closed, even_odd, .. }) => Step::Path { style: *style, closed: *closed, even_odd: *even_odd },
            Some(Rec::Shape(shape)) => Step::Shape(shape.clone()),
            Some(Rec::SymbolInstance { class, matrix }) => Step::Symbol { class: *class, matrix: *matrix },
            Some(Rec::Blend { lists }) => Step::Blend(*lists),
            Some(Rec::Text(text)) => Step::Text(text.clone()),
            Some(Rec::Image) => Step::Image,
            Some(_) if self.raw.name(id) == "Guides" => Step::Guides,
            Some(_) => {
                let n = self.raw.name(id);
                Step::Other(if n.is_empty() { "unknown".into() } else { n.to_string() })
            }
            None => Step::Missing,
        }
    }

    fn clip(&mut self, mut kids: Vec<Item>) -> Vec<Item> {
        if !matches!(kids.first(), Some(Item::Path(_))) {
            return vec![Item::Group(kids)];
        }
        let Item::Path(clip) = kids.remove(0) else { return vec![Item::Group(kids)] };
        if kids.is_empty() { vec![Item::Path(clip)] } else { vec![Item::ClipGroup { clip, children: kids }] }
    }

    fn compound(&mut self, style: u32, list: u32, matrix: Matrix) -> Option<Item> {
        let ids = self.list(list);
        let mut subpaths = Vec::new();
        let mut even = false;
        let mut paint_style = style;
        let mut used_child = false;
        for id in ids {
            let Some(Rec::Path { style: child, closed, even_odd, nodes, .. }) = self.raw.get(id) else {
                self.note("a compound path contained something other than a path");
                continue;
            };
            if !used_child && *child != 0 {
                paint_style = *child;
                used_child = true;
            }
            even |= *even_odd;
            if let Some(sub) = subpath(nodes, *closed, matrix) {
                subpaths.push(sub);
            }
        }
        if subpaths.is_empty() {
            return None;
        }
        let (fill, stroke, contents) = self.paint(paint_style);
        let path = Path { subpaths, even_odd: even, fill, stroke };
        Some(self.with_content(path, true, contents, matrix))
    }

    fn path_item(&mut self, id: u32, style: u32, closed: bool, even_odd: bool, matrix: Matrix) -> Option<Item> {
        let sub = {
            let Rec::Path { nodes, .. } = self.raw.get(id)? else { return None };
            subpath(nodes, closed, matrix)?
        };
        let (fill, stroke, contents) = self.paint(style);
        let path = Path { subpaths: vec![sub], even_odd, fill, stroke };
        Some(self.with_content(path, false, contents, matrix))
    }

    fn shape_item(&mut self, shape: &ShapeRec, matrix: Matrix) -> Option<Item> {
        let (style, local, sub, even_odd) = match shape {
            ShapeRec::Rect { style, xform, b, rx, ry, .. } => (*style, self.xform(*xform), rect_sub(*b, *rx, *ry)?, false),
            ShapeRec::Oval { style, xform, b, arc, .. } => (*style, self.xform(*xform), oval_sub(*b, *arc)?, false),
            ShapeRec::Poly { style, xform, n, even_odd, cx, cy, r1, r2, a1, a2, .. } => {
                let sub = poly_sub(*n, *cx, *cy, *r1, *r2, *a1, *a2)?;
                (*style, self.xform(*xform), sub, *even_odd)
            }
        };
        let m = mul(matrix, local);
        let sub = map_sub(sub, m);
        if sub.cubics.is_empty() {
            return None;
        }
        let (fill, stroke, contents) = self.paint(style);
        let path = Path { subpaths: vec![sub], even_odd, fill, stroke };
        Some(self.with_content(path, false, contents, matrix))
    }

    /// A non-zero `contents` id is artwork used as the fill. The shape clips that art. A stroke
    /// on the shape is drawn on top, because the clip path itself is not painted. Art that does
    /// not resolve keeps the shape and a warning.
    fn with_content(&mut self, path: Path, compound: bool, content: u32, matrix: Matrix) -> Item {
        let plain = |path: Path| if compound { Item::Compound(path) } else { Item::Path(path) };
        if content == 0 {
            return plain(path);
        }
        let art = self.one(content, matrix);
        if !has_drawable(&art) {
            self.note("content fills were left unfilled");
            return plain(path);
        }
        let mut clip_path = path.clone();
        clip_path.fill = Fill::None;
        clip_path.stroke = None;
        let clipped = Item::ClipGroup { clip: clip_path, children: art };
        if path.stroke.is_some() {
            let mut outline = path;
            outline.fill = Fill::None;
            Item::Group(vec![clipped, plain(outline)])
        } else {
            clipped
        }
    }

    fn symbol(&mut self, class: u32, local: Matrix, matrix: Matrix) -> Vec<Item> {
        let Some(Rec::SymbolClass { group }) = self.raw.get(class) else {
            self.note("a symbol instance had no art");
            return Vec::new();
        };
        let group = *group;
        let local = sane(local, self);
        self.one(group, mul(matrix, local))
    }

    fn text_item(&mut self, text: &crate::records::TextRec, matrix: Matrix) -> Option<Item> {
        let m = mul(matrix, self.xform(text.xform));
        let left = prop_num(&text.props, 0x134C).unwrap_or(0.0);
        let top = prop_num(&text.props, 0x13DC).unwrap_or(0.0);
        let width = prop_num(&text.props, 0x140C).unwrap_or(0.0).abs();
        let height = prop_num(&text.props, 0x131C).unwrap_or(0.0).abs();
        let origin = apply(m, [left, top]);
        let w = apply(m, [left + width, top]);
        let h = apply(m, [left, top + height]);
        let runs = self.runs(text.text, text.attrs);
        if runs.iter().all(|r| r.text.is_empty()) {
            return None;
        }
        Some(Item::Text(Text { x: origin[0], y: origin[1], width: dist(origin, w), height: dist(origin, h), runs }))
    }

    fn runs(&mut self, story: u32, defaults: u32) -> Vec<TextRun> {
        let Some(Rec::Story { paras }) = self.raw.get(story) else { return Vec::new() };
        let paras = paras.clone();
        let mut out: Vec<TextRun> = Vec::new();
        for (i, para) in paras.iter().enumerate() {
            if i > 0
                && let Some(last) = out.last_mut()
            {
                last.text.push('\n');
            }
            let Some(Rec::Paragraph(p)) = self.raw.get(*para) else { continue };
            let block = self.raw.text(p.block).to_string();
            let mut runs = p.runs.clone();
            runs.sort_by_key(|(start, _)| *start);
            if runs.is_empty() {
                out.push(self.run(&block, defaults));
                continue;
            }
            let units = block.encode_utf16().count();
            for (n, (start, style)) in runs.iter().enumerate() {
                let from = usize::from(*start).min(units);
                let to = runs.get(n + 1).map(|(s, _)| usize::from(*s)).unwrap_or(units).min(units);
                let text = slice_units(&block, from, to);
                let mut run = self.run("", if *style == 0 { defaults } else { *style });
                if run.font.is_empty() {
                    run = self.run("", defaults);
                }
                run.text = text;
                out.push(run);
            }
        }
        out
    }

    fn run(&mut self, text: &str, style: u32) -> TextRun {
        let props = match self.raw.get(style) {
            Some(Rec::Props(p)) => p.clone(),
            _ => Vec::new(),
        };
        let mut font = prop_text(self.raw, &props, 0x1739);
        let mut size = prop_num(&props, 0x1734);
        if let Some(font_id) = prop_ref(&props, 0x16C1)
            && let Some(Rec::Props(font_props)) = self.raw.get(font_id)
        {
            if font.is_empty() {
                font = prop_text(self.raw, font_props, 0x0E11);
            }
            if font.is_empty() {
                font = prop_text(self.raw, font_props, 0x0E39);
            }
            if size.is_none() {
                size = prop_num(font_props, 0x0E24);
            }
        }
        let color = prop_ref(&props, 0x16B9).and_then(|id| self.rgba(id, 0)).unwrap_or(Rgba { r: 0.0, g: 0.0, b: 0.0, a: 1.0 });
        TextRun {
            text: text.to_string(),
            font: if font.is_empty() { "Times".into() } else { font },
            size: size.filter(|s| s.is_finite() && *s > 0.0).unwrap_or(12.0),
            color,
        }
    }

    fn xform(&mut self, id: u32) -> Matrix {
        match self.raw.get(id) {
            Some(Rec::Xform(m)) => sane(*m, self),
            _ => IDENTITY,
        }
    }

    fn paint(&mut self, style: u32) -> (Fill, Option<Stroke>, u32) {
        let (fill_id, stroke_id, contents) = self.styles(style);
        (self.fill_of(fill_id), self.stroke_of(stroke_id), contents)
    }

    fn styles(&mut self, style: u32) -> (u32, u32, u32) {
        let mut chain = Vec::new();
        let mut id = style;
        while id != 0 && chain.len() < 32 && !chain.contains(&id) {
            chain.push(id);
            id = parent_of(self.raw, id);
        }
        chain.reverse();
        let mut fill = 0u32;
        let mut stroke = 0u32;
        let mut contents = 0u32;
        for id in chain {
            self.apply_style(id, &mut fill, &mut stroke, &mut contents);
        }
        (fill, stroke, contents)
    }

    fn apply_style(&mut self, id: u32, fill: &mut u32, stroke: &mut u32, contents: &mut u32) {
        self.apply_style_depth(id, fill, stroke, contents, 0);
    }

    fn apply_style_depth(&mut self, id: u32, fill: &mut u32, stroke: &mut u32, contents: &mut u32, depth: u8) {
        if depth > 32 {
            self.note("cyclic or deeply nested objects were left out");
            return;
        }
        let picked = self.peek_style(id);
        if let Some(attrs) = picked.attrs {
            self.attributes(attrs, fill, stroke, contents, depth);
        }
        if let Some(v) = picked.fill {
            *fill = v;
        }
        if let Some(v) = picked.stroke {
            *stroke = v;
        }
        if let Some(v) = picked.contents
            && v != 0
        {
            *contents = v;
        }
        if let Some(style) = picked.filter {
            self.note("filters on styles were left off");
            self.apply_style_depth(style, fill, stroke, contents, depth + 1);
        }
    }

    /// Fill, stroke and content-fill ids on one style record. Read while the record is borrowed,
    /// then applied after the borrow ends.
    fn peek_style(&self, id: u32) -> StylePick {
        let mut picked = StylePick::default();
        match self.raw.get(id) {
            Some(Rec::Pairs { pairs, .. }) => self.scan_pairs(pairs, &mut picked),
            Some(Rec::Graphic { attrs, pairs, .. }) => {
                picked.attrs = Some(*attrs);
                self.scan_pairs(pairs, &mut picked);
            }
            Some(Rec::FilterStyle { style }) => picked.filter = Some(*style),
            _ => {}
        }
        picked
    }

    fn scan_pairs(&self, pairs: &[(u32, u32)], picked: &mut StylePick) {
        for (name, value) in pairs {
            if *name == self.fill_name {
                picked.fill = Some(*value);
            } else if *name == self.stroke_name {
                picked.stroke = Some(*value);
            } else if *name == self.contents_name && *value != 0 {
                picked.contents = Some(*value);
            }
        }
    }

    fn attributes(&mut self, list: u32, fill: &mut u32, stroke: &mut u32, contents: &mut u32, depth: u8) {
        if depth > 32 {
            self.note("cyclic or deeply nested objects were left out");
            return;
        }
        for id in self.list(list) {
            let Some(value) = holder_value(self.raw, id, 0) else { continue };
            match self.raw.get(value) {
                Some(Rec::Fill(_)) => *fill = value,
                Some(Rec::Stroke(_)) => *stroke = value,
                Some(Rec::Graphic { .. } | Rec::Pairs { .. } | Rec::FilterStyle { .. }) => {
                    self.apply_style_depth(value, fill, stroke, contents, depth + 1);
                }
                _ => {}
            }
        }
    }

    fn fill_of(&mut self, id: u32) -> Fill {
        let Some(Rec::Fill(fill)) = self.raw.get(id) else { return Fill::None };
        match fill {
            FillRec::Solid(c) => self.rgba(*c, 0).map(Fill::Solid).unwrap_or(Fill::None),
            FillRec::PostScript(c) => {
                self.note("PostScript paints were drawn as a solid colour");
                self.rgba(*c, 0).map(Fill::Solid).unwrap_or(Fill::None)
            }
            FillRec::Pattern(c) => {
                self.note("pattern fills were drawn as their solid colour");
                self.rgba(*c, 0).map(Fill::Solid).unwrap_or(Fill::None)
            }
            FillRec::Tile => {
                self.note("tile fills were left unfilled");
                Fill::None
            }
            FillRec::Custom { color, .. } => {
                self.note("procedure paints were drawn as a solid colour");
                self.rgba(*color, 0).map(Fill::Solid).unwrap_or(Fill::None)
            }
            FillRec::Lens { color, value, mode } => self.lens(*color, *value, *mode),
            FillRec::Linear { c1, c2, angle, stops, note } => {
                self.grad_note(*note, false);
                let stops = self.stops(*stops, *c1, *c2);
                if stops.is_empty() { Fill::None } else { Fill::Linear { angle: gradient_angle(*angle), stops } }
            }
            FillRec::Radial { c1, c2, cx, cy, stops, note } => {
                self.grad_note(*note, true);
                let stops = self.stops(*stops, *c1, *c2);
                if stops.is_empty() { Fill::None } else { Fill::Radial { cx: *cx, cy: 1.0 - *cy, stops } }
            }
        }
    }

    fn grad_note(&mut self, note: GradNote, radial: bool) {
        match note {
            GradNote::None => {}
            GradNote::Tapered => self.note("tapered gradients were drawn as linear gradients"),
            GradNote::Cone => self.note("conic gradients were drawn as linear gradients"),
            GradNote::Contour if radial => self.note("contour gradients were drawn as radial gradients"),
            GradNote::Contour => self.note("contour gradients were drawn as linear gradients"),
        }
    }

    fn lens(&mut self, color: u32, value: f64, mode: u8) -> Fill {
        self.note("lens fills were approximated");
        let pct = if value.is_finite() { (value / 100.0).clamp(0.0, 1.0) } else { 0.0 };
        let ink = self.rgba(color, 0).unwrap_or(Rgba { r: 0.0, g: 0.0, b: 0.0, a: 1.0 });
        let rgba = match mode {
            0 => Rgba { a: (1.0 - pct).clamp(0.0, 1.0), ..ink },
            1 | 4 => return Fill::None,
            2 => Rgba { r: 1.0, g: 1.0, b: 1.0, a: pct },
            3 => Rgba { r: 0.0, g: 0.0, b: 0.0, a: pct },
            _ => {
                let y = 0.3 * ink.r + 0.59 * ink.g + 0.11 * ink.b;
                Rgba { r: y, g: y, b: y, a: 1.0 }
            }
        };
        Fill::Solid(rgba)
    }

    fn stops(&mut self, id: u32, c1: u32, c2: u32) -> Vec<Stop> {
        if let Some(Rec::Stops(list)) = self.raw.get(id) {
            let mut out = Vec::new();
            for (color, pos) in list {
                if let Some(color) = self.rgba(*color, 0)
                    && pos.is_finite()
                {
                    out.push(Stop { offset: pos.clamp(0.0, 1.0), color });
                }
            }
            if !out.is_empty() {
                return out;
            }
        }
        let mut out = Vec::new();
        if let Some(color) = self.rgba(c1, 0) {
            out.push(Stop { offset: 0.0, color });
        }
        if let Some(color) = self.rgba(c2, 0) {
            out.push(Stop { offset: 1.0, color });
        }
        out
    }

    fn stroke_of(&mut self, id: u32) -> Option<Stroke> {
        let Rec::Stroke(stroke) = self.raw.get(id)? else {
            if let Some(Rec::Fill(FillRec::Custom { color, width })) = self.raw.get(id) {
                self.note("procedure paints were drawn as a solid colour");
                let color = self.rgba(*color, 0)?;
                return Some(Stroke { color, width: width.unwrap_or(1.0), dash: Vec::new() });
            }
            return None;
        };
        let stroke = stroke.clone();
        self.stroke_rec(&stroke)
    }

    fn stroke_rec(&mut self, stroke: &StrokeRec) -> Option<Stroke> {
        if stroke.arrows {
            self.note("arrowheads were left off");
        }
        if stroke.patterned {
            self.note("pattern strokes were drawn solid");
        }
        if stroke.postscript {
            self.note("PostScript paints were drawn as a solid colour");
        }
        let color = self.rgba(stroke.color, 0)?;
        let width = if stroke.width.is_finite() { stroke.width.max(0.0) } else { 1.0 };
        let dash = match self.raw.get(stroke.dashes) {
            Some(Rec::Dashes(v)) if v.len() >= 2 && v.iter().all(|n| n.is_finite() && *n >= 0.0) => v.clone(),
            _ => Vec::new(),
        };
        Some(Stroke { color, width, dash })
    }

    fn rgba(&self, id: u32, depth: u8) -> Option<Rgba> {
        if depth > 8 {
            return None;
        }
        match self.raw.get(id)? {
            Rec::Color(ColorRec::Rgb(r, g, b)) => Some(rgba(*r, *g, *b, 1.0)),
            Rec::Color(ColorRec::Tint { base, tint }) => {
                let base = self.rgba(*base, depth + 1)?;
                let t = f64::from(*tint) / 65536.0;
                Some(rgba(base.r * t + (1.0 - t), base.g * t + (1.0 - t), base.b * t + (1.0 - t), 1.0))
            }
            Rec::Fill(FillRec::Solid(c) | FillRec::PostScript(c) | FillRec::Pattern(c)) => self.rgba(*c, depth + 1),
            _ => None,
        }
    }
}

/// FreeHand's angle is counterclockwise from +x in y-up space. After the page flip that is the
/// same angle [`vectorcraft_color::GradientGeom::fit`] uses (0 = +x, positive toward the top).
fn gradient_angle(fh_deg: f64) -> f64 {
    if fh_deg.is_finite() { fh_deg } else { 0.0 }
}

fn rgba(r: f64, g: f64, b: f64, a: f64) -> Rgba {
    let c = |v: f64| if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 };
    Rgba { r: c(r), g: c(g), b: c(b), a: c(a) }
}

fn page_rect(props: &[Prop]) -> Option<[f64; 4]> {
    let mut min_x = None;
    let mut min_y = None;
    let mut width = None;
    let mut height = None;
    for p in props {
        let Some(v) = prop_value(p) else { continue };
        match p.id {
            0x1C24 | 0x1C7C => min_x = Some(v),
            0x1C2C | 0x1C84 => min_y = Some(v),
            0x1C34 => width = Some(v),
            0x1C3C => height = Some(v),
            _ => {}
        }
    }
    let (x, y, w, h) = (min_x?, min_y?, width?, height?);
    (w > 0.0 && h > 0.0 && w < 1.0e7 && h < 1.0e7).then_some([x, y, w, h])
}

fn prop_value(p: &Prop) -> Option<f64> {
    if p.kind == 2 {
        return None;
    }
    let v = f64::from(p.num as i32) / 65536.0;
    v.is_finite().then_some(v)
}

fn prop_num(props: &[Prop], id: u16) -> Option<f64> {
    props.iter().rev().find(|p| p.id == id).and_then(prop_value)
}

fn prop_ref(props: &[Prop], id: u16) -> Option<u32> {
    props.iter().rev().find(|p| p.id == id && p.kind == 2).map(|p| p.num)
}

fn prop_text(raw: &RawFile, props: &[Prop], id: u16) -> String {
    prop_ref(props, id).map(|n| raw.text(n).to_string()).unwrap_or_default()
}

fn parent_of(raw: &RawFile, id: u32) -> u32 {
    match raw.get(id) {
        Some(Rec::Pairs { parent, .. } | Rec::Graphic { parent, .. }) => *parent,
        _ => 0,
    }
}

fn holder_value(raw: &RawFile, id: u32, depth: u8) -> Option<u32> {
    if depth > 32 {
        return None;
    }
    match raw.get(id)? {
        Rec::Holder { value, .. } if *value != 0 => Some(*value),
        Rec::Holder { parent, .. } => holder_value(raw, *parent, depth + 1),
        Rec::FilterStyle { style } => Some(*style),
        _ => None,
    }
}

fn sane(m: Matrix, b: &mut Builder<'_>) -> Matrix {
    if m.iter().all(|v| v.is_finite()) {
        m
    } else {
        b.note("a non-finite transform was ignored");
        IDENTITY
    }
}

fn mul(a: Matrix, b: Matrix) -> Matrix {
    [
        a[0] * b[0] + a[1] * b[3],
        a[0] * b[1] + a[1] * b[4],
        a[0] * b[2] + a[1] * b[5] + a[2],
        a[3] * b[0] + a[4] * b[3],
        a[3] * b[1] + a[4] * b[4],
        a[3] * b[2] + a[4] * b[5] + a[5],
    ]
}

fn apply(m: Matrix, p: [f64; 2]) -> [f64; 2] {
    [m[0] * p[0] + m[1] * p[1] + m[2], m[3] * p[0] + m[4] * p[1] + m[5]]
}

fn dist(a: [f64; 2], b: [f64; 2]) -> f64 {
    let d = ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt();
    if d.is_finite() { d } else { 0.0 }
}

fn subpath(nodes: &[Node3], closed: bool, matrix: Matrix) -> Option<Subpath> {
    let first = nodes.first()?;
    if !finite_pt(first.a) {
        return None;
    }
    let start = first.a;
    let cin = first.cin;
    let mut cubics = Vec::new();
    for pair in nodes.windows(2) {
        let Some(a) = pair.first() else { continue };
        let Some(b) = pair.get(1) else { continue };
        push_cubic(&mut cubics, a.cout, b.cin, b.a);
    }
    if closed && nodes.len() >= 2 {
        let last = nodes.last()?;
        push_cubic(&mut cubics, last.cout, cin, start);
    }
    if cubics.is_empty() && !closed {
        return None;
    }
    Some(map_sub(Subpath { start, cubics, closed: closed && nodes.len() >= 2 }, matrix))
}

fn push_cubic(out: &mut Vec<[[f64; 2]; 3]>, c1: [f64; 2], c2: [f64; 2], end: [f64; 2]) {
    if finite_pt(c1) && finite_pt(c2) && finite_pt(end) {
        out.push([c1, c2, end]);
    }
}

fn finite_pt(p: [f64; 2]) -> bool {
    p[0].is_finite() && p[1].is_finite()
}

fn map_sub(sub: Subpath, m: Matrix) -> Subpath {
    Subpath {
        start: apply(m, sub.start),
        cubics: sub.cubics.into_iter().map(|[c1, c2, e]| [apply(m, c1), apply(m, c2), apply(m, e)]).collect(),
        closed: sub.closed,
    }
}

fn line_to(end: [f64; 2]) -> [[f64; 2]; 3] {
    [end, end, end]
}

fn rect_sub(b: [f64; 4], rx: f64, ry: f64) -> Option<Subpath> {
    if !b.iter().all(|v| v.is_finite()) {
        return None;
    }
    let (x0, x1) = ord(b[0], b[2]);
    let (y0, y1) = ord(b[1], b[3]);
    let (w, h) = (x1 - x0, y1 - y0);
    if w <= 0.0 || h <= 0.0 {
        return None;
    }
    let rx = if rx.is_finite() { rx.abs().min(w / 2.0) } else { 0.0 };
    let ry = if ry.is_finite() { ry.abs().min(h / 2.0) } else { 0.0 };
    if rx < 1.0e-6 && ry < 1.0e-6 {
        let pts = [[x0, y0], [x1, y0], [x1, y1], [x0, y1]];
        return Some(poly_line(&pts, true));
    }
    let (rk, ry_k) = (rx * KAPPA, ry * KAPPA);
    let mut cubics = Vec::new();
    let corner = |out: &mut Vec<[[f64; 2]; 3]>, a: [f64; 2], b: [f64; 2], c1: [f64; 2], c2: [f64; 2]| {
        out.push(line_to(a));
        out.push([c1, c2, b]);
    };
    // Start just after the bottom-left corner and walk counterclockwise in y-up space.
    let start = [x0 + rx, y0];
    corner(&mut cubics, [x1 - rx, y0], [x1, y0 + ry], [x1 - rx + rk, y0], [x1, y0 + ry - ry_k]);
    corner(&mut cubics, [x1, y1 - ry], [x1 - rx, y1], [x1, y1 - ry + ry_k], [x1 - rx + rk, y1]);
    corner(&mut cubics, [x0 + rx, y1], [x0, y1 - ry], [x0 + rx - rk, y1], [x0, y1 - ry + ry_k]);
    corner(&mut cubics, [x0, y0 + ry], start, [x0, y0 + ry - ry_k], [x0 + rx - rk, y0]);
    Some(Subpath { start, cubics, closed: true })
}

fn oval_sub(b: [f64; 4], arc: Option<(f64, f64, bool)>) -> Option<Subpath> {
    if !b.iter().all(|v| v.is_finite()) {
        return None;
    }
    let (x0, x1) = ord(b[0], b[2]);
    let (y0, y1) = ord(b[1], b[3]);
    let (rx, ry) = ((x1 - x0) / 2.0, (y1 - y0) / 2.0);
    if rx <= 0.0 || ry <= 0.0 {
        return None;
    }
    let c = [(x0 + x1) / 2.0, (y0 + y1) / 2.0];
    if let Some((start, end, pie)) = arc {
        return Some(arc_sub(c, rx, ry, start, end, pie));
    }
    let k = KAPPA;
    let right = [c[0] + rx, c[1]];
    let top = [c[0], c[1] + ry];
    let left = [c[0] - rx, c[1]];
    let bottom = [c[0], c[1] - ry];
    let cubics = vec![
        [[c[0] + rx, c[1] + k * ry], [c[0] + k * rx, c[1] + ry], top],
        [[c[0] - k * rx, c[1] + ry], [c[0] - rx, c[1] + k * ry], left],
        [[c[0] - rx, c[1] - k * ry], [c[0] - k * rx, c[1] - ry], bottom],
        [[c[0] + k * rx, c[1] - ry], [c[0] + rx, c[1] - k * ry], right],
    ];
    Some(Subpath { start: right, cubics, closed: true })
}

fn arc_sub(c: [f64; 2], rx: f64, ry: f64, start: f64, end: f64, pie: bool) -> Subpath {
    let mut sweep = end - start;
    if sweep <= 0.0 {
        sweep += 360.0;
    }
    if sweep > 360.0 {
        sweep = 360.0;
    }
    let steps = ((sweep / 12.0).ceil() as usize).clamp(1, 48);
    let at = |deg: f64| {
        let r = deg.to_radians();
        [c[0] + rx * r.cos(), c[1] + ry * r.sin()]
    };
    let mut pts = Vec::new();
    if pie {
        pts.push(c);
    }
    for i in 0..=steps {
        pts.push(at(start + sweep * (i as f64) / (steps as f64)));
    }
    if pie {
        pts.push(c);
    }
    poly_line(&pts, pie)
}

fn poly_sub(n: u16, cx: f64, cy: f64, r1: f64, r2: f64, a1: f64, a2: f64) -> Option<Subpath> {
    if !(cx.is_finite() && cy.is_finite() && r1.is_finite() && r2.is_finite() && a1.is_finite() && a2.is_finite()) {
        return None;
    }
    let n = usize::from(n);
    if !(3..=720).contains(&n) {
        return None;
    }
    let step = 360.0 / n as f64;
    let mut pts = Vec::new();
    for i in 0..n {
        let radius = if i % 2 == 0 { r1 } else { r2 };
        let mut ang = a1 + step * i as f64;
        if i % 2 == 1 {
            ang += a2;
        }
        let r = ang.to_radians();
        pts.push([cx + radius * r.cos(), cy + radius * r.sin()]);
    }
    Some(poly_line(&pts, true))
}

fn poly_line(pts: &[[f64; 2]], closed: bool) -> Subpath {
    let start = pts.first().copied().unwrap_or([0.0, 0.0]);
    let mut cubics = Vec::new();
    for p in pts.iter().skip(1) {
        cubics.push(line_to(*p));
    }
    if closed && pts.len() >= 2 {
        cubics.push(line_to(start));
    }
    Subpath { start, cubics, closed: closed && pts.len() >= 2 }
}

fn has_drawable(items: &[Item]) -> bool {
    items.iter().any(|item| match item {
        Item::Group(kids) | Item::ClipGroup { children: kids, .. } => has_drawable(kids),
        Item::Path(path) | Item::Compound(path) => !path.subpaths.is_empty(),
        Item::Text(text) => text.runs.iter().any(|run| !run.text.is_empty()),
    })
}

fn ord(a: f64, b: f64) -> (f64, f64) {
    if a <= b { (a, b) } else { (b, a) }
}

#[derive(Default)]
struct StylePick {
    fill: Option<u32>,
    stroke: Option<u32>,
    contents: Option<u32>,
    attrs: Option<u32>,
    filter: Option<u32>,
}

enum Step {
    Group { elements: u32, xform: u32, clip: bool },
    Composite { style: u32, elements: u32 },
    Path { style: u32, closed: bool, even_odd: bool },
    Shape(ShapeRec),
    Symbol { class: u32, matrix: Matrix },
    Blend([u32; 3]),
    Text(crate::records::TextRec),
    Image,
    Guides,
    Other(String),
    Missing,
}

fn slice_units(text: &str, from: usize, to: usize) -> String {
    if to <= from {
        return String::new();
    }
    String::from_utf16_lossy(&text.encode_utf16().skip(from).take(to - from).collect::<Vec<_>>())
}

impl RawFile {
    fn text(&self, id: u32) -> &str {
        match self.get(id) {
            Some(Rec::Str(s)) => s,
            _ => "",
        }
    }
}
