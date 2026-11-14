//! The program's art and swatches (see [`super`] for the form).

use std::collections::{HashMap, HashSet};
use std::fmt::Write;
use std::sync::Arc;

use vectorcraft_color::cms::Cms;
use vectorcraft_color::swatch::{REGISTRATION, Swatch};
use vectorcraft_color::{BlendMode, Color, Paint};
use vectorcraft_doc::{AppearanceItem, Document, FillLayer, Knockout, LayerColor, Node, NodeId, NodeKind, OpacityMask, StrokeLayer, TextWrap};
use vectorcraft_effects::stroke;
use vectorcraft_geom::{Affine, FillRule, PathData, Shape, SubPath};
use vectorcraft_pathops::BoolOp;

use crate::ai_ops::{self, unit};
use crate::ps;

mod brushes;
mod envelopes;
mod images;
mod paints;
mod styles;
mod symbols;
mod texts;
mod variables;

#[cfg(test)]
pub(super) use brushes::BRUSHES;
#[cfg(test)]
pub(super) use envelopes::ENVELOPES;
pub(super) use styles::{LARGE_CANVAS_STYLES, style_links, styled_objects};
#[cfg(test)]
pub(super) use styles::{PANEL_EMPTY, PANEL_STYLES, STYLE_LINKS, same_appearance};

pub(super) use images::PAYLOAD;
#[cfg(test)]
pub(super) use images::{IMAGE_DEPTH, IMAGES, LINKS_EMBEDDED};
#[cfg(test)]
pub(super) use paints::{GRADIENT_HUE, GRADIENT_MIX, GRADIENT_SHAPES, GRADIENTS, STOP_LINKS, SWATCH_PLACEMENT, free_gradient_name};
#[cfg(test)]
pub(super) use symbols::{MISSING_SYMBOL, SYMBOL_ART, SYMBOL_COUNT, SYMBOL_OPTIONS};
#[cfg(test)]
pub(super) use texts::TYPE_ATTRIBUTES;
#[cfg(test)]
pub(super) use variables::{ACTIVE, BINDINGS, LIMITS, VALUES};

/// How deep symbols, live objects and pattern tiles may nest in one another (deeper art is left
/// out). Groups and layers need no limit of their own: a document is never deeper than
/// [`vectorcraft_doc::MAX_DEPTH`].
pub(super) const MAX_NEST: u32 = 8;

pub(super) const TYPE: &str = "type in opacity masks, pattern tiles, symbols and art cut to a clipping object, mirrored type, area type and type on a path that is turned or unevenly scaled or whose frame or path has several parts, area type in an open frame, type on an open path whose ends meet, area type that flows around a text wrap, type of only white space, threads of type on a path, threads whose frames differ in orientation or hold a frame written as outlines, type of sizes past what the reader reads, stories past what the reader reads (1,048,576 bytes a story, 16,777,216 bytes in all, and the size of text document it reads), and type with inline graphics, ruby, gradient or pattern paint, or fills, strokes, effects or an opacity mask of the object's own, is written as its outlines in the editing data for other apps";
pub(super) const PATTERNS: &str = "pattern fills whose tiles aren't a grid or whose art reaches past the tile, pattern fills in another pattern's tile or in a symbol, and pattern fills nested more than 8 deep are written as their tiles cut to the shape in the editing data for other apps";
pub(super) const PATTERN_TILES: &str = "pattern fills with more tiles than a fill places are written filled gray in the editing data for other apps";
pub(super) const CLIPPING: &str = "clipping groups and clipping layers whose clipping object isn't a path, and the clipping groups inside them, are written as their art cut to that object in the editing data for other apps";
pub(super) const CLIP_PAINT: &str = "the fills and strokes of clipping paths that a plain fill and line can't draw are written as paths of their own inside the clipping (fills under the clipped art, strokes over it) in the editing data for other apps";
pub(super) const CLIP_PAINT_MASKS: &str = "the opacity masks of clipping paths whose fills and strokes are written as paths of their own stay on the clipping path, not on those paths, in the editing data for other apps";
pub(super) const LIVE: &str = "blends, meshes, repeats, placed documents and intertwines are written as their art in the editing data for other apps";
pub(super) const GRAPHS: &str = "graphs are written as their art, without their data or settings, in the editing data for other apps";
pub(super) const STROKES: &str = "strokes a plain line can't draw (inside or outside the path, variable width, dots, dashes fitted to corners, gradients, patterns, arrowheads) are written as filled outlines, and brushed strokes as their brush's art, in the editing data for other apps";
pub(super) const ITEM_TRANSPARENCY: &str =
    "the opacity and blending modes of single fills and strokes are left out of the editing data for other apps";
pub(super) const LAYER_MASKS: &str = "opacity masks on layers are left out of the editing data for other apps";
pub(super) const DEEP_MASKS: &str =
    "opacity masks more than 8 deep in opacity masks, symbols, live objects and pattern tiles are left out of the editing data for other apps";
const GROUPED_LAYER: &str =
    "the transparency and opacity masks of layers that an effect made into a group are left out of the editing data for other apps";
pub(super) const EFFECTS: &str = "live effects other than geometry effects are left out of the editing data for other apps";
pub(super) const SWATCHES: &str = "freeform gradient swatches, gradient swatches of more than 256 stops, and pattern swatches whose patterns the data can't define, are left out of the editing data for other apps";
pub(super) const SYMBOL_LIMIT: &str = "symbol instances past 16,000,000 objects of symbol art in all are left out of the editing data for other apps";
pub(super) const OBJECT_LIMIT: &str = "art past 4,000,000 objects in all is left out of the editing data for other apps";
const TEMPLATES: &str = "template layers are written as dimmed layers that don't print in the editing data for other apps";
pub(super) const DYNAMIC_TEXT: &str = "Dynamic Text is written as the sizes the type is drawn at in the editing data for other apps";

/// The transparency `mode opacity isolate knockout shape Xy` sets: the blending mode ([`blend_code`]),
/// the opacity, Isolate Blending, Knockout Group (0 off, 1 on, 2 neutral) and Opacity & Mask Define
/// Knockout Shape. It stays set for the objects that follow, as the lock and hidden state do; a
/// group, a clipping group or a layer takes it after its end.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Xy {
    blend: BlendMode,
    opacity: f32,
    isolate: bool,
    knockout: Knockout,
    shape: bool,
}

impl Xy {
    /// The state before any `Xy`, and an opaque path's.
    const OPAQUE: Xy = Xy { blend: BlendMode::Normal, opacity: 1.0, isolate: false, knockout: Knockout::Off, shape: false };
    /// A group's or a layer's without transparency of its own (no `Xy` after its end).
    const PLAIN: Xy = Xy { knockout: Knockout::Neutral, ..Self::OPAQUE };

    /// `n`'s transparency.
    fn of(n: &Node) -> Xy {
        let opacity = if n.opacity.is_finite() { n.opacity.clamp(0.0, 1.0) } else { 1.0 };
        Xy { blend: n.blend, opacity, isolate: n.isolate, knockout: n.knockout, shape: n.knockout_shape }
    }

    /// As a path's own: isolation and knockout are a group's.
    fn object(self) -> Xy {
        Xy { isolate: false, knockout: Knockout::Off, ..self }
    }
}

/// The number `Xy` gives blending mode `b`.
fn blend_code(b: BlendMode) -> u8 {
    match b {
        BlendMode::Normal => 0,
        BlendMode::Multiply => 1,
        BlendMode::Screen => 2,
        BlendMode::Overlay => 3,
        BlendMode::SoftLight => 4,
        BlendMode::HardLight => 5,
        BlendMode::ColorDodge => 6,
        BlendMode::ColorBurn => 7,
        BlendMode::Darken => 8,
        BlendMode::Lighten => 9,
        BlendMode::Difference => 10,
        BlendMode::Exclusion => 11,
        BlendMode::Hue => 12,
        BlendMode::Saturation => 13,
        BlendMode::Color => 14,
        BlendMode::Luminosity => 15,
    }
}

/// What the writer had charged when a part of the art started ([`Writer::charges`]).
#[derive(Clone, Copy)]
struct Charges {
    objects: usize,
    symbol_nodes: u64,
    kept_objects: usize,
    kept_symbol_nodes: u64,
    left_out: usize,
}

pub(super) struct Writer<'a> {
    doc: &'a Document,
    /// The document → art space.
    to_art: Affine,
    cms: Arc<Cms>,
    pub(super) body: String,
    pub(super) warnings: Vec<String>,
    /// The lock and hidden state (`A`, `Xw`) the last operators set; `None` at a layer's start.
    state: Option<(bool, bool)>,
    /// The transparency the last `Xy` set; `None` when it isn't known (after an opacity mask, whose
    /// art's last `Xy` stays set).
    gs: Option<Xy>,
    /// Where the state block of the group ended last starts and ends in [`Self::body`].
    block_start: usize,
    block_end: usize,
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
    /// The objects written so far, counted as the reader counts the objects it reads.
    objects: usize,
    /// How many times art was left out for [`Self::max_objects`].
    left_out: usize,
    /// The objects of the pattern and symbol definitions written so far, which stay written when
    /// the art that first used them is not ([`Self::give_back`]).
    definition_objects: usize,
    /// The objects of symbol art in those definitions ([`Self::symbol_nodes`]).
    definition_symbol_nodes: u64,
    /// The most objects written ([`Self::take_objects`]): the most the reader reads,
    /// [`crate::import::MAX_EDITING_OBJECTS`].
    pub(super) max_objects: usize,
    /// The document → the space gradients' matrices (`Xm`, `Bm`, `Bc`) and focal points are
    /// written in: art space, and in a pattern's definition [`Self::tile_grad_space`].
    grad_space: Affine,
    /// The pattern's space (y down) → the space a pattern's definition writes gradients' matrices
    /// in: the canvas's (y down), from art space's origin there.
    tile_grad_space: Affine,
    /// The document → the canvas (y down), the space of a gradient's transformation in `Bg`; in a
    /// pattern's definition, the pattern's space turned y up.
    canvas: Affine,
    /// The gradient definitions, in the order they are first used.
    gradients: Vec<paints::GradientDef>,
    /// The number the next new gradient definition's name tries first (1 is `Gradient`).
    next_gradient: u64,
    /// The pattern definitions written, each after those its art uses.
    patterns: Vec<paints::PatternOut>,
    /// The patterns whose definitions are being written.
    defining: Vec<String>,
    /// Writing the palette: the definitions it adds are for its swatches only.
    in_palette: bool,
    /// Art space's origin on the canvas (y down), where a placed file's box starts.
    canvas_origin: (f64, f64),
    /// The binary parts of the program (images' samples, placed files' previews), each in the
    /// text as a mark ([`PAYLOAD`], its index, [`PAYLOAD`]).
    pub(super) payloads: Vec<Vec<u8>>,
    /// How many parts of the data written on `%_` lines (opacity masks' art, patterns' tiles) the
    /// art being written is in.
    hidden: u32,
    /// The `hidden` and `nest` values at which the objects of the opacity mask's art being written
    /// are written: objects there may be written with their art styles.
    mask_level: Option<(u32, u32)>,
    /// The image just written was written as a placed file.
    placed: bool,
    /// The type just written was written as type, not as its outlines.
    typed: bool,
    /// The objects bound to variables so far, in the order they are written: each one's id, its XML
    /// id and the variables it is bound to ([`Self::bound_variables`]).
    bound: Vec<(vectorcraft_doc::NodeId, String, Vec<String>)>,
    /// The text document: the stories of the type written as text.
    pub(super) texts: texts::TextDoc,
    /// Symbol instances on the layers may be written as instances ([`Self::live_instance`]); false
    /// on a large canvas, where they are written as their art.
    pub(super) symbols_live: bool,
    /// The document symbols are defined from: on a large canvas, the document at its own size
    /// (symbols' art is in its own space, which the canvas's scale doesn't change).
    pub(super) symbol_doc: &'a Document,
    /// A symbol's art is being written ([`Self::define_symbol`]).
    in_symbol: bool,
    /// The document → the space an instance's `RTransformMatrix` takes its symbol's art to: art
    /// space, and in a symbol's art the canvas's turned y up about art space's origin there.
    instance_space: Affine,
    /// The symbol definitions written (`%AI14_BeginSymbol` … `%AI10_EndSymbol`), each after those
    /// its art uses, and their names.
    symbol_defs: Vec<String>,
    defined_symbols: HashSet<String>,
    /// The symbols whose definitions are being written.
    defining_symbols: Vec<String>,
    /// Whether each symbol's own art paints with a pattern, and the symbols its instances use
    /// ([`Self::symbol_own_patterns`]).
    pattern_symbols: HashMap<String, (bool, Vec<String>)>,
    /// Whether each symbol's art paints with a pattern, nested symbols included
    /// ([`Self::symbol_paints_pattern`]).
    patterned_symbols: HashMap<String, bool>,
    /// Pattern tiles were written in the art being written ([`Self::define_symbol`] starts it over
    /// for each symbol's art).
    wrote_pattern: bool,
    /// The symbols whose definitions hold pattern tiles: a brushed stroke's brush art or a live
    /// object's art can paint with a pattern that the symbol's own objects don't name.
    pattern_written: HashSet<String>,
    /// Art that draws as it should only where it is was written in the art being written
    /// ([`Self::define_symbol`] starts it over for each symbol's art): a stroke as its outline or its
    /// brush's art, type as its outlines, a live object or a symbol instance as its art, or a shape
    /// with rounded live corners.
    wrote_fixed: bool,
    /// The symbols whose definitions hold such art, nested symbols' included
    /// ([`Self::keeps_scale`]).
    fixed_symbols: HashSet<String>,
    /// The largest scale a live instance in the art being written applies, nested symbols' scales
    /// included, and that of each symbol's definition.
    inner_scale: f64,
    symbol_scales: HashMap<String, f64>,
    /// The entries a symbol's dictionary adds to its art's dictionary, and the art's id.
    root_entries: Option<(vectorcraft_doc::NodeId, String)>,
    /// The objects that may be written with an art style, as they were before baking, by id
    /// ([`styles::styled_objects`]).
    pub(super) styled: HashMap<NodeId, Arc<Node>>,
    /// The art styles written ([`styles`]).
    styles: styles::Styles,
    /// An object's drawn look is being written: no object in it is written with a style, and its
    /// effects are the style's.
    in_look: bool,
    /// How many parts of art drawn for an object (a live object's art, a symbol instance's art, a
    /// brushed stroke's pieces) the art being written is in. Text wrap options are written only
    /// outside them, since only the document's own objects wrap type.
    drawn: u32,
    /// The brushes written ([`brushes`]).
    brush_set: brushes::Brushes,
    /// Brushes may be written; false on a large canvas, where they are left out.
    pub(super) brushes_live: bool,
    /// The object just written is an envelope's distorted art, which the plugin's `XP` follows
    /// ([`Self::envelope`]).
    deform: bool,
    /// How many more envelopes may be written live: as many as the reader checks in one file
    /// ([`crate::import::ENVELOPE_CHECKS`]).
    envelopes_left: usize,
}

impl<'a> Writer<'a> {
    /// A writer of `doc`'s art through `to_art`; `origin` is art space's origin on the canvas.
    pub(super) fn new(doc: &'a Document, to_art: Affine, origin: (f64, f64)) -> Self {
        Self {
            doc,
            to_art,
            grad_space: to_art,
            tile_grad_space: Affine::translate((-origin.0, origin.1)),
            canvas: Affine::new([1.0, 0.0, 0.0, -1.0, origin.0, origin.1]) * to_art,
            gradients: vec![],
            next_gradient: 1,
            patterns: vec![],
            defining: vec![],
            in_palette: false,
            canvas_origin: origin,
            payloads: vec![],
            hidden: 0,
            mask_level: None,
            placed: false,
            typed: false,
            bound: vec![],
            texts: texts::TextDoc::default(),
            cms: vectorcraft_color::cms::active(),
            body: String::new(),
            warnings: vec![],
            state: None,
            gs: Some(Xy::OPAQUE),
            block_start: 0,
            block_end: usize::MAX,
            clip: None,
            nest: 0,
            ids: HashSet::new(),
            next: HashMap::new(),
            brushes: None,
            symbol_nodes: 0,
            symbols_live: true,
            symbol_doc: doc,
            in_symbol: false,
            instance_space: to_art,
            symbol_defs: vec![],
            defined_symbols: HashSet::new(),
            defining_symbols: vec![],
            pattern_symbols: HashMap::new(),
            patterned_symbols: HashMap::new(),
            wrote_pattern: false,
            pattern_written: HashSet::new(),
            wrote_fixed: false,
            fixed_symbols: HashSet::new(),
            inner_scale: 1.0,
            symbol_scales: HashMap::new(),
            root_entries: None,
            styled: HashMap::new(),
            styles: styles::Styles::default(),
            in_look: false,
            drawn: 0,
            brush_set: brushes::Brushes::default(),
            brushes_live: true,
            deform: false,
            envelopes_left: crate::import::ENVELOPE_CHECKS,
            objects: 0,
            left_out: 0,
            definition_objects: 0,
            definition_symbol_nodes: 0,
            max_objects: crate::import::MAX_EDITING_OBJECTS,
        }
    }

    /// Whether `count` more objects may be written: the objects written so far, counted as the
    /// reader counts them ([`object_count`]), stay within [`Self::max_objects`]. Past it the art is
    /// left out, with a warning.
    fn take_objects(&mut self, count: usize) -> bool {
        let total = self.objects.saturating_add(count);
        if total > self.max_objects {
            self.warn(OBJECT_LIMIT);
            self.left_out += 1;
            return false;
        }
        self.objects = total;
        true
    }

    /// Start a group with `op` (`u`, or `q` for a clipping group), counted as one object
    /// ([`Self::take_objects`]). False, writing nothing, when there is no room for it.
    fn open_group(&mut self, op: &str) -> bool {
        if !self.take_objects(1) {
            return false;
        }
        self.line(op);
        true
    }

    /// Whether `n`, an instance of `symbol` placed by `xf`, is written as a live instance
    /// ([`Self::live_instance`]). The instance is one object, without the art of its definition,
    /// and room for it is kept before the definition is written.
    fn live_instance_in_room(&mut self, n: &Node, symbol: &str, xf: Affine) -> bool {
        if !self.take_objects(1) {
            return false;
        }
        if self.live_instance(n, symbol, xf) {
            return true;
        }
        self.objects -= 1;
        false
    }

    /// What has been charged so far, for [`Self::keep_since`] and [`Self::give_back`].
    fn charges(&self) -> Charges {
        Charges {
            objects: self.objects,
            symbol_nodes: self.symbol_nodes,
            kept_objects: self.definition_objects,
            kept_symbol_nodes: self.definition_symbol_nodes,
            left_out: self.left_out,
        }
    }

    /// Whether art was left out for [`Self::max_objects`] since `c`.
    fn left_out_since(&self, c: Charges) -> bool {
        self.left_out != c.left_out
    }

    /// A definition that started at `c` is written: what was charged since stays charged when the
    /// art that first used it is given back ([`Self::give_back`]).
    fn keep_since(&mut self, c: Charges) {
        self.definition_objects = c.kept_objects + (self.objects - c.objects);
        self.definition_symbol_nodes = c.kept_symbol_nodes + (self.symbol_nodes - c.symbol_nodes);
    }

    /// Art written since `c` as `lines` is not used: the objects and symbol art charged since are
    /// given back, except those of the definitions written since, and the images `lines` marks
    /// are emptied (the others keep their indices).
    fn give_back(&mut self, c: Charges, lines: &str) {
        self.objects = c.objects + (self.definition_objects - c.kept_objects);
        self.symbol_nodes = c.symbol_nodes + (self.definition_symbol_nodes - c.kept_symbol_nodes);
        for index in lines.split(PAYLOAD).skip(1).step_by(2) {
            if let Some(p) = index.parse::<usize>().ok().and_then(|i| self.payloads.get_mut(i)) {
                *p = Vec::new();
            }
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
        self.effects(n);
        match members.iter().find(|c| c.is_layer()) {
            Some(l) => {
                if Xy::of(n) != Xy::PLAIN || n.extras().mask.is_some() {
                    self.warn(GROUPED_LAYER);
                }
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

    /// A layer or sublayer: `%AI5_BeginLayer`, its options `Lb`, its name `Ln` and as an XML id, its art (its
    /// clipping path last, with `W`, when it clips), `after` (outside the clipping path), its transparency
    /// (`Xy`, `0 0 Xd`, `7 () XW`) when it has any, `LB`, `%AI5_EndLayer--`.
    fn write_layer(&mut self, n: &Node, after: &[Arc<Node>]) {
        let NodeKind::Layer { color, template, printable, children, clip, preview, dim_images } = &n.kind else { return };
        let (index, [r, g, b]) = ai_ops::layer_color(*color);
        if *template {
            self.warn(TEMPLATES);
        }
        self.effects(n);
        if n.extras().mask.is_some() {
            self.warn(LAYER_MASKS);
        }
        let flag = |b: bool| u8::from(b);
        let dim = dim_images.unwrap_or(50);
        let name = n.name.as_deref().unwrap_or("Layer");
        // Ruler guides are kept in the document's native copy only.
        let children: Vec<Arc<Node>> = children.iter().filter(|c| !c.is_ruler_guide()).cloned().collect();
        let children = &children;
        // The last flag makes the layer a clipping layer: its top object clips the rest. Only a
        // clipping path or compound path, outside any art cut to a clipping object, is written as
        // one ([`Self::clip_group`] writes the others).
        let clips = *clip && after.is_empty() && self.clip.is_none() && children.first().is_some_and(|c| clip_path(c).is_some());
        let _ = writeln!(
            self.body,
            "%AI5_BeginLayer\n{} {} {} {} {} 0 1 {index} {r} {g} {b} 0 {dim} {} Lb\n{} Ln",
            flag(n.visible),
            flag(*preview),
            flag(!n.locked),
            flag(*printable && !*template),
            flag(dim_images.is_some() || *template),
            flag(clips),
            ps::bytes(name.as_bytes()),
        );
        self.xml_id(if name.is_empty() { "Layer" } else { name });
        self.state = None;
        if clips {
            self.clipped(children);
        } else if *clip && !children.is_empty() {
            // The added art lies outside the clipping path, or the clipping object isn't a path:
            // the clipped art is a clipping group, or a group of the art cut to the object.
            self.clip_group(children, Xy::PLAIN);
        } else {
            for c in children {
                self.node(c);
            }
        }
        for c in after {
            self.node(c);
        }
        let xy = Xy::of(n);
        if xy != Xy::PLAIN {
            // Always written: the layer takes the `Xy` its own state block has.
            self.write_xy(xy);
            self.line("0 0 Xd\n7 () XW");
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

    /// Set the transparency of the object that follows to `xy` (`Xy`, when it isn't set already).
    fn set_xy(&mut self, xy: Xy) {
        if self.gs != Some(xy) {
            self.write_xy(xy);
        }
    }

    /// [`Self::set_xy`] for an object that writes no paint before its transparency (type, an image,
    /// a guide): `XR` comes first, as in [`Self::paint_state`], because an `Xy` right after a compound
    /// path or a text object would be theirs.
    fn set_object_xy(&mut self, xy: Xy) {
        if self.gs != Some(xy) {
            self.line("0 XR");
            self.write_xy(xy);
        }
    }

    /// `mode opacity isolate knockout shape Xy`.
    fn write_xy(&mut self, xy: Xy) {
        let knockout = match xy.knockout {
            Knockout::Off => 0,
            Knockout::On => 1,
            Knockout::Neutral => 2,
        };
        let _ = write!(self.body, "{} ", blend_code(xy.blend));
        ps::push_num(&mut self.body, f64::from(xy.opacity));
        let _ = writeln!(self.body, " {} {knockout} {} Xy", u8::from(xy.isolate), u8::from(xy.shape));
        self.gs = Some(xy);
    }

    /// The end `close` (`U` or `Q`) of a group of transparency `xy`, then its state block: when it
    /// has transparency of its own, `Xy`, `0 0 Xd` and `6 () XW`; else `9 () XW`. The group's
    /// dictionary goes between the two ([`Self::node`]).
    fn end_group(&mut self, close: &str, xy: Xy) {
        self.line(close);
        self.state_block(xy);
    }

    /// The state block of the group or text object just written ([`Self::end_group`]).
    fn state_block(&mut self, xy: Xy) {
        self.block_start = self.body.len();
        if xy == Xy::PLAIN {
            self.line("9 () XW");
        } else {
            self.write_xy(xy);
            self.line("0 0 Xd\n6 () XW");
        }
        self.block_end = self.body.len();
    }

    /// Warn when `n` has effects the data leaves out.
    fn effects(&mut self, n: &Node) {
        if self.in_look {
            return;
        }
        let items = n.appearance.items.iter().filter(|i| i.visible()).flat_map(|i| i.effects());
        if n.appearance.effects.iter().chain(items).any(|e| e.visible) {
            self.warn(EFFECTS);
        }
    }

    /// One object: its state before it, its transparency, and its name and opacity mask after it.
    fn node(&mut self, n: &Node) {
        if let NodeKind::Layer { .. } = n.kind {
            return self.write_layer(n, &[]);
        }
        // Ruler guides are kept in the document's native copy only: no object, state or style.
        if n.is_ruler_guide() {
            return;
        }
        if self.in_symbol {
            self.wrote_fixed |= symbols::redrawn(n);
        }
        if n.extras().graph.is_some() && !self.in_look {
            self.warn(GRAPHS);
        }
        // On the layers or in an opacity mask's art, outside art cut to a clipping object: with its
        // art style, when it has one.
        if !self.in_look
            && ((self.hidden == 0 && self.nest == 0) || self.mask_level == Some((self.hidden, self.nest)))
            && self.clip.is_none()
            && !self.in_symbol
            && let Some(orig) = self.styled.get(&n.id).cloned()
            && let Some(name) = self.object_style(n, &orig)
        {
            return self.styled_object(n, &orig, &name);
        }
        self.effects(n);
        self.state(n.locked, !n.visible);
        let xy = Xy::of(n);
        let start = self.body.len();
        match &n.kind {
            NodeKind::Layer { .. } => {}
            NodeKind::Group { children, clip: true } => self.clip_group(children, xy),
            NodeKind::Group { children, clip: false } => {
                if self.open_group("u") {
                    for c in children {
                        self.node(c);
                    }
                    self.end_group("U", xy);
                }
            }
            NodeKind::Path { path, rule, guide, .. } => {
                if *guide {
                    // A path for each subpath with points, with the guide's own transparency, not
                    // the one the object before it set.
                    let subpaths = path.subpaths.iter().filter(|sp| !sp.anchors.is_empty()).count();
                    if subpaths > 0 && self.take_objects(subpaths) {
                        self.set_object_xy(xy.object());
                        ai_ops::guide(&mut self.body, self.to_art, path, true);
                    }
                } else {
                    self.shape(n, path, *rule, xy);
                }
            }
            NodeKind::Compound { rule, .. } => self.shape(n, &compound_path(n), *rule, xy),
            NodeKind::Text(_) => self.text(n, xy),
            NodeKind::Image(im) => self.image(&self.doc.shown_image(n, im), xy),
            NodeKind::SymbolInstance { symbol, xf } if self.live_instance_in_room(n, symbol, *xf) => self.symbol_instance(symbol, *xf, xy),
            NodeKind::SymbolInstance { symbol, xf } => {
                if !self.open_group("u") {
                    return;
                }
                // The canvas places an instance inside a symbol's art where the outer instance puts
                // it (9-slice guides, stains and patterns included), and this art doesn't move with it.
                self.wrote_fixed |= self.in_symbol;
                let doc = self.doc;
                if doc.symbols.iter().any(|s| &s.name == symbol) {
                    self.warn(symbols::SYMBOL_ART);
                } else {
                    self.warn(symbols::MISSING_SYMBOL);
                }
                if let Some(sym) = doc.symbols.iter().find(|s| &s.name == symbol)
                    && self.nest < MAX_NEST
                {
                    // Stained by the instance's fill, as the canvas draws it.
                    let stained = vectorcraft_brush::instance_art(&sym.art, n);
                    // Through the symbol's 9-slice guides when it has them, as the canvas draws it;
                    // placing through them needs room for the most objects it can make, and the
                    // objects it makes count.
                    let map = doc.symbol_slice_map(symbol, *xf);
                    let most = map.map_or(stained.count() as u64, |m| m.most_placed(&stained, doc));
                    let placed = vectorcraft_doc::charge_symbol_art(&mut self.symbol_nodes, most, || {
                        let art = match map {
                            Some(m) => m.place(&stained, doc),
                            None => {
                                let mut a = stained.clone();
                                a.transform(*xf, false);
                                a
                            }
                        };
                        let count = if map.is_some() { art.count_with_masks() } else { art.count() } as u64;
                        (art, count)
                    });
                    if let Some(mut art) = placed {
                        art.visible = true;
                        art.locked = false;
                        art.name = None;
                        self.nest += 1;
                        self.drawn += 1;
                        self.node(&art);
                        self.drawn -= 1;
                        self.nest -= 1;
                    } else {
                        self.warn(SYMBOL_LIMIT);
                    }
                }
                self.end_group("U", xy);
            }
            NodeKind::Envelope { .. } if self.envelope(n, xy) => {}
            NodeKind::Envelope { .. } => self.drawn_art(n, xy, Some(envelopes::ENVELOPES)),
            NodeKind::Blend { .. } | NodeKind::Mesh(_) | NodeKind::Repeat(_) | NodeKind::PlacedDocument(_) | NodeKind::Intertwine { .. } => {
                self.drawn_art(n, xy, Some(LIVE));
            }
            // Ruler guides are kept in the document's native copy only.
            NodeKind::RulerGuide { .. } => {}
        }
        // Taken before the dictionary, whose opacity mask art may hold envelopes of its own.
        let deform = std::mem::take(&mut self.deform);
        if self.body.len() > start {
            // A group's dictionary goes between its end and its state block.
            let block = (self.block_end == self.body.len() && self.block_start >= start).then(|| self.body.split_off(self.block_start));
            self.block_end = usize::MAX;
            self.dictionary(n);
            if let Some(b) = block {
                self.body.push_str(&b);
            }
        }
        // An envelope's plugin `XP` follows its distorted art, after the art's state block.
        if deform {
            self.line(&envelopes::plugin_line());
        }
    }

    /// Live object `n`, of transparency `xy`, as a group of its art, with `warning` when it is given.
    fn drawn_art(&mut self, n: &Node, xy: Xy, warning: Option<&str>) {
        if !self.open_group("u") {
            return;
        }
        if let Some(w) = warning {
            self.warn(w);
        }
        if self.nest < MAX_NEST {
            let g = vectorcraft_effects::expand_live_deep(Some(self.doc), n);
            self.nest += 1;
            self.drawn += 1;
            for c in g.children().map(Vec::as_slice).unwrap_or_default() {
                self.node(c);
            }
            self.drawn -= 1;
            self.nest -= 1;
        }
        self.end_group("U", xy);
    }

    /// `n`'s `/ArtDictionary`, after it, when it has a name (`AI10_ArtUID`, see [`Self::take_id`]),
    /// is bound to variables ([`Self::bound_variables`]; without a name, it gets an XML id and an
    /// empty `AIArtName`), has text wrap options ([`Self::text_wrap`]), has an opacity mask
    /// (`AI9_Mask`, see [`Self::mask`]), or is a symbol's art (its symbol's entries first, see
    /// [`Self::define_symbol`]).
    fn dictionary(&mut self, n: &Node) {
        let name = n.name.as_deref().filter(|s| !s.is_empty());
        let wrap = n.extras().wrap.filter(|_| self.drawn == 0 && !self.in_look);
        let placed = std::mem::take(&mut self.placed);
        let typed = std::mem::take(&mut self.typed);
        let bound = self.bound_variables(n, placed, typed);
        let mut mask = n.extras().mask.as_deref();
        // Masks nested too deep are left out (an empty mask would hide the object).
        if mask.is_some() && self.nest >= MAX_NEST {
            self.warn(DEEP_MASKS);
            mask = None;
        }
        let entries = match &self.root_entries {
            Some((id, _)) if *id == n.id => self.root_entries.take().map(|(_, e)| e),
            _ => None,
        };
        if name.is_none() && mask.is_none() && bound.is_empty() && entries.is_none() && wrap.is_none() {
            return;
        }
        let start = self.body.len();
        let has_id = name.is_some() || !bound.is_empty();
        let has_entries = entries.is_some();
        self.body.push_str("%_/ArtDictionary :\n");
        if let Some(e) = entries {
            self.body.push_str(&e);
        }
        if has_id {
            let id = self.take_id(name.unwrap_or("XMLID"));
            let _ = writeln!(self.body, "%_/XMLUID : {} ; (AI10_ArtUID) ,", ps::bytes(id.as_bytes()));
            if name.is_none() {
                // The XML id names the object unless the dictionary gives it another name.
                self.body.push_str("%_() /String (AIArtName) ,\n");
            }
            if !bound.is_empty() {
                self.bound.push((n.id, id, bound));
            }
        }
        if let Some(w) = wrap {
            self.text_wrap(w);
        }
        // A mask whose art doesn't fit is left out, and with nothing else in it the dictionary too.
        if let Some(m) = mask
            && !self.mask(m)
            && !has_id
            && !has_entries
            && wrap.is_none()
        {
            return self.body.truncate(start);
        }
        self.body.push_str("%_;\n%_\n");
    }

    /// Text wrap options, as entries of their object's dictionary: the offset in art space's points
    /// (`AI11TextWrapOffset`, `/Real`; a value that isn't finite is written as the default 6 pt,
    /// and one past the -1000 to 1000 pt the reader keeps as the nearer end of that range) and
    /// Invert Wrap (`AI11TextWrapInvert`, `/Bool`, 1 or 0), as the reader reads them back.
    fn text_wrap(&mut self, w: TextWrap) {
        let offset = if w.offset.is_finite() { w.offset.clamp(-1000.0, 1000.0) } else { TextWrap::default().offset };
        self.body.push_str("%_");
        ps::push_num(&mut self.body, offset);
        let _ = writeln!(self.body, " /Real (AI11TextWrapOffset) ,\n%_{} /Bool (AI11TextWrapInvert) ,", u8::from(w.invert));
    }

    /// An `/ArtDictionary` whose `AI10_ArtUID` is `name` as an XML id.
    fn xml_id(&mut self, name: &str) {
        let id = self.take_id(name);
        let _ = writeln!(self.body, "%_/ArtDictionary :\n%_/XMLUID : {} ; (AI10_ArtUID) ,\n%_;\n%_", ps::bytes(id.as_bytes()));
    }

    /// `name` as an XML id no layer or object has yet, numbered (`_` and 20 digits, `_`) when one
    /// has it.
    fn take_id(&mut self, name: &str) -> String {
        let base = vectorcraft_doc::xml_id::from_object_name(name);
        let mut id = base.clone();
        let next = self.next.entry(base.clone()).or_insert(0);
        while self.ids.contains(&id) {
            *next += 1;
            id = format!("{base}_{next:020}_");
        }
        self.ids.insert(id.clone());
        id
    }

    /// An opacity mask, as an entry of its object's dictionary: `/Mask :`, `X=`, its art on `%_`
    /// lines (from the state a dictionary's art starts in: unlocked, shown, opaque), `X+`, `(Art) ,`,
    /// its options as the `/Bool` entries `Clipping`, `Inverted`, `Disabled` and `Linked`, and
    /// `; (AI9_Mask) ,`. The art is in document coordinates, as the object's is. False, writing
    /// nothing, when some of the art is left out for the object limit ([`Self::take_objects`]): a
    /// mask without all its art would change what it shows.
    fn mask(&mut self, m: &OpacityMask) -> bool {
        let outer = std::mem::take(&mut self.body);
        let (state, gs, clip) = (self.state, self.gs, self.clip.take());
        let (charges, bound) = (self.charges(), self.bound.len());
        (self.state, self.gs) = (Some((false, false)), Some(Xy::OPAQUE));
        // The canvas draws the art's root even when it is hidden, and the reader puts a hidden object
        // in a group that doesn't draw it, so a hidden root is written shown.
        let shown;
        let art = if m.art.visible {
            &m.art
        } else {
            let mut a = (*m.art).clone();
            a.visible = true;
            shown = Arc::new(a);
            &shown
        };
        // A plain group of two or more objects, or of none, is the group the art is read into. A group
        // of one object is written as a group, because the reader makes one shown object the art.
        // A group that may be written with its art style is written as one object.
        let plain = matches!(art.kind, NodeKind::Group { clip: false, .. })
            && Xy::of(art) == Xy::PLAIN
            && art.visible
            && !art.locked
            && art.name.as_deref().is_none_or(str::is_empty)
            && art.extras().mask.is_none()
            && !self.styled.contains_key(&art.id);
        let members = match art.children() {
            Some(c) if plain && c.len() != 1 => c.as_slice(),
            _ => std::slice::from_ref(art),
        };
        self.nest += 1;
        self.hidden += 1;
        // The art's objects (not those inside its symbols, live objects or pattern tiles, which are
        // deeper) may be written with their art styles.
        let level = self.mask_level.replace((self.hidden, self.nest));
        for c in members {
            self.node(c);
        }
        self.mask_level = level;
        self.hidden -= 1;
        self.nest -= 1;
        let lines = std::mem::replace(&mut self.body, outer);
        if self.left_out_since(charges) {
            // The art isn't written, nor are the variable bindings of its objects.
            self.give_back(charges, &lines);
            self.bound.truncate(bound);
            (self.state, self.gs, self.clip, self.block_end) = (state, gs, clip, usize::MAX);
            return false;
        }
        self.body.push_str("%_/Mask :\n%_X=\n");
        hide(&lines, &mut self.body);
        self.body.push_str("%_X+\n%_(Art) ,\n");
        for (key, on) in [("Clipping", m.clip), ("Inverted", m.invert), ("Disabled", m.disabled), ("Linked", m.linked)] {
            let _ = writeln!(self.body, "%_{} /Bool ({key}) ,", u8::from(on));
        }
        self.body.push_str("%_; (AI9_Mask) ,\n");
        // The dictionary ends with the state before it, except the transparency its art set last.
        (self.state, self.gs, self.clip, self.block_end) = (state, None, clip, usize::MAX);
        true
    }

    /// Warn that art is cut to clipping object `first` when it isn't a path or compound path (a path
    /// without points cuts everything away, as it clips everything away).
    fn warn_cut(&mut self, first: Option<&Arc<Node>>) {
        if first.is_some_and(|c| !matches!(c.kind, NodeKind::Path { guide: false, .. } | NodeKind::Compound { .. })) {
            self.warn(CLIPPING);
        }
    }

    /// A clipping group: `q`, its members, its clipping path or compound path last (the top object:
    /// a mask shows the objects beneath it, §5.6.2) with `W`, `Q`, then the group's transparency.
    /// `Q` sets back the state at `q`. A clipping object of another kind cuts the members instead,
    /// in a group ([`Self::clip_contents`]). A clipping group without members is an empty group.
    fn clip_group(&mut self, children: &[Arc<Node>], xy: Xy) {
        if self.clip.is_some() || children.first().is_none_or(|c| clip_path(c).is_none()) {
            self.warn_cut(children.first());
            // Inside art cut to a clipping object, a clipping path cuts its art too.
            if self.clip.is_some() && !children.is_empty() {
                self.warn(CLIPPING);
            }
            if self.open_group("u") {
                self.clip_contents(children);
                self.end_group("U", xy);
            }
            return;
        }
        let at_q = (self.state, self.gs);
        if !self.open_group("q") {
            return;
        }
        self.clipped(children);
        (self.state, self.gs) = at_q;
        self.end_group("Q", xy);
    }

    /// The members of a clipping group or layer, the first one clipping the others: the others,
    /// then the first one as the clipping path (see [`Self::clip_group`]), its name after it. Fills
    /// and strokes the clipping path can't paint itself are paths of their own, the fills under the
    /// others and the strokes over them. Without room for the clipping path, nothing is written
    /// ([`Self::take_objects`]).
    fn clipped(&mut self, children: &[Arc<Node>]) {
        let Some((first, rest)) = children.split_first() else { return };
        let Some((path, rule)) = clip_path(first) else {
            self.warn_cut(Some(first));
            return self.clip_contents(children);
        };
        let items = self.items(first, &path.to_bezpath());
        let painted = self.plain_paint(&items);
        // The clipping path goes after the others, so its objects are counted first.
        let subpaths: Vec<&SubPath> = path.subpaths.iter().filter(|sp| !sp.anchors.is_empty()).collect();
        let (fill, stroke) = painted.unwrap_or_default();
        if !self.take_objects(clip_object_count(&subpaths, fill, stroke)) {
            return;
        }
        if items.iter().any(|i| i.opacity() < 1.0 || i.blend() != BlendMode::Normal) {
            self.warn(ITEM_TRANSPARENCY);
        }
        // Paint the clipping path can't carry: its fills under the others, its strokes over them.
        let (fills, strokes): (Vec<&AppearanceItem>, Vec<&AppearanceItem>) =
            if painted.is_none() { items.iter().partition(|i| matches!(i, AppearanceItem::Fill(_))) } else { (vec![], vec![]) };
        if painted.is_none() {
            self.warn(CLIP_PAINT);
            if first.extras().mask.is_some() && !items.is_empty() {
                self.warn(CLIP_PAINT_MASKS);
            }
        }
        self.clip_paint_part(first, &path, rule, &fills);
        for c in rest {
            self.node(c);
        }
        self.clip_paint_part(first, &path, rule, &strokes);
        self.state(first.locked, !first.visible);
        let start = self.body.len();
        self.paint_state(fill, stroke, Xy::of(first).object(), rule);
        let op = paint_op(fill.is_some(), stroke.is_some());
        let several = subpaths.len() > 1;
        if several {
            self.line("*u");
        }
        let last = subpaths.len().saturating_sub(1);
        let outline = path.to_bezpath();
        for (i, sp) in subpaths.into_iter().enumerate() {
            ai_ops::subpath(&mut self.body, self.to_art, sp);
            let _ = writeln!(self.body, "{}\nW", if sp.closed { 'h' } else { 'H' });
            let gradients = if i == last { (gradient(fill.map(|f| &f.paint)), gradient(stroke.map(|s| &s.paint))) } else { (None, None) };
            self.painted(op, sp.closed, gradients, stroke.is_some(), true, &outline);
        }
        if several {
            self.line("*U");
        }
        if self.body.len() > start {
            self.dictionary(first);
        }
    }

    /// The fills or the strokes `items` of clipping path `first` (outline `path`) that it can't paint
    /// itself, locked and hidden as it is, with its transparency once for all of them: one object,
    /// or a group of an object for each (as the canvas draws them, [`Node::clip_paint`]).
    fn clip_paint_part(&mut self, first: &Node, path: &PathData, rule: FillRule, items: &[&AppearanceItem]) {
        if items.is_empty() {
            return;
        }
        self.state(first.locked, !first.visible);
        let xy = Xy::of(first);
        if let [one] = items {
            return self.item(path, rule, one, xy);
        }
        if !self.open_group("u") {
            return;
        }
        for i in items {
            self.item(path, rule, i, Xy::PLAIN);
        }
        self.end_group("U", xy);
    }

    /// A container's members, the first one's outline cutting the others: its fills painted
    /// behind them (cut to the region outside too), its strokes over them, not cut by it. Its text
    /// wrap options are written once: with its fills, or with its strokes when no fill object is
    /// written (it has no fills, or the region outside leaves nothing of them).
    fn clip_contents(&mut self, children: &[Arc<Node>]) {
        let Some((first, rest)) = children.split_first() else { return };
        // Nothing to clip by hides the clipped art.
        let Some((outline, rule)) = vectorcraft_effects::clip_outline(first) else { return };
        let mut paint = first.clip_paint();
        let outer = self.clip.clone();
        let region = self.cut(&PathData::from_bezpath(&outline), rule);
        self.clip = Some(region);
        if let Some(fill) = &paint.fill {
            let objects = self.objects;
            self.node(fill);
            if self.objects > objects
                && let Some(stroke) = &mut paint.stroke
                && stroke.extras().wrap.is_some()
            {
                stroke.extras_mut().wrap = None;
            }
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

    /// A path's fills and strokes, with the path's transparency `xy`: one object with one fill and
    /// a plain stroke over it where it can be, else a group of an object for each (in paint order).
    fn shape(&mut self, n: &Node, path: &PathData, rule: FillRule, xy: Xy) {
        if path.is_empty() {
            return;
        }
        let items = self.items(n, &path.to_bezpath());
        if items.iter().any(|i| i.opacity() < 1.0 || i.blend() != BlendMode::Normal) {
            self.warn(ITEM_TRANSPARENCY);
        }
        match (self.plain_paint(&items), items.as_slice()) {
            (Some((fill, stroke)), _) => self.path_object(path, rule, fill, stroke, xy),
            (None, [one]) => self.item(path, rule, one, xy),
            (None, items) => {
                if self.open_group("u") {
                    for i in items {
                        self.item(path, rule, i, Xy::PLAIN);
                    }
                    self.end_group("U", xy);
                }
            }
        }
    }

    /// One fill or stroke of a path as an object of its own, of transparency `xy`.
    fn item(&mut self, path: &PathData, rule: FillRule, item: &AppearanceItem, xy: Xy) {
        match item {
            AppearanceItem::Fill(f) => self.fill_object(path, rule, f, xy),
            AppearanceItem::Stroke(s) if self.direct_stroke(s) => self.path_object(path, rule, None, Some(s), xy),
            AppearanceItem::Stroke(s) => {
                // In a styled path's or compound path's drawn look the stroke, brushed or not, is kept
                // by the object's art style, which is written only when the reader reads the object's
                // appearance (and its brush) back from it. A styled group's style holds no fills or
                // strokes: its members are written again after the look, and warn there.
                if !self.in_look {
                    self.warn(STROKES);
                }
                self.wrote_fixed = true;
                if self.brush_art(path, s, xy) {
                    return;
                }
                let outline = stroke::outline_region(path, rule, s);
                let fill = FillLayer { overprint: s.overprint, ..FillLayer::new(s.paint.clone()) };
                self.fill_object(&outline, FillRule::NonZero, &fill, xy);
            }
        }
    }

    /// A brushed stroke `st` of `path` as a group of its brush's art, of transparency `xy`. False
    /// when it has no brush the document knows (or brushes nest too deep). Without room for the
    /// group ([`Self::take_objects`]), the stroke is left out.
    fn brush_art(&mut self, path: &PathData, st: &StrokeLayer, xy: Xy) -> bool {
        if self.nest >= MAX_NEST {
            return false;
        }
        let doc = self.doc;
        let brushes = self.brushes.get_or_insert_with(|| vectorcraft_brush::library(doc));
        let Some(b) = st.brush.as_deref().and_then(|name| brushes.iter().find(|b| b.name == name)) else { return false };
        let art = vectorcraft_brush::stroke_pieces(doc, b, &path.to_bezpath(), st);
        let state = self.state;
        if !self.open_group("u") {
            return true;
        }
        self.nest += 1;
        self.drawn += 1;
        for piece in &art {
            self.node(piece);
        }
        self.drawn -= 1;
        if let Some((locked, hidden)) = state {
            self.state(locked, hidden);
        }
        self.end_group("U", xy);
        self.nest -= 1;
        true
    }

    /// A fill as an object of its own, of transparency `xy`: a color, a placed gradient or a
    /// pattern the data can define as the path's fill, another pattern as its tiles cut to the path.
    fn fill_object(&mut self, path: &PathData, rule: FillRule, f: &FillLayer, xy: Xy) {
        if !matches!(f.paint, Paint::None) && self.direct(&f.paint) {
            return self.path_object(path, rule, Some(f), None, xy);
        }
        match &f.paint {
            // A gradient not placed yet: placed on the path, or its average color.
            Paint::Solid { .. } | Paint::Gradient(_) => {
                let flat = self.placed(&f.paint, path.to_bezpath().bounding_box());
                self.path_object(path, rule, Some(&FillLayer { paint: flat, ..f.clone() }), None, xy);
            }
            Paint::Pattern { pattern, xf } => {
                self.warn(PATTERNS);
                let doc = self.doc;
                let Some(def) = doc.pattern(pattern).filter(|_| self.nest < MAX_NEST) else {
                    let flat = FillLayer { paint: Paint::solid(Color::gray(0.5)), ..f.clone() };
                    return self.path_object(path, rule, Some(&flat), None, xy);
                };
                let tiles = def.instances_in(*xf, path.to_bezpath().bounding_box());
                // More tiles than a fill places come back as none: the shape is filled gray, as a
                // pattern that isn't found is.
                if tiles.is_empty() && !def.art.is_empty() {
                    self.warn(PATTERN_TILES);
                    let flat = FillLayer { paint: Paint::solid(Color::gray(0.5)), ..f.clone() };
                    return self.path_object(path, rule, Some(&flat), None, xy);
                }
                let state = self.state;
                if !self.open_group("u") {
                    return;
                }
                self.wrote_pattern = true;
                let outer = self.clip.clone();
                let region = self.cut(path, rule);
                self.clip = Some(region);
                self.nest += 1;
                self.drawn += 1;
                for t in &tiles {
                    self.node(t);
                }
                self.drawn -= 1;
                if let Some((locked, hidden)) = state {
                    self.state(locked, hidden);
                }
                self.end_group("U", xy);
                self.nest -= 1;
                self.clip = outer;
            }
            Paint::None => {}
        }
    }

    /// A path object of transparency `xy` painted with `fill` (a color) and `stroke` (a plain line
    /// of a color) over it; several subpaths make a compound path. Where the art is cut to a
    /// clipping path, the fill is the path's region cut to it and the stroke its outline cut to it,
    /// filled.
    fn path_object(&mut self, path: &PathData, rule: FillRule, fill: Option<&FillLayer>, stroke: Option<&StrokeLayer>, xy: Xy) {
        if self.clip.is_none() {
            return self.plain_object(path, rule, fill, stroke, xy.object());
        }
        let both = fill.is_some() && stroke.is_some();
        let own = if both { Xy::OPAQUE } else { xy.object() };
        if both && !self.open_group("u") {
            return;
        }
        if fill.is_some() || stroke.is_none() {
            let region = self.cut(path, rule);
            self.plain_object(&region, FillRule::NonZero, fill, None, own);
        }
        if let Some(s) = stroke {
            self.wrote_fixed = true;
            let outline = stroke::outline_region(path, rule, s);
            let region = self.cut(&outline, FillRule::NonZero);
            let fill = FillLayer { overprint: s.overprint, ..FillLayer::new(s.paint.clone()) };
            self.plain_object(&region, FillRule::NonZero, Some(&fill), None, own);
        }
        if both {
            self.end_group("U", xy);
        }
    }

    /// The paint of a path of transparency `xy` filled by `rule`: its fill color and `O`, its stroke
    /// color and `R`, `Xy`, the line style, `XR`. Without paint, `XR` comes first: an `Xy` right
    /// after a compound path or a text object would be theirs.
    fn paint_state(&mut self, fill: Option<&FillLayer>, stroke: Option<&StrokeLayer>, xy: Xy, rule: FillRule) {
        let rule = if rule == FillRule::EvenOdd { "1 XR" } else { "0 XR" };
        if let Some(f) = fill {
            self.color(&f.paint, false);
            self.line(if f.overprint { "1 O" } else { "0 O" });
        }
        if let Some(s) = stroke {
            self.color(&s.paint, true);
            self.line(if s.overprint { "1 R" } else { "0 R" });
        }
        if fill.is_none() && stroke.is_none() {
            self.line(rule);
            return self.set_xy(xy);
        }
        self.set_xy(xy);
        if let Some(s) = stroke {
            ai_ops::line_style(&mut self.body, s);
        }
        self.line(rule);
    }

    /// [`Self::path_object`] without cutting: the paint ([`Self::paint_state`]), each subpath's
    /// points and painting operator (`*u` … `*U` around several).
    fn plain_object(&mut self, path: &PathData, rule: FillRule, fill: Option<&FillLayer>, stroke: Option<&StrokeLayer>, xy: Xy) {
        let subpaths: Vec<&SubPath> = path.subpaths.iter().filter(|sp| !sp.anchors.is_empty()).collect();
        if subpaths.is_empty() || !self.take_objects(object_count(subpaths.len())) {
            return;
        }
        self.paint_state(fill, stroke, xy, rule);
        let op = paint_op(fill.is_some(), stroke.is_some());
        let several = subpaths.len() > 1;
        if several {
            self.line("*u");
        }
        // A gradient's instance paints the last subpath; the compound path takes its paint.
        let last = subpaths.len().saturating_sub(1);
        let outline = path.to_bezpath();
        for (i, sp) in subpaths.into_iter().enumerate() {
            ai_ops::subpath(&mut self.body, self.to_art, sp);
            let gradients = if i == last { (gradient(fill.map(|f| &f.paint)), gradient(stroke.map(|s| &s.paint))) } else { (None, None) };
            self.painted(op, sp.closed, gradients, stroke.is_some(), false, &outline);
        }
        if several {
            self.line("*U");
        }
    }

    // ---------- type ----------

    /// Type `n`: a text object stub naming its story in the text document ([`texts`]) when the
    /// text document can hold it, outside opacity masks, pattern tiles, symbols' art and art cut to
    /// a clipping object; else its outlines ([`Self::outlined_text`]). A frame of threaded area type
    /// names the story of its thread, which its first frame holds.
    fn text(&mut self, n: &Node, xy: Xy) {
        let NodeKind::Text(t) = &n.kind else { return };
        // Dynamic Text is written as the sizes its layout draws the type at.
        let fitted = t.dynamic_text.then(|| vectorcraft_text::drawn(t));
        let t = fitted.as_deref().unwrap_or(&**t);
        let doc = self.doc;
        let thread = doc.text_threads.iter().find(|th| th.contains(&n.id));
        // Type that can't be a stub is written as its outlines, which count their own objects.
        if self.hidden > 0 || self.clip.is_some() || self.in_symbol || (thread.is_none() && !texts::holds(n, t)) {
            return self.outlined_text(n, xy);
        }
        // The stub's objects ([`texts::stub_objects`], [`Self::take_objects`]); without room for
        // them, the type is left out before its story goes into the text document.
        let stub = texts::stub_objects(t);
        if !self.take_objects(stub) {
            return;
        }
        let placed = if let Some(thread) = thread {
            // A thread is checked for what it loses, in every frame, when its story is written.
            let placed = self.texts.thread(doc, thread, n.id, self.canvas, &|id| !cut_to_clipping_object(doc, id));
            placed.map(|p| (p, self.texts.take_lost()))
        } else {
            self.texts.add(t, self.canvas).map(|story| ((story, 0), texts::loses(t)))
        };
        let Some(((story, frame), lost)) = placed else {
            // The text document can't hold it: the outlines count their own objects.
            self.objects -= stub;
            return self.outlined_text(n, xy);
        };
        self.typed = true;
        if lost {
            self.warn(texts::TYPE_ATTRIBUTES);
        }
        if fitted.is_some() {
            self.warn(DYNAMIC_TEXT);
        }
        // The stub takes the transparency set before it, as a path does. Its isolation and knockout
        // are read only from a state block after it, as a group's are.
        self.set_object_xy(xy.object());
        texts::stub(&mut self.body, story, frame, t, self.to_art);
        if xy.isolate || xy.knockout != Knockout::Neutral {
            self.state_block(xy);
        }
    }

    /// Type `n` as a group of what Type › Create Outlines makes of it
    /// ([`vectorcraft_effects::outline_text`]): each run's glyphs with their underline and
    /// strikethrough, its color glyphs, and the object's own fills and strokes; then its inline
    /// graphics as their symbols' art where the layout places them, as the PDF page draws them;
    /// the group has the type's transparency `xy`.
    fn outlined_text(&mut self, n: &Node, xy: Xy) {
        if !self.open_group("u") {
            return;
        }
        self.warn(TYPE);
        // The outlines keep the sizes Dynamic Text draws the type at, but not the setting.
        if matches!(&n.kind, NodeKind::Text(t) if t.dynamic_text) {
            self.warn(DYNAMIC_TEXT);
        }
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
                self.drawn += 1;
                self.node(&art_node);
                self.drawn -= 1;
                self.nest -= 1;
            }
        }
        for c in over {
            self.node(c);
        }
        if let Some((locked, hidden)) = state {
            self.state(locked, hidden);
        }
        self.end_group("U", xy);
    }

    // ---------- color ----------

    /// Set the fill (or with `stroke`, the stroke) to paint `p`: a spot or global swatch by its
    /// name, Registration, the color itself, or a pattern ([`Self::pattern_paint`]). A gradient sets
    /// nothing: its instance paints ([`Self::painted`]).
    fn color(&mut self, p: &Paint, stroke: bool) {
        let (color, swatch, tint) = match p {
            Paint::Solid { color, swatch, tint } => (color, swatch, tint),
            Paint::Pattern { pattern, xf } => return self.pattern_paint(pattern, *xf, stroke),
            Paint::Gradient(_) => return,
            Paint::None => return self.plain_color(Color::gray(1.0), stroke),
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

    /// A color that names no swatch: `g` (gray), `k` (CMYK), else `c m y k r g b Xa` (RGB in the
    /// working RGB space, and Lab as display RGB), with its CMYK equivalent (an RGB color's from its
    /// values as written, a Lab color's from its Lab values).
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
                let source = if matches!(color, Color::Rgb { .. }) { ai_ops::as_written(color) } else { color };
                let cmyk = self.cms.to_cmyk(&source, self.cms.settings().intent).map(unit);
                ps::push_nums(s, &cmyk.map(f64::from));
                ps::push_nums(s, &ai_ops::rgb_values(color).map(f64::from));
                let _ = writeln!(s, "{}", op("Xa", "XA"));
            }
        }
    }

    /// A spot (`spot`) or global process color `name` of full color `full` at `tint`, written to
    /// `s`: its CMYK equivalent (from its RGB or Lab values as written), its RGB or Lab values
    /// (clamped as [`ai_ops::as_written`] clamps them), the name, how much lighter than the full
    /// color, and for any but a CMYK spot color (`x`) the model (0 CMYK, 1 RGB, 2 Lab) before `Xx`
    /// (spot) or `Xk` (global process).
    fn named(&self, s: &mut String, name: &str, full: Color, tint: f32, spot: bool, stroke: bool) {
        let full = ai_ops::as_written(full);
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
        self.in_palette = true;
        s.push_str("%AI5_BeginPalette\n0 0 Pb\n");
        self.registration_cell(s);
        let doc = self.doc;
        for sw in doc.swatches.iter() {
            self.swatch(s, sw);
        }
        for g in doc.swatch_groups.iter() {
            let mut cells = String::new();
            let count = g.swatches.iter().filter(|sw| self.swatch(&mut cells, sw)).count();
            let _ = writeln!(s, "0 {} {count} Pg", ps::bytes(g.name.as_bytes()));
            s.push_str(&cells);
        }
        s.push_str("PB\n%AI5_EndPalette\n");
        self.in_palette = false;
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

    /// One swatch of the palette: a color swatch's color, a gradient swatch's or pattern swatch's
    /// paint ([`Self::swatch_paint`]), then `(name) Pc`. False when it isn't written: Registration
    /// (written first), the None swatch, and paint the data can't hold (with a warning).
    fn swatch(&mut self, s: &mut String, sw: &Swatch) -> bool {
        if sw.name == REGISTRATION || matches!(sw.paint, Paint::None) {
            return false;
        }
        match &sw.paint {
            Paint::Solid { color, swatch, .. } => {
                if swatch.is_none() && (sw.global || sw.spot) {
                    self.named(s, &sw.name, *color, 1.0, sw.spot, false);
                } else {
                    let body = std::mem::take(&mut self.body);
                    self.plain_color(*color, false);
                    s.push_str(&std::mem::replace(&mut self.body, body));
                }
            }
            _ if self.swatch_paint(s, sw) => {}
            _ => {
                self.warn(SWATCHES);
                return false;
            }
        }
        let _ = writeln!(s, "{} Pc", ps::bytes(sw.name.as_bytes()));
        true
    }

    /// The spot color swatches, with their full colors' CMYK equivalents (as the palette writes
    /// them), for the header.
    pub(super) fn spot_swatches(&self) -> Vec<(String, [f32; 4])> {
        let intent = self.cms.settings().intent;
        self.doc
            .swatches_iter()
            .filter(|sw| sw.spot)
            .filter_map(|sw| match sw.paint {
                Paint::Solid { color, swatch: None, .. } => {
                    Some((sw.name.clone(), self.cms.to_cmyk(&self.doc.linked_color(ai_ops::as_written(color), true), intent).map(unit)))
                }
                _ => None,
            })
            .collect()
    }
}

/// The gradient of paint `p`, when it has one.
fn gradient(p: Option<&Paint>) -> Option<&vectorcraft_color::GradientPaint> {
    match p {
        Some(Paint::Gradient(g)) => Some(g),
        _ => None,
    }
}

/// `n`'s fills and strokes that paint.
fn visible_items(n: &Node) -> Vec<&AppearanceItem> {
    n.appearance
        .items
        .iter()
        .filter(|i| match i {
            AppearanceItem::Fill(f) => f.visible && !f.paint.is_none(),
            AppearanceItem::Stroke(s) => s.visible && !s.paint.is_none() && s.width > 0.0 && s.width.is_finite(),
        })
        .collect()
}

/// The painting operator of a closed subpath (§5.3.3): `b` fill and stroke, `f` fill, `s`
/// stroke, `n` neither; an open one's is upper case.
fn paint_op(fill: bool, stroke: bool) -> char {
    match (fill, stroke) {
        (true, true) => 'b',
        (true, false) => 'f',
        (false, true) => 's',
        (false, false) => 'n',
    }
}

/// The objects the reader makes of a path object of `subpaths` subpaths with points: a path for
/// each, and a compound path holding them when there are several.
fn object_count(subpaths: usize) -> usize {
    match subpaths {
        0 => 0,
        1 => 1,
        n => n + 1,
    }
}

/// The objects the reader counts for clipping path `subpaths` (those with points) painted with
/// `fill` and `stroke` ([`Writer::clipped`]). Its last subpath carries a gradient fill's or
/// stroke's gradient, so the subpaths of a filled compound clipping path with a gradient paint
/// differently, and the reader reads their fills apart from it: a compound path of a copy of each,
/// which counts the copy and its points ([`object_count`] for the clipping path itself).
fn clip_object_count(subpaths: &[&SubPath], fill: Option<&FillLayer>, stroke: Option<&StrokeLayer>) -> usize {
    let own = object_count(subpaths.len());
    let apart = subpaths.len() > 1 && fill.is_some() && (gradient(fill.map(|f| &f.paint)).is_some() || gradient(stroke.map(|s| &s.paint)).is_some());
    if !apart {
        return own;
    }
    subpaths.iter().fold(own.saturating_add(1), |n, sp| n.saturating_add(1).saturating_add(sp.anchors.len()))
}

/// The subpaths of compound path `n`'s members, as one path.
fn compound_path(n: &Node) -> PathData {
    let mut subpaths = vec![];
    for p in n.children().map(Vec::as_slice).unwrap_or_default().iter().filter_map(|c| c.path_data()) {
        subpaths.extend(p.subpaths.iter().cloned());
    }
    PathData::new(subpaths)
}

/// Is object `id` of `doc` in (or inside) a clipping group or clipping layer whose clipping object
/// isn't a path or compound path (its art is written cut to that object, [`Writer::clip_group`])?
/// Type that is a clipping object is one of these: it isn't a path.
fn cut_to_clipping_object(doc: &Document, id: NodeId) -> bool {
    let ancestry = doc.ancestry(id).unwrap_or_default();
    let above = ancestry.split_last().map(|(_, a)| a).unwrap_or_default();
    above.iter().filter_map(|a| doc.node(*a)).any(|p| {
        matches!(p.kind, NodeKind::Group { clip: true, .. } | NodeKind::Layer { clip: true, .. })
            && p.children().and_then(|c| c.first()).is_none_or(|c| clip_path(c).is_none())
    })
}

/// The outline and fill rule a clipping path or compound path `n` clips by; `None` for other
/// clipping objects and for a path without points (it clips everything away, see
/// [`Writer::clip_contents`]).
fn clip_path(n: &Node) -> Option<(PathData, FillRule)> {
    let (path, rule) = match &n.kind {
        NodeKind::Path { path, rule, guide: false, .. } => (path.clone(), *rule),
        NodeKind::Compound { rule, .. } => (compound_path(n), *rule),
        _ => return None,
    };
    path.subpaths.iter().any(|sp| !sp.anchors.is_empty()).then_some((path, rule))
}

/// `text`'s lines on `%_` lines, as a part of the data that readers of the printed form skip (an
/// opacity mask's art, a pattern's tile) has them. An image's samples stay as they are after an
/// `XI` without `%_`, and `%_%%EndData` ends them, as in the files users sent. Samples hidden
/// already (an opacity mask's art in a pattern's tile) keep their one `%_`.
pub(super) fn hide(text: &str, out: &mut String) {
    for l in text.lines() {
        if l.starts_with(images::SAMPLES_LINE) {
            match l.strip_suffix("%%EndData") {
                Some(samples) if !samples.ends_with("%_") => {
                    out.push_str(samples);
                    out.push_str("%_%%EndData");
                }
                _ => out.push_str(l),
            }
        } else {
            if !l.starts_with("%_") {
                out.push_str("%_");
            }
            out.push_str(l);
        }
        out.push('\n');
    }
}

/// The image `bytes` decoded, when its header gives at most `max` pixels; the decoder may allocate
/// at most 8 bytes a pixel.
fn decode_bounded(bytes: &[u8], max: u64) -> Option<image::DynamicImage> {
    let reader = || -> image::ImageResult<image::ImageReader<std::io::Cursor<&[u8]>>> {
        let mut r = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format()?;
        let mut limits = image::Limits::default();
        limits.max_alloc = Some(max * 8);
        r.limits(limits);
        Ok(r)
    };
    let (w, h) = reader().and_then(|r| r.into_dimensions()).ok()?;
    if u64::from(w) * u64::from(h) > max {
        return None;
    }
    reader().and_then(|r| r.decode()).ok()
}
