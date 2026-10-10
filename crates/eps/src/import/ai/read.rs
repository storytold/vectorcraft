//! The reader: runs the editing data's operators into a document.
//!
//! - Art space has y up; the first artboard's top left corner is the document's origin.
//! - Layers: `%AI5_BeginLayer`, `visible preview unlocked printing dimmed … colorIndex r g b …
//!   dimPercent … Lb`, `(name) Ln`, its art, `LB`.
//! - Groups `u` … `U`, compound paths `*u` … `*U`, clipping groups `q` … `Q` (the path painted
//!   with `W` clips); a group's transparency and name follow its end.
//! - Paths: `m`, `l`/`L`, `c`/`C`, `v`/`V`, `y`/`Y`, painted by `N n F f S s B b` (lower case
//!   closes), `(op) *` for a guide. Stroke `w J j M d`, fill rule `XR`, overprint `O`/`R`.
//! - State: hidden `Xw`, locked `A`, transparency `mode opacity isolate knockout shape Xy`.
//! - Names: an `/ArtDictionary` after an object, its `AI10_ArtUID` the name's XML id.
//! - Artboards: the `ArtboardArray` of the `/Document` dictionary, else `%AI3_Cropmarks`.
//! - Images: `%AI5_BeginRaster`, the colour space `XN`, `[matrix] bounds w h bits type alpha …`
//!   and the samples after `XI`.
//! - Art with an appearance the format can't write as plain art (several fills or strokes,
//!   effects, brushes) comes first as that look, drawn (a group), then the object itself on `%_`
//!   lines and `1 (style) XW`: the drawn look is kept, named after the object.
//!
//! Sections the reader doesn't need (patterns, brushes, symbols, styles, swatches, filters) are
//! skipped; type, symbols, pattern fills, placed files, legacy type and operators it doesn't know
//! are errors.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

use kurbo::{Affine, BezPath, PathEl, Point, Rect};
use vectorcraft_color::gradient::Gradient;
use vectorcraft_color::swatch::Swatch;
use vectorcraft_color::{BlendMode, Paint};
use vectorcraft_doc::{
    Appearance, AppearanceItem, Artboard, ColorMode, Dash, Document, FillLayer, ImageBlob, ImageObject, Knockout, LayerColor, LineCap, LineJoin,
    Node, NodeId, NodeKind, StrokeLayer,
};
use vectorcraft_geom::{FillRule, PathData};

use super::lex::{Lexer, Tok};
use super::obj::{self, Obj, V};
use super::paint::{self, GradientDef, Instance, Named};

/// Deepest nesting of layers, groups and dictionaries.
const MAX_DEPTH: usize = 256;
/// Most operands on the stack.
const MAX_STACK: usize = 1 << 20;
/// Most objects read.
const MAX_NODES: usize = 4_000_000;
/// Most points in one path.
const MAX_POINTS: usize = 4_000_000;
/// Largest image read (pixels).
const MAX_PIXELS: u64 = 1 << 25;
/// Largest side of the document and its artboards (points).
const MAX_SIDE: f64 = 1e6;

/// What the editing data holds: the document of its art, and where its art space is.
#[derive(Debug)]
pub struct Structure {
    /// The layers, artboards, swatches and colour mode. A text object is an empty group named
    /// [`TEXT_SLOT`] and its story's number, for the importer to fill.
    pub doc: Document,
    /// Art space (y up) → the document.
    pub to_doc: Affine,
    /// The art's bounding box in art space (`%%HiResBoundingBox`, else `%%BoundingBox`).
    pub bbox: Option<[f64; 4]>,
    /// The first artboard in art space.
    pub artboard: Option<Rect>,
    /// `%AI3_TemplateBox`, whose centre is the canvas's (the text document's origin).
    pub template: Option<[f64; 4]>,
    /// What layers that don't show have that isn't read (operators, objects): left out of them.
    pub hidden_unread: BTreeSet<String>,
    /// The names of text objects, by their slot's id.
    pub slot_names: HashMap<NodeId, String>,
    pub warnings: Vec<String>,
}

/// The name of the empty group that stands for a text object, followed by the number of its
/// story in the text document when the file says.
pub(crate) const TEXT_SLOT: &str = "\u{0}text";

/// Is `name` that of a text slot, and which story is it of?
pub(crate) fn slot_of(name: Option<&str>) -> Option<Option<u32>> {
    name?.strip_prefix(TEXT_SLOT).map(|rest| rest.split(':').next().and_then(|s| s.parse().ok()))
}

/// Which of its story's frames a text slot named `name` is (0 when it doesn't say).
pub(crate) fn slot_frame(name: Option<&str>) -> usize {
    name.and_then(|n| n.strip_prefix(TEXT_SLOT)).and_then(|rest| rest.split_once(':')).and_then(|(_, f)| f.parse().ok()).unwrap_or(0)
}

/// Read the structure of the editing data `data`.
pub fn read(data: &[u8]) -> Result<Structure, String> {
    let mut r = Reader::new(data);
    r.run()?;
    r.finish()
}

#[derive(Clone, Debug)]
struct GState {
    fill: Paint,
    stroke: Paint,
    width: f64,
    cap: LineCap,
    join: LineJoin,
    miter: f64,
    dash: Option<Dash>,
    rule: FillRule,
    opacity: f32,
    blend: BlendMode,
    isolate: bool,
    knockout: Knockout,
    knockout_shape: bool,
    overprint_fill: bool,
    overprint_stroke: bool,
    locked: bool,
    hidden: bool,
}

impl Default for GState {
    fn default() -> Self {
        let black = Paint::solid(vectorcraft_color::Color::BLACK);
        Self {
            fill: black.clone(),
            stroke: black,
            width: 1.0,
            cap: LineCap::Butt,
            join: LineJoin::Miter,
            miter: 10.0,
            dash: None,
            rule: FillRule::NonZero,
            opacity: 1.0,
            blend: BlendMode::Normal,
            isolate: false,
            knockout: Knockout::Off,
            knockout_shape: false,
            overprint_fill: false,
            overprint_stroke: false,
            locked: false,
            hidden: false,
        }
    }
}

/// The blend modes by their number in `Xy`.
const BLENDS: [BlendMode; 16] = [
    BlendMode::Normal,
    BlendMode::Multiply,
    BlendMode::Screen,
    BlendMode::Overlay,
    BlendMode::SoftLight,
    BlendMode::HardLight,
    BlendMode::ColorDodge,
    BlendMode::ColorBurn,
    BlendMode::Darken,
    BlendMode::Lighten,
    BlendMode::Difference,
    BlendMode::Exclusion,
    BlendMode::Hue,
    BlendMode::Saturation,
    BlendMode::Color,
    BlendMode::Luminosity,
];

#[derive(Debug, Default)]
struct LayerAttrs {
    name: String,
    visible: bool,
    preview: bool,
    locked: bool,
    printable: bool,
    dim: Option<u8>,
    color: Option<LayerColor>,
}

#[derive(Debug)]
enum Kind {
    Layer(Box<LayerAttrs>),
    Group,
    Compound,
    Clip,
    Obj(Box<Obj>),
}

#[derive(Debug)]
struct Frame {
    kind: Kind,
    children: Vec<Node>,
    /// Opened on a `%_` line.
    hidden: bool,
    /// [`Reader::after_object`] when it opened.
    after: bool,
    /// The state when it opened (a container's lock and visibility; a dictionary's state to go
    /// back to).
    gs: GState,
    /// The operand stack's height when it opened.
    base: usize,
    /// The clipping paths' indexes among the children.
    clips: Vec<usize>,
}

/// An image's operands, from `XN` and before `XI`.
#[derive(Debug, Default)]
struct Raster {
    space: Option<String>,
}

struct Reader<'a> {
    lex: Lexer<'a>,
    stack: Vec<V>,
    /// Where the `[` marks are on the stack, innermost last (some may have gone with operands).
    marks: Vec<usize>,
    gs: GState,
    frames: Vec<Frame>,
    path: BezPath,
    points: usize,
    /// The path is a clipping path (`W`).
    clip_next: bool,
    doc: Document,
    to_doc: Option<Affine>,
    cropmarks: Option<Rect>,
    artboards: Vec<(String, Rect)>,
    gradients: HashMap<String, Gradient>,
    gradient_def: Option<GradientDef>,
    instance: Option<Instance>,
    pending_fill: Option<Paint>,
    pending_stroke: Option<Paint>,
    spots: Vec<Named>,
    raster: Option<Raster>,
    /// The last object read is the last child of the innermost frame, and nothing started since.
    after_object: bool,
    nodes: usize,
    /// What layers that show have that this reader doesn't read (errors).
    unsupported: BTreeSet<String>,
    /// Operators the reader doesn't know on layers that show.
    unknown: BTreeSet<String>,
    /// What layers that don't show have that this reader doesn't read.
    hidden_unread: BTreeSet<String>,
    /// The header's bounding boxes and template box.
    bbox: Option<[f64; 4]>,
    hires: Option<[f64; 4]>,
    template: Option<[f64; 4]>,
    /// Colours set in each model, when the header doesn't say the document's.
    cmyk_colors: usize,
    rgb_colors: usize,
    color_model: Option<bool>,
    warnings: Vec<String>,
    /// Objects read as their drawn look (several fills or strokes, effects, brushes).
    drawn_looks: usize,
    /// Nodes read from `%_` lines.
    hidden_ids: HashSet<NodeId>,
    /// CMYK samples converted, by value.
    cmyk_cache: HashMap<[u8; 4], [u8; 3]>,
    /// The page ended: what follows isn't art.
    done: bool,
    /// The dictionary closed last was opened on a `%_` line.
    obj_hidden: bool,
    /// The names of text objects, by their slot's id.
    slot_names: HashMap<NodeId, String>,
    /// The artboard's size and centre in files without artboards or crop marks.
    art_size: Option<(f64, f64)>,
    template_center: Option<(f64, f64)>,
}

/// Operators that change nothing the reader keeps.
const IGNORED: &[&str] = &[
    "Ae",
    "AE",
    "Ap",
    "As",
    "Xd",
    "Xr",
    "XG",
    "Xh",
    "XH",
    "XF",
    "D",
    "X=",
    "X+",
    "Bc",
    "Xm",
    "XP",
    "Np",
    "TE",
    "TZ",
    "Xs",
    "Xt",
    "Xi",
    "XI",
    "`",
    "LB2",
    "Lc",
    "Xv",
    "XV",
    "Xq",
    "XQ",
    "Xg",
    "Xn",
    "Bn",
    "gsave",
    "grestore",
    "showpage",
    "annotatepage",
];

/// Sections skipped whole, by their markers without the `AI<version>_` prefix: begin → end.
const SKIPPED: &[(&str, &str)] = &[
    ("BeginPattern", "EndPattern"),
    ("BeginBrushPattern", "EndBrushPattern"),
    ("BeginSVGFilter", "EndSVGFilter"),
    ("BeginSymbol", "EndSymbol"),
    ("BeginPluginObject", "EndPluginObject"),
    ("BeginArtStyles", "EndArtStyles"),
    ("BeginArtStyleList", "EndArtStyleList"),
    ("BeginPalette", "EndPalette"),
    ("BeginSymbolList", "EndSymbolList"),
    ("BeginTextDocument", "EndTextDocument"),
    ("BeginEncoding", "EndEncoding"),
    ("Alternate_Content", "End_Versioned_Content"),
];

/// The note for a `/ForeignObject`, art the editor shows but doesn't edit.
pub const NON_NATIVE_ART: &str = "non-native art (a placed PDF's content) is left out: VectorCraft doesn't draw it from the editing data yet";

/// A section comment's marker without its `AI<version>_` prefix and value (`AI14_BeginSymbol` →
/// `BeginSymbol`); DSC comments (`%%BeginProlog`) keep their `%`.
fn marker(c: &str) -> &str {
    let word = c.split(|ch: char| ch == ':' || ch.is_whitespace()).next().unwrap_or(c);
    word.strip_prefix("AI").map(|r| r.trim_start_matches(|ch: char| ch.is_ascii_digit())).and_then(|r| r.strip_prefix('_')).unwrap_or(word)
}

impl<'a> Reader<'a> {
    fn new(data: &'a [u8]) -> Self {
        let mut doc = Document::new(1.0, 1.0);
        doc.layers.clear();
        Self {
            lex: Lexer::new(data),
            stack: Vec::new(),
            marks: Vec::new(),
            gs: GState::default(),
            frames: Vec::new(),
            path: BezPath::new(),
            points: 0,
            clip_next: false,
            doc,
            to_doc: None,
            cropmarks: None,
            artboards: Vec::new(),
            gradients: HashMap::new(),
            gradient_def: None,
            instance: None,
            pending_fill: None,
            pending_stroke: None,
            spots: Vec::new(),
            raster: None,
            after_object: false,
            nodes: 0,
            unsupported: BTreeSet::new(),
            unknown: BTreeSet::new(),
            hidden_unread: BTreeSet::new(),
            bbox: None,
            hires: None,
            template: None,
            cmyk_colors: 0,
            rgb_colors: 0,
            color_model: None,
            warnings: Vec::new(),
            drawn_looks: 0,
            hidden_ids: HashSet::new(),
            cmyk_cache: HashMap::new(),
            done: false,
            obj_hidden: false,
            slot_names: HashMap::new(),
            art_size: None,
            template_center: None,
        }
    }

    fn run(&mut self) -> Result<(), String> {
        while let Some(t) = self.lex.next_token() {
            if self.done {
                break;
            }
            let hidden = t.hidden;
            match t.tok {
                Tok::Num(v) => self.push(V::Num(v))?,
                Tok::Str(s) | Tok::Hex(s) => self.push(V::Str(s))?,
                Tok::Name(n) => self.push(V::Name(obj::text(n)))?,
                Tok::Word(w) => {
                    // Operators are ASCII; others are unknown words all the same.
                    let w = std::str::from_utf8(w).unwrap_or("?");
                    self.op(w, hidden)?;
                }
                Tok::Comment(c) => self.comment(c)?,
                Tok::Data(d) => self.image(d)?,
            }
        }
        if self.frames.iter().any(|f| matches!(f.kind, Kind::Layer(_))) {
            return Err("it ends inside a layer".into());
        }
        Ok(())
    }

    fn push(&mut self, v: V) -> Result<(), String> {
        if self.stack.len() >= MAX_STACK {
            return Err("it has too many values in a row".into());
        }
        self.stack.push(v);
        Ok(())
    }

    /// The operands of the operator being run (above the innermost dictionary's), taken off.
    fn operands(&mut self) -> Vec<V> {
        let base = self.frames.last().map_or(0, |f| f.base).min(self.stack.len());
        self.stack.split_off(base)
    }

    fn nums(vals: &[V]) -> Vec<f64> {
        vals.iter().filter_map(V::num).collect()
    }

    // ---- comments --------------------------------------------------------------------------

    fn comment(&mut self, c: &[u8]) -> Result<(), String> {
        let line = String::from_utf8_lossy(c);
        let line = line.trim_end();
        let m = marker(line);
        if let Some((_, end)) = SKIPPED.iter().find(|(b, _)| *b == m) {
            return self.skip_to(end);
        }
        match m {
            "%BeginProlog" => return self.skip_to("%EndProlog"),
            "%BeginResource" => return self.skip_to("%EndResource"),
            // The art ends with the page.
            "%PageTrailer" | "%Trailer" => {
                self.done = true;
                return Ok(());
            }
            _ => {}
        }
        if line.starts_with("AI5_BeginLayer") {
            return self.open(Kind::Layer(Box::default()), false);
        }
        if line.starts_with("AI5_BeginRaster") {
            self.raster = Some(Raster::default());
        } else if line.starts_with("AI5_EndRaster") {
            self.raster = None;
        } else if line.starts_with("AI5_BeginPlace") {
            self.unreadable("placed files");
        } else if self.frames.is_empty()
            && let Some(v) = line.strip_prefix("%HiResBoundingBox:")
        {
            self.hires = four(v).or(self.hires);
        } else if self.frames.is_empty()
            && let Some(v) = line.strip_prefix("%BoundingBox:")
        {
            self.bbox = four(v).or(self.bbox);
        } else if let Some(v) = line.strip_prefix("AI3_Cropmarks:") {
            let n: Vec<f64> = v.split_whitespace().filter_map(|w| w.parse().ok()).collect();
            if let [x0, y0, x1, y1] = n.as_slice() {
                let r = Rect::new(*x0, *y0, *x1, *y1);
                if sane(r) {
                    self.cropmarks = Some(r);
                }
            }
        } else if let Some(v) = line.strip_prefix("AI5_ArtSize:") {
            self.art_size = two(v);
        } else if let Some(v) = line.strip_prefix("AI3_TemplateBox:") {
            self.template_center = two(v);
            self.template = four(v).or(self.template);
        } else if let Some(v) = line.strip_prefix("AI9_ColorModel:") {
            self.color_model = Some(v.trim() == "2");
        }
        Ok(())
    }

    /// Skip to the comment with marker `end` (sections of the same kind nested in it count, and
    /// versioned content in an alternate).
    fn skip_to(&mut self, end: &str) -> Result<(), String> {
        let begin = SKIPPED.iter().find(|(_, e)| *e == end).map_or("", |(b, _)| *b);
        let begin = if end == "End_Versioned_Content" { "Begin_Content_if_version_gt" } else { begin };
        let mut depth = 0usize;
        while let Some(t) = self.lex.next_token() {
            if let Tok::Comment(c) = t.tok {
                let line = String::from_utf8_lossy(c);
                let m = marker(&line);
                if m == begin {
                    depth += 1;
                } else if m == end {
                    if depth == 0 {
                        return Ok(());
                    }
                    depth -= 1;
                }
            }
        }
        Err(format!("its section ending with `%{end}` doesn't end"))
    }

    // ---- frames ----------------------------------------------------------------------------

    fn open(&mut self, kind: Kind, hidden: bool) -> Result<(), String> {
        if self.frames.len() >= MAX_DEPTH {
            return Err("its objects are nested too deeply".into());
        }
        if matches!(kind, Kind::Layer(_)) {
            self.ensure_space();
        }
        let base = if matches!(kind, Kind::Obj(_)) { self.stack.len() } else { self.frames.last().map_or(0, |f| f.base) };
        self.frames.push(Frame { kind, children: Vec::new(), hidden, after: self.after_object, gs: self.gs.clone(), base, clips: Vec::new() });
        self.after_object = false;
        Ok(())
    }

    /// Close the innermost frame, which must be of the kind `is` accepts.
    fn close(&mut self, what: &str, is: impl Fn(&Kind) -> bool) -> Result<Frame, String> {
        match self.frames.last() {
            Some(f) if is(&f.kind) => self.frames.pop().ok_or_else(|| format!("`{what}` closes nothing")),
            _ => Err(format!("`{what}` closes something it didn't open")),
        }
    }

    /// Add a node to the innermost container.
    fn add(&mut self, node: Node, hidden: bool) -> Result<(), String> {
        self.nodes += 1;
        if self.nodes > MAX_NODES {
            return Err(format!("it has more than {MAX_NODES} objects"));
        }
        if hidden {
            self.hidden_ids.insert(node.id);
        }
        match self.frames.last_mut() {
            Some(f) => f.children.push(node),
            None => return Err("it has art outside its layers".into()),
        }
        self.after_object = true;
        Ok(())
    }

    /// The last object read, while nothing else started.
    fn last_object(&mut self) -> Option<&mut Node> {
        if !self.after_object {
            return None;
        }
        self.frames.last_mut()?.children.last_mut()
    }

    fn new_node(&mut self, kind: NodeKind, gs: &GState) -> Node {
        let mut n = Node::new(self.doc.alloc_id(), kind);
        n.visible = !gs.hidden;
        n.locked = gs.locked;
        n
    }

    fn end_container(&mut self, what: &str, hidden: bool) -> Result<(), String> {
        let f = match what {
            "U" => self.close(what, |k| matches!(k, Kind::Group))?,
            "*U" => self.close(what, |k| matches!(k, Kind::Compound))?,
            _ => self.close(what, |k| matches!(k, Kind::Clip))?,
        };
        let Frame { kind, children: mut kids, gs, clips, hidden: opened_hidden, .. } = f;
        // A compound path of clipping paths clips its clipping group.
        let clipping = matches!(kind, Kind::Compound) && !clips.is_empty();
        let node = match kind {
            Kind::Compound => {
                let Some(first) = kids.first() else { return Ok(()) };
                let (appearance, opacity, blend, rule) = (first.appearance.clone(), first.opacity, first.blend, path_rule(first));
                for k in &mut kids {
                    k.opacity = 1.0;
                    k.blend = BlendMode::Normal;
                    k.visible = true;
                    k.locked = false;
                }
                let mut n = self.new_node(NodeKind::Compound { children: kids.into_iter().map(Arc::new).collect(), rule }, &gs);
                n.appearance = appearance;
                n.opacity = opacity;
                n.blend = blend;
                n
            }
            // A group with a clipping path (`q` … `Q`, or a `W` path in a group) is a clipping group.
            _ => {
                let (children, clip) = self.clip_first(kids, &clips);
                self.new_node(NodeKind::Group { children, clip }, &gs)
            }
        };
        if clipping && let Some(f) = self.frames.last_mut() {
            f.clips.push(f.children.len());
        }
        self.add(node, opened_hidden || hidden)
    }

    /// `kids` with the clipping paths among them (indexes `clips`) first, several as one compound
    /// path → the children and whether they clip.
    fn clip_first(&mut self, kids: Vec<Node>, clips: &[usize]) -> (Vec<Arc<Node>>, bool) {
        let (mut clip, rest): (Vec<Indexed>, Vec<Indexed>) = kids.into_iter().enumerate().partition(|(i, _)| clips.contains(i));
        let clip_node = match clip.len() {
            0 => None,
            1 => clip.pop().map(|(_, n)| n),
            _ => {
                let rule = clip.first().map_or(FillRule::NonZero, |(_, n)| path_rule(n));
                let children = clip.into_iter().map(|(_, n)| Arc::new(n)).collect();
                Some(Node::new(self.doc.alloc_id(), NodeKind::Compound { children, rule }))
            }
        };
        let clips = clip_node.is_some();
        let children = clip_node.into_iter().chain(rest.into_iter().map(|(_, n)| n)).map(Arc::new).collect();
        (children, clips)
    }

    fn end_layer(&mut self) -> Result<(), String> {
        let f = self.close("LB", |k| matches!(k, Kind::Layer(_)))?;
        let Kind::Layer(a) = f.kind else { return Ok(()) };
        let (children, clip) = self.clip_first(f.children, &f.clips);
        let color = a.color.unwrap_or(LayerColor::Preset(0));
        let mut layer = Node::new(
            self.doc.alloc_id(),
            NodeKind::Layer { color, template: false, printable: a.printable, children, clip, preview: a.preview, dim_images: a.dim },
        );
        layer.name = Some(a.name.clone());
        layer.visible = a.visible;
        layer.locked = a.locked;
        match self.frames.last_mut() {
            Some(parent) => {
                parent.children.push(layer);
                self.after_object = true;
            }
            None => {
                self.doc.layers.push(Arc::new(layer));
                self.after_object = false;
            }
        }
        Ok(())
    }

    // ---- operators -------------------------------------------------------------------------

    fn op(&mut self, w: &str, hidden: bool) -> Result<(), String> {
        match w {
            "[" => {
                self.marks.push(self.stack.len());
                return self.push(V::Mark);
            }
            "]" => {
                // The innermost `[` still on the stack (the stack is cut back by operators).
                let mut at = self.stack.len();
                while let Some(m) = self.marks.pop() {
                    if matches!(self.stack.get(m), Some(V::Mark)) {
                        at = m;
                        break;
                    }
                }
                let items: Vec<V> = self.stack.split_off(at).into_iter().skip(1).collect();
                return self.push(V::Arr(items));
            }
            ":" => return self.begin_obj(hidden),
            "," => {
                if let Some(Frame { kind: Kind::Obj(o), base, .. }) = self.frames.last_mut() {
                    obj::add_entry(o, &mut self.stack, *base);
                    // Binary data in ASCII85 follows, up to its `~>`: a `/Binary` dictionary's
                    // (`/ASCII85Decode ,`) or a non-native art object's (`/ForeignObject`'s
                    // `/Data ,`, the PDF it keeps), in comment lines. ASCII85 has `_`, so a line
                    // of it may start with `%_` and read as hidden tokens: a `(` there would open
                    // a string that swallows the rest of the layer.
                    let key = o.entries.last().and_then(|(k, _)| k.as_deref());
                    if (o.ty == "Binary" && key == Some("ASCII85Decode")) || (o.ty == "ForeignObject" && key == Some("Data")) {
                        self.lex.skip_past(b"~>");
                    }
                }
                return Ok(());
            }
            ";" => return self.end_obj(),
            _ => {}
        }
        // What follows an object (its name, transparency and style) ends with anything else.
        if !matches!(w, "Xy" | "Xd" | "XW" | "BB") {
            self.after_object = false;
        }
        let vals = self.operands();
        if let Some(d) = self.gradient_def.as_mut() {
            match w {
                "Bs" => d.stop(&vals, &mut self.spots),
                "BD" => {
                    if let Some((name, g)) = self.gradient_def.take().and_then(GradientDef::finish) {
                        self.gradients.insert(name, g);
                    }
                }
                _ => {}
            }
            return Ok(());
        }
        match w {
            // Containers.
            "u" => {
                self.end_path_quietly();
                self.open(Kind::Group, hidden)?
            }
            "*u" => {
                self.end_path_quietly();
                self.open(Kind::Compound, hidden)?
            }
            "q" => {
                self.end_path_quietly();
                self.open(Kind::Clip, hidden)?
            }
            "U" | "*U" | "Q" => self.end_container(w, hidden)?,
            "Lb" => self.layer_attrs(&vals),
            "Ln" => {
                if let Some(Frame { kind: Kind::Layer(a), .. }) = self.frames.last_mut() {
                    a.name = vals.iter().rev().find_map(V::text).unwrap_or_default();
                }
            }
            "LB" => self.end_layer()?,
            // Paths.
            "m" | "l" | "L" | "c" | "C" | "v" | "V" | "y" | "Y" => self.path_op(w, &vals)?,
            "h" => close(&mut self.path),
            // Ends a path that stays open.
            "H" => {}
            "W" => self.clip_next = true,
            "N" | "n" | "F" | "f" | "S" | "s" | "B" | "b" => self.paint(w, hidden, false)?,
            "*" => {
                let op = vals.iter().rev().find_map(V::text).unwrap_or_else(|| "N".into());
                self.paint(&op, hidden, true)?;
            }
            // Colours.
            "g" | "G" | "k" | "K" | "x" | "X" | "Xa" | "XA" | "Xx" | "XX" => self.color(w, &vals),
            "p" | "P" => self.unreadable("pattern fills"),
            "O" => self.gs.overprint_fill = Self::nums(&vals).last() == Some(&1.0),
            "R" => self.gs.overprint_stroke = Self::nums(&vals).last() == Some(&1.0),
            "XR" => self.gs.rule = if Self::nums(&vals).last() == Some(&1.0) { FillRule::EvenOdd } else { FillRule::NonZero },
            "w" => {
                if let Some(v) = Self::nums(&vals).last().filter(|v| v.is_finite() && **v >= 0.0) {
                    self.gs.width = *v;
                }
            }
            "J" => {
                self.gs.cap = match Self::nums(&vals).last().map(|v| *v as i64) {
                    Some(1) => LineCap::Round,
                    Some(2) => LineCap::Square,
                    _ => LineCap::Butt,
                }
            }
            "j" => {
                self.gs.join = match Self::nums(&vals).last().map(|v| *v as i64) {
                    Some(1) => LineJoin::Round,
                    Some(2) => LineJoin::Bevel,
                    _ => LineJoin::Miter,
                }
            }
            "M" => {
                if let Some(v) = Self::nums(&vals).last().filter(|v| v.is_finite() && **v >= 1.0) {
                    self.gs.miter = v.min(500.0);
                }
            }
            "d" => self.dash(&vals),
            // Transparency and state.
            "Xy" => self.transparency(&vals),
            "Xw" => self.gs.hidden = Self::nums(&vals).last() == Some(&1.0),
            "A" => self.gs.locked = Self::nums(&vals).last() == Some(&1.0),
            "XW" => self.style_marker(&vals, hidden),
            // Gradients.
            "Bd" => self.gradient_def = GradientDef::begin(&vals),
            "Bs" | "BD" => {}
            "Bb" => self.instance = Some(Instance::begin(&vals)),
            "Bg" => {
                if let Some(i) = self.instance.as_mut() {
                    i.bg(&vals);
                }
            }
            "Bm" => {
                if let Some(i) = self.instance.as_mut() {
                    i.bm(&vals);
                }
            }
            "Bh" => {
                if let Some(i) = self.instance.as_mut() {
                    i.bh(&vals);
                }
            }
            "BB" => {
                if let Some(i) = self.instance.take() {
                    self.set_instance(&i);
                }
            }
            // Images.
            "XN" => {
                if let Some(r) = self.raster.as_mut() {
                    r.space = vals.iter().rev().find_map(V::text);
                }
            }
            // Type in the legacy format.
            "To" | "TO" | "Tp" | "TP" | "Tx" | "TX" | "Tj" | "Tk" => self.unreadable("type in the legacy format"),
            _ if IGNORED.contains(&w) => {}
            // Before the art, the setup runs procedures of the printing format.
            _ if self.frames.is_empty() => {}
            _ => {
                let op: String = w.chars().take(24).collect();
                if !self.shown() {
                    if self.hidden_unread.len() < 16 {
                        self.hidden_unread.insert(format!("`{op}`"));
                    }
                } else if self.unknown.len() < 16 {
                    self.unknown.insert(op);
                }
            }
        }
        Ok(())
    }

    /// `:` after `/Type`: a dictionary opens.
    fn begin_obj(&mut self, hidden: bool) -> Result<(), String> {
        let ty = match self.stack.pop() {
            Some(V::Name(n)) => n,
            Some(other) => {
                self.stack.push(other);
                String::new()
            }
            None => String::new(),
        };
        self.open(Kind::Obj(Box::new(Obj { ty, ..Obj::default() })), hidden)?;
        // A dictionary's state starts afresh (styles set theirs inside).
        self.gs = GState::default();
        Ok(())
    }

    /// `;`: the innermost dictionary closes.
    fn end_obj(&mut self) -> Result<(), String> {
        if !matches!(self.frames.last(), Some(Frame { kind: Kind::Obj(_), .. })) {
            return Ok(());
        }
        let Some(f) = self.frames.pop() else { return Ok(()) };
        let Kind::Obj(mut o) = f.kind else { return Ok(()) };
        obj::close(&mut o, &mut self.stack, f.base);
        self.obj_hidden = f.hidden;
        // Art made inside a dictionary (type's paths) isn't kept.
        self.gs = f.gs;
        self.after_object = f.after;
        if matches!(self.frames.last(), Some(Frame { kind: Kind::Obj(_), .. })) {
            return self.push(V::Obj(Rc::new(*o)));
        }
        match o.ty.as_str() {
            "ArtDictionary" => self.art_dictionary(&o),
            "Document" => self.document(&o),
            "AI11Text" => self.text_slot(&o)?,
            "SymbolInstance" => self.unreadable("symbols"),
            // Art Illustrator shows but doesn't edit (a placed PDF's content): its PDF is kept in
            // the dictionary, which nothing draws from yet, so the file opens without it.
            "ForeignObject" => self.warn(NON_NATIVE_ART),
            _ => {}
        }
        Ok(())
    }

    /// A text object (`/AI11Text`): an empty group standing for it, named after its story.
    fn text_slot(&mut self, o: &Obj) -> Result<(), String> {
        let index = |key: &str| o.nums(key).first().copied().filter(|v| (0.0..f64::from(u32::MAX)).contains(v)).map(|v| v as u32);
        let (story, frame) = (index("StoryIndex"), index("FrameIndex").unwrap_or(0));
        let gs = self.gs.clone();
        let mut n = self.new_node(NodeKind::Group { children: vec![], clip: false }, &gs);
        n.name = Some(story.map_or_else(|| TEXT_SLOT.to_string(), |s| format!("{TEXT_SLOT}{s}:{frame}")));
        let hidden = self.obj_hidden;
        self.add(n, hidden)
    }

    /// Note something the data has that isn't read: an error on a layer that shows, left out of a
    /// layer that doesn't.
    fn unreadable(&mut self, what: &str) {
        if self.shown() {
            self.unsupported.insert(what.into());
        } else {
            self.hidden_unread.insert(what.into());
        }
    }

    /// Do the layers being read show?
    fn shown(&self) -> bool {
        self.frames.iter().all(|f| !matches!(&f.kind, Kind::Layer(a) if !a.visible))
    }

    /// An object's dictionary: its name (`AIArtName`, or the XML id `AI10_ArtUID`).
    fn art_dictionary(&mut self, o: &Obj) {
        let name = match o.text("AIArtName") {
            Some(name) => name,
            None => match o.obj("AI10_ArtUID").and_then(Obj::content_text) {
                Some(id) => obj::xml_name(&id),
                None => return,
            },
        };
        let Some(n) = self.last_object().filter(|n| !matches!(n.kind, NodeKind::Layer { .. }) && !name.is_empty()) else { return };
        if slot_of(n.name.as_deref()).is_some() {
            let id = n.id;
            self.slot_names.insert(id, name);
        } else {
            n.name = Some(name);
        }
    }

    /// The `/Document` dictionary: the artboards.
    fn document(&mut self, o: &Obj) {
        let Some(list) = find_entry(o, "ArtboardArray", 0) else { return };
        for (i, ab) in list.items().enumerate().take(1000) {
            let (p1, p2) = (ab.nums("PositionPoint1"), ab.nums("PositionPoint2"));
            let ([x0, y0, ..], [x1, y1, ..]) = (p1.as_slice(), p2.as_slice()) else { continue };
            let r = Rect::new(*x0, *y0, *x1, *y1).abs();
            if !sane(r) || r.area() <= 0.0 {
                continue;
            }
            let name = ab.text("Name").unwrap_or_else(|| format!("Artboard {}", i + 1));
            self.artboards.push((name, r));
        }
    }

    /// `… Lb`: a layer's options.
    fn layer_attrs(&mut self, vals: &[V]) {
        let n = Self::nums(vals);
        let Some(Frame { kind: Kind::Layer(a), .. }) = self.frames.last_mut() else { return };
        let flag = |i: usize, default: bool| n.get(i).map_or(default, |v| *v != 0.0);
        a.visible = flag(0, true);
        a.preview = flag(1, true);
        a.locked = !flag(2, true);
        a.printable = flag(3, true);
        let dim_pct = n.get(12).copied().filter(|v| v.is_finite()).unwrap_or(50.0).clamp(0.0, 100.0);
        a.dim = flag(4, false).then_some(dim_pct.round() as u8);
        let index = n.get(7).copied().unwrap_or(0.0);
        let rgb = |i: usize| n.get(i).map_or(0, |v| v.clamp(0.0, 255.0) as u8);
        a.color = Some(if (0.0..vectorcraft_doc::LAYER_COLORS.len() as f64).contains(&index) && index.fract() == 0.0 {
            LayerColor::Preset(index as u8)
        } else {
            LayerColor::Custom([rgb(8), rgb(9), rgb(10)])
        });
    }

    fn color(&mut self, w: &str, vals: &[V]) {
        let Some((p, named)) = paint::color_op(w, vals) else { return };
        match p.color() {
            Some(vectorcraft_color::Color::Rgb { .. }) => self.rgb_colors += 1,
            Some(vectorcraft_color::Color::Cmyk { .. }) => self.cmyk_colors += 1,
            _ => {}
        }
        if let Some(n) = named {
            self.spots.push(n);
        }
        if w.chars().all(|c| c.is_ascii_lowercase()) || w == "Xa" || w == "Xx" {
            self.gs.fill = p;
        } else {
            self.gs.stroke = p;
        }
    }

    /// `[dashes] phase d`.
    fn dash(&mut self, vals: &[V]) {
        let pattern: Vec<f64> = vals.iter().rev().find_map(|v| if let V::Arr(a) = v { Some(Self::nums(a)) } else { None }).unwrap_or_default();
        let offset = Self::nums(vals).last().copied().unwrap_or(0.0);
        self.gs.dash = (!pattern.is_empty() && pattern.len() <= 64 && pattern.iter().all(|v| v.is_finite() && *v >= 0.0)).then(|| Dash {
            pattern,
            offset: if offset.is_finite() { offset } else { 0.0 },
            align_corners: false,
        });
    }

    /// `mode opacity isolate knockout shape Xy`; after a group's end, the group's too.
    fn transparency(&mut self, vals: &[V]) {
        let n = Self::nums(vals);
        let [mode, opacity, rest @ ..] = n.as_slice() else { return };
        let gs = &mut self.gs;
        gs.blend = BLENDS.get(*mode as usize).copied().filter(|_| *mode >= 0.0).unwrap_or(BlendMode::Normal);
        gs.opacity = if opacity.is_finite() { opacity.clamp(0.0, 1.0) as f32 } else { 1.0 };
        gs.isolate = rest.first().is_some_and(|v| *v != 0.0);
        gs.knockout = match rest.get(1).map(|v| *v as i64) {
            Some(1) => Knockout::On,
            Some(2) => Knockout::Neutral,
            _ => Knockout::Off,
        };
        gs.knockout_shape = rest.get(2).is_some_and(|v| *v != 0.0);
        let gs = self.gs.clone();
        if let Some(n) = self.last_object()
            && matches!(n.kind, NodeKind::Group { .. } | NodeKind::Compound { .. })
        {
            n.opacity = gs.opacity;
            n.blend = gs.blend;
            n.isolate = gs.isolate;
            n.knockout = gs.knockout;
        }
    }

    /// `n (style) XW` after an object: `1 (style)` after the object written on `%_` lines, its
    /// drawn look before it. Other markers end what follows an object.
    fn style_marker(&mut self, vals: &[V], hidden: bool) {
        let code = Self::nums(vals).last().copied().unwrap_or(0.0);
        let style = vals.iter().rev().find_map(V::text).unwrap_or_default();
        let after = std::mem::take(&mut self.after_object);
        if hidden || code != 1.0 || style.is_empty() || !after {
            return;
        }
        let Some(f) = self.frames.last_mut() else { return };
        let n = f.children.len();
        let (Some(object), Some(look)) = (n.checked_sub(1).and_then(|i| f.children.get(i)), n.checked_sub(2).and_then(|i| f.children.get(i))) else {
            return;
        };
        if !self.hidden_ids.contains(&object.id) || self.hidden_ids.contains(&look.id) || !matches!(look.kind, NodeKind::Group { clip: false, .. }) {
            return;
        }
        let Some(object) = f.children.pop() else { return };
        let Some(look) = f.children.last_mut() else { return };
        strip(look, &self.hidden_ids);
        look.name = object.name.clone().or(look.name.take());
        look.visible = object.visible;
        look.locked = object.locked;
        self.drawn_looks += 1;
    }

    // ---- paths -----------------------------------------------------------------------------

    fn path_op(&mut self, w: &str, vals: &[V]) -> Result<(), String> {
        let n = Self::nums(vals);
        let p = |i: usize| -> Option<Point> {
            let at = n.len().checked_sub(i)?;
            Some(Point::new(*n.get(at)?, *n.get(at + 1)?))
        };
        self.points += 1;
        if self.points > MAX_POINTS {
            return Err("a path has too many points".into());
        }
        let current = self.path.elements().last().and_then(|e| e.end_point());
        match w {
            "m" => {
                let Some(a) = p(2) else { return Ok(()) };
                self.path.move_to(a);
            }
            _ if current.is_none() => {}
            "l" | "L" => {
                if let Some(a) = p(2) {
                    self.path.line_to(a);
                }
            }
            "c" | "C" => {
                if let (Some(a), Some(b), Some(c)) = (p(6), p(4), p(2)) {
                    self.path.curve_to(a, b, c);
                }
            }
            "v" | "V" => {
                if let (Some(cur), Some(b), Some(c)) = (current, p(4), p(2)) {
                    self.path.curve_to(cur, b, c);
                }
            }
            _ => {
                if let (Some(a), Some(c)) = (p(4), p(2)) {
                    self.path.curve_to(a, c, c);
                }
            }
        }
        Ok(())
    }

    /// A path left unpainted before a container opens.
    fn end_path_quietly(&mut self) {
        self.path = BezPath::new();
        self.points = 0;
        self.clip_next = false;
    }

    /// Paint the current path (`op`), as a guide when `guide`.
    fn paint(&mut self, op: &str, hidden: bool, guide: bool) -> Result<(), String> {
        let mut bp = std::mem::take(&mut self.path);
        self.points = 0;
        let clip = std::mem::take(&mut self.clip_next);
        if let Some(i) = self.instance.take() {
            self.set_instance(&i);
        }
        let (fill_paint, stroke_paint) = (self.pending_fill.take(), self.pending_stroke.take());
        if bp.elements().is_empty() {
            return Ok(());
        }
        if op.chars().next().is_some_and(|c| c.is_ascii_lowercase()) {
            close(&mut bp);
        }
        let to_doc = self.ensure_space();
        let path = PathData::from_bezpath(&(to_doc * bp));
        let gs = self.gs.clone();
        let (fill, stroke) = match op.to_ascii_lowercase().as_str() {
            "f" => (true, false),
            "s" => (false, true),
            "b" => (true, true),
            _ => (false, false),
        };
        let mut items = Vec::new();
        if fill && !clip {
            items.push(AppearanceItem::Fill(FillLayer { overprint: gs.overprint_fill, ..FillLayer::new(fill_paint.unwrap_or(gs.fill.clone())) }));
        }
        if stroke && !clip {
            let mut st = StrokeLayer::new(stroke_paint.unwrap_or(gs.stroke.clone()), gs.width);
            st.cap = gs.cap;
            st.join = gs.join;
            st.miter_limit = gs.miter;
            st.dash = gs.dash.clone();
            st.overprint = gs.overprint_stroke;
            items.push(AppearanceItem::Stroke(st));
        }
        let kind = NodeKind::Path { path, rule: gs.rule, live: None, clipping: clip, guide };
        let mut node = self.new_node(kind, &gs);
        node.appearance = Appearance { items, ..Appearance::default() };
        node.opacity = gs.opacity;
        node.blend = gs.blend;
        node.knockout_shape = gs.knockout_shape;
        if clip && let Some(f) = self.frames.last_mut() {
            f.clips.push(f.children.len());
        }
        self.add(node, hidden)
    }

    fn set_instance(&mut self, i: &Instance) {
        let to_doc = self.ensure_space();
        match i.paint(&self.gradients, to_doc) {
            Some(p) if i.stroke => self.pending_stroke = Some(p),
            Some(p) => self.pending_fill = Some(p),
            None => {
                let name = i.name.clone().unwrap_or_default();
                self.warn(&format!("gradient “{name}” isn't defined in the file: a plain colour stands in for it"));
            }
        }
    }

    // ---- images ----------------------------------------------------------------------------

    /// The samples after `XI`, its operands on the stack.
    fn image(&mut self, data: &[u8]) -> Result<(), String> {
        let vals = self.operands();
        let space = self.raster.as_ref().and_then(|r| r.space.clone()).unwrap_or_default();
        let Some(m) = vals.iter().find_map(|v| if let V::Arr(a) = v { Some(Self::nums(a)) } else { None }) else { return Ok(()) };
        let n = Self::nums(&vals);
        let [_, _, _, _, w, h, bits, kind, alpha, _, binary, ..] = n.as_slice() else { return Ok(()) };
        let (w, h) = (*w as u64, *h as u64);
        if w == 0 || h == 0 || w.saturating_mul(h) > MAX_PIXELS || ![1.0, 8.0].contains(bits) {
            self.warn("an image VectorCraft can't read was left out");
            return Ok(());
        }
        let channels: usize = match space.as_str() {
            "DeviceGray" => 1,
            "DeviceCMYK" => 4,
            "DeviceRGB" => 3,
            _ => match *kind as i64 {
                1 => 1,
                4 => 4,
                _ => 3,
            },
        };
        let decoded;
        let data = if *binary == 0.0 {
            let digits: Vec<u8> = data.iter().copied().filter(|b| b.is_ascii_hexdigit()).collect();
            decoded = super::super::lex::hex_decode(&digits).unwrap_or_default();
            decoded.as_slice()
        } else {
            data
        };
        let row = if *bits == 1.0 { (w as usize * channels).div_ceil(8) } else { w as usize * channels };
        let need = row.saturating_mul(h as usize);
        // An alpha channel follows the colour samples, one byte a pixel.
        let alpha_len = if *alpha > 0.0 { (w * h) as usize } else { 0 };
        let (Some(samples), opacity) = (data.get(..need), data.get(need..need.saturating_add(alpha_len))) else {
            self.warn("an image whose data is cut short was left out");
            return Ok(());
        };
        let opacity = opacity.filter(|_| alpha_len > 0);
        let mut rgba = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h as usize {
            let line = samples.get(y * row..(y + 1) * row).unwrap_or_default();
            for x in 0..w as usize {
                let px = if *bits == 1.0 {
                    let bit = line.get(x / 8).map_or(0, |b| (b >> (7 - x % 8)) & 1);
                    let v = if bit == 1 { 255 } else { 0 };
                    [v, v, v]
                } else {
                    let s = |c: usize| line.get(x * channels + c).copied().unwrap_or(0);
                    match channels {
                        1 => [s(0); 3],
                        4 => {
                            let key = [s(0), s(1), s(2), s(3)];
                            if self.cmyk_cache.len() > 1 << 16 {
                                self.cmyk_cache.clear();
                            }
                            *self.cmyk_cache.entry(key).or_insert_with(|| cmyk_rgb(key))
                        }
                        _ => [s(0), s(1), s(2)],
                    }
                };
                let a = opacity.and_then(|o| o.get(y * w as usize + x)).copied().unwrap_or(255);
                rgba.extend_from_slice(&[px[0], px[1], px[2], a]);
            }
        }
        let Some(img) = image::RgbaImage::from_raw(w as u32, h as u32, rgba) else { return Ok(()) };
        let mut png = vec![];
        if img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).is_err() {
            self.warn("an image VectorCraft can't read was left out");
            return Ok(());
        }
        // The matrix maps pixels (y down) into art space with its y flipped.
        let [a, b, c, d, tx, ty] = m.as_slice() else { return Ok(()) };
        let to_doc = self.ensure_space();
        let flip = Affine::new([1.0, 0.0, 0.0, -1.0, 0.0, 0.0]);
        let xf = to_doc * flip * Affine::new([*a, *b, *c, *d, *tx, -*ty]);
        if !xf.as_coeffs().iter().all(|v| v.is_finite()) || xf.determinant().abs() < 1e-12 {
            return Ok(());
        }
        let blob = ImageBlob::new("image/png", png);
        let key = blob.content_key();
        self.doc.images.entry(key.clone()).or_insert(blob);
        let gs = self.gs.clone();
        let im = ImageObject { key, width: w as u32, height: h as u32, xf, link: None, placement: Default::default() };
        let mut node = self.new_node(NodeKind::Image(im), &gs);
        node.opacity = gs.opacity;
        node.blend = gs.blend;
        self.add(node, false)
    }

    // ---- the document ----------------------------------------------------------------------

    /// The artboard of a file without artboards or crop marks: its size around the template
    /// box's centre (half a point off the grid).
    fn sized_board(&self) -> Option<Rect> {
        let ((w, h), (cx, cy)) = (self.art_size?, self.template_center?);
        let (x0, y1) = ((cx - w / 2.0).floor(), (cy + h / 2.0).ceil());
        let r = Rect::new(x0, y1 - h, x0 + w, y1);
        (sane(r) && w > 0.0 && h > 0.0).then_some(r)
    }

    /// Art space → the document (fixed when the art starts, from the artboards read by then).
    fn ensure_space(&mut self) -> Affine {
        if let Some(m) = self.to_doc {
            return m;
        }
        let first = self.artboards.first().map(|(_, r)| *r).or(self.cropmarks).or_else(|| self.sized_board());
        let top_left = first.map_or(Point::new(0.0, 792.0), |r| Point::new(r.x0, r.y1));
        let m = Affine::new([1.0, 0.0, 0.0, -1.0, -top_left.x, top_left.y]);
        self.to_doc = Some(m);
        m
    }

    fn warn(&mut self, w: &str) {
        if !self.warnings.iter().any(|x| x == w) && self.warnings.len() < 64 {
            self.warnings.push(w.to_string());
        }
    }

    fn finish(mut self) -> Result<Structure, String> {
        if !self.unknown.is_empty() {
            let ops: Vec<String> = self.unknown.iter().map(|o| format!("`{o}`")).collect();
            self.unsupported.insert(format!("operators VectorCraft doesn't know ({})", ops.join(", ")));
        }
        if !self.unsupported.is_empty() {
            let list: Vec<&str> = self.unsupported.iter().map(String::as_str).collect();
            return Err(format!("it has {}, which VectorCraft doesn't read from it yet", join_and(&list)));
        }
        if self.doc.layers.is_empty() {
            return Err("it has no layers".into());
        }
        let to_doc = self.ensure_space();
        let artboard = self.artboards.first().map(|(_, r)| *r).or(self.cropmarks).or_else(|| self.sized_board());
        drop_commented_copies(&mut self.doc.layers, &self.hidden_ids);
        let mut doc = std::mem::replace(&mut self.doc, Document::new(1.0, 1.0));
        let boards: Vec<(String, Rect)> = if self.artboards.is_empty() {
            self.cropmarks.or_else(|| self.sized_board()).map(|r| vec![("Artboard 1".to_string(), r)]).unwrap_or_default()
        } else {
            self.artboards.clone()
        };
        doc.artboards = boards
            .into_iter()
            .enumerate()
            .map(|(i, (name, r))| Artboard {
                id: i as u32 + 1,
                name,
                rect: to_doc.transform_rect_bbox(r),
                show_center_mark: false,
                show_cross_hairs: false,
            })
            .collect();
        if doc.artboards.is_empty() {
            let rect = doc.art_bounds().filter(|r| sane(*r) && r.area() > 0.0).unwrap_or(Rect::new(0.0, 0.0, 612.0, 792.0));
            doc.artboards = vec![Artboard { id: 1, name: "Artboard 1".into(), rect, show_center_mark: false, show_cross_hairs: false }];
        }
        if self.color_model.unwrap_or(self.cmyk_colors > self.rgb_colors) {
            doc.color_mode = ColorMode::Cmyk;
            (doc.swatches, doc.swatch_groups) = vectorcraft_color::default_swatches(ColorMode::Cmyk.model());
        }
        for n in &self.spots {
            if doc.swatch(&n.name).is_none() {
                doc.swatches.push(Swatch { name: n.name.clone(), paint: Paint::solid(n.color), global: true, spot: true });
            }
        }
        let mut warnings = self.warnings;
        if self.drawn_looks > 0 {
            warnings.push(format!(
                "objects with several fills or strokes, effects or brushes came in as their drawn look ({}): their appearance isn't editable yet",
                self.drawn_looks
            ));
        }
        Ok(Structure {
            doc,
            to_doc,
            bbox: self.hires.or(self.bbox),
            artboard,
            template: self.template,
            hidden_unread: self.hidden_unread,
            slot_names: self.slot_names,
            warnings,
        })
    }
}

/// The text objects written twice, plainly and on `%_` lines: the commented copy (`hidden`) of a
/// story that also has a plain one goes.
fn drop_commented_copies(layers: &mut [Arc<Node>], hidden: &HashSet<NodeId>) {
    fn plain(nodes: &[Arc<Node>], hidden: &HashSet<NodeId>, out: &mut HashSet<u32>) {
        for n in nodes {
            match slot_of(n.name.as_deref()) {
                Some(Some(story)) if !hidden.contains(&n.id) => {
                    out.insert(story);
                }
                _ => {
                    if let Some(c) = n.children() {
                        plain(c, hidden, out);
                    }
                }
            }
        }
    }
    fn drop(nodes: &mut Vec<Arc<Node>>, hidden: &HashSet<NodeId>, plain: &HashSet<u32>) {
        nodes.retain(|n| !(hidden.contains(&n.id) && slot_of(n.name.as_deref()).flatten().is_some_and(|s| plain.contains(&s))));
        for n in nodes.iter_mut() {
            if n.children().is_some_and(|c| !c.is_empty())
                && let Some(c) = Arc::make_mut(n).children_mut()
            {
                drop(c, hidden, plain);
            }
        }
    }
    let mut stories = HashSet::new();
    plain(layers, hidden, &mut stories);
    if stories.is_empty() {
        return;
    }
    for l in layers.iter_mut() {
        if let Some(c) = Arc::make_mut(l).children_mut() {
            drop(c, hidden, &stories);
        }
    }
}

/// A node and its index among its siblings.
type Indexed = (usize, Node);

/// Close the subpath `bp` ends with, if it has one open.
fn close(bp: &mut BezPath) {
    if bp.elements().last().is_some_and(|e| !matches!(e, PathEl::ClosePath)) {
        bp.close_path();
    }
}

/// The fill rule of a path or compound node.
fn path_rule(n: &Node) -> FillRule {
    match &n.kind {
        NodeKind::Path { rule, .. } | NodeKind::Compound { rule, .. } => *rule,
        _ => FillRule::NonZero,
    }
}

/// Remove from `n` the children read from `%_` lines (`hidden`: the object itself, not its look).
fn strip(n: &mut Node, hidden: &HashSet<NodeId>) {
    if let Some(children) = n.children_mut() {
        children.retain(|c| !hidden.contains(&c.id));
        for c in children.iter_mut() {
            strip(Arc::make_mut(c), hidden);
        }
    }
}

/// The dictionary of entry `key` in `o` or the dictionaries it holds.
fn find_entry<'o>(o: &'o Obj, key: &str, depth: usize) -> Option<&'o Obj> {
    if depth > 8 {
        return None;
    }
    o.obj(key).or_else(|| o.entries.iter().flat_map(|(_, v)| v.iter()).filter_map(V::obj).find_map(|c| find_entry(c, key, depth + 1)))
}

fn sane(r: Rect) -> bool {
    [r.x0, r.y0, r.x1, r.y1].iter().all(|v| v.is_finite() && v.abs() <= MAX_SIDE)
}

/// The four numbers of a comment's value (a box that may be a point).
fn four(v: &str) -> Option<[f64; 4]> {
    let n: Vec<f64> = v.split_whitespace().map_while(|w| w.parse::<f64>().ok()).collect();
    let [a, b, c, d] = n.as_slice() else { return None };
    [a, b, c, d].iter().all(|v| v.is_finite() && v.abs() <= MAX_SIDE).then_some([*a, *b, *c, *d])
}

/// The first two numbers of a comment's value.
fn two(v: &str) -> Option<(f64, f64)> {
    let n: Vec<f64> = v.split_whitespace().filter_map(|w| w.parse().ok()).filter(|v: &f64| v.is_finite()).collect();
    match n.as_slice() {
        [a, b, ..] => Some((*a, *b)),
        _ => None,
    }
}

/// A CMYK sample in RGB.
fn cmyk_rgb([c, m, y, k]: [u8; 4]) -> [u8; 3] {
    let f = |v: u8| f32::from(v) / 255.0;
    let rgb = vectorcraft_color::Color::cmyk(f(c), f(m), f(y), f(k)).to_rgba8(1.0);
    [rgb[0], rgb[1], rgb[2]]
}

/// `a`, `a and b`, `a, b and c`.
fn join_and(items: &[&str]) -> String {
    match items {
        [] => String::new(),
        [one] => (*one).to_string(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}
