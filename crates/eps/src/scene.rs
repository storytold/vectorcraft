//! The document walk: each visible object is written into the page in paint order, in document
//! space (the page's first operator maps it onto PostScript's y-up space). Mirrors the PDF
//! writer's walk, without transparency.

use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt::Write;
use std::sync::Arc;

use kurbo::Shape;
use vectorcraft_color::{BlendMode, Color, GradientKind, GradientPaint, Paint};
use vectorcraft_doc::{
    AppearanceItem, ColorMode, Document, Effect, ImageBlob, LineCap, LineJoin, Node, NodeKind, StrokeAlign, StrokeLayer, SymbolBoxes, TextObject,
};
use vectorcraft_effects::stroke::{self, WrittenShape};
use vectorcraft_geom::{Affine, BezPath, FillRule, PathData, Point, Rect, Vec2};

use crate::print::fonts::{FontJob, UsedGlyphs, font_name};
use crate::ps::{self, clip_op, fill_op, push_nums, push_path};
use crate::{EpsOptions, Level, Overprint};

/// How deep symbols and brush art may nest in one another (deeper art is left out).
const MAX_NEST: u32 = 8;
/// Most pixels along a side of a freeform gradient's image (its colour field is smooth).
const MAX_FIELD_PX: f64 = 512.0;
/// Most bands a stepped gradient is drawn with.
const MAX_STEPS: usize = 1024;
/// Thinnest band of a stepped gradient (points).
const MIN_BAND: f64 = 0.25;
/// Largest image written (pixels).
const MAX_IMAGE_PIXELS: u64 = 1 << 26;

const TRANSPARENCY: &str =
    "transparency (opacity, blending modes, opacity masks, see-through gradients) is written opaque: EPS has none, flatten it first";
const RASTER_FX: &str = "raster effects (shadows, glows, blurs, feathering) are left out: EPS has no transparency, flatten them first";
const IMAGE_ALPHA_L2: &str = "transparent image pixels are written white at PostScript Level 2 (Level 3 masks them out)";
const IMAGE_PARTLY: &str = "partly transparent image pixels are written over white";
/// The warning for an image left out because it can't be decoded: the note raster exports give
/// (`vectorcraft-engine`'s `rasterfx::undecoded_warning`).
const UNDECODED: &str = "an image couldn't be decoded and is missing from the picture";
const LARGE_IMAGE: &str = "an image that is too large for EPS was left out";
const SPOT_GRADIENTS_RGB: &str = "gradients mixing a spot color with RGB or Lab colors are written without the spot ink";
const SPOT_GRADIENTS_L2: &str =
    "gradients mixing a spot color with other inks are written in process colors at PostScript Level 2 (Level 3 keeps the spot inks)";
const SPOT_GRADIENTS_SHARED: &str = "gradients whose spot inks can't share a DeviceN color space are written without the spot inks";
const SPOT_GRADIENTS_PERCEPTUAL: &str = "Perceptual gradients of spot colors are written in process colors";
const SPOT_GRADIENTS_HUE: &str = "gradients of spot colors mixed around the hue wheel are written in process colors";
const CONIC_WEDGES: &str = "conic gradients are written as solid wedges (EPS has no conic shading)";
const DIAMOND_QUADRANTS: &str = "diamond gradients are written as four linear gradients (EPS has no diamond shading)";
const SPREAD_CAPPED: &str =
    "gradients that repeat or reflect are written with at most 256 runs of their stops (at most 16384 stops); they pad past those";
/// Wedges a conic gradient is written as: one a degree.
const CONIC_WEDGE_COUNT: usize = 360;
/// The most colorants a DeviceN color space may have (PostScript's limit).
const MAX_DEVICE_N: usize = 32;
const MISSING_PATTERN: &str = "missing patterns are written as mid-grey";
const TOO_LARGE: &str = "the artwork is too large for EPS: what follows 256 MB of PostScript was left out";
/// The most PostScript the page body may grow to: symbols and patterns that hold one another
/// expand into every instance, so nesting depth alone doesn't bound the output. An image placed
/// more than once counts its data once.
const MAX_BODY: usize = 256 << 20;
/// Most compressed bytes in one string of an image's samples at Level 2 (under PostScript's
/// 65,535-byte string limit).
const IMAGE_CHUNK: usize = 48 << 10;

/// An image as written: what sets up its color, its image dictionary and its compressed samples.
/// Placed once, it is written where it is drawn, its data following the operators that read it.
/// Placed more than once, its samples are written once in the setup as `VCim<i>` (a reusable
/// stream at Level 3, an array of strings at Level 2) and each placement reads them from there.
struct ImageCode {
    /// The color space or ink operators, each line ending in a line break.
    prelude: String,
    /// `<< /ImageType … /ImageMatrix […]` with its mask colors: the dictionary up to its data source.
    dict: String,
    /// The decoding filter of the samples: `/FlateDecode` or `/RunLengthDecode`.
    filter: &'static str,
    /// `image` or `imagemask`.
    op: &'static str,
    /// The compressed samples in ASCII85, as lines with the end marker.
    ascii: String,
    /// The compressed samples, kept at Level 2, where a reused image's samples are written as
    /// strings of at most [`IMAGE_CHUNK`] bytes.
    data: Vec<u8>,
    /// How many placements in the page draw it.
    uses: usize,
    /// The length of [`ImageCode::push_inline`]'s text.
    inline_len: usize,
    /// The length of [`ImageCode::reference`]'s text.
    ref_len: usize,
}

impl ImageCode {
    /// A hash of everything that is written for the image: equal images have equal keys.
    fn content_key(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (&self.prelude, &self.dict, self.filter, self.op, &self.ascii).hash(&mut h);
        h.finish()
    }

    /// Is everything written for `other` the same as for this image?
    fn same_image(&self, other: &ImageCode) -> bool {
        self.prelude == other.prelude && self.dict == other.dict && self.filter == other.filter && self.op == other.op && self.ascii == other.ascii
    }

    /// The operators and data that draw the image where it is placed.
    fn push_inline(&self, s: &mut String) {
        ps::push_inline_image(s, "VCsrc", &self.prelude, &self.dict, self.filter, self.op, &self.ascii);
    }

    /// The operators that draw the image `VCim<i>` holds, for an image placed more than once.
    fn reference(&self, i: usize, level: Level) -> String {
        let mut s = String::new();
        let name = format!("VCim{i}");
        match level {
            // The stream is read again from its start.
            Level::Three => ps::push_reusable_image_ref(&mut s, &name, &self.prelude, &self.dict, self.filter, self.op),
            // A procedure gives the strings in turn, then an empty string: the end of the data.
            Level::Two => ps::push_image_array_ref(&mut s, "VCi", &name, &self.prelude, &self.dict, self.filter, self.op),
        }
        s
    }

    /// The setup's definition of `VCim<i>`: the image's compressed samples, once.
    fn resource(&self, i: usize, level: Level) -> String {
        let mut s = String::with_capacity(self.ascii.len() + 200);
        match level {
            Level::Three => {
                // The stream reads all of its data when it is made, so the data follows the
                // procedure that makes it.
                let intent = if self.op == "imagemask" { 1 } else { 0 };
                ps::push_reusable_image_resource(&mut s, "VCsrc", &format!("VCim{i}"), intent, &self.ascii);
            }
            Level::Two => {
                let _ = writeln!(s, "/VCim{i} [");
                for chunk in self.data.chunks(IMAGE_CHUNK) {
                    s.push_str("<~");
                    s.push_str(&ps::ascii85(chunk));
                }
                s.push_str("] def\n");
            }
        }
        s
    }
}

/// What the walk wrote: definitions for the setup, the page, the spot colours it uses (name and
/// CMYK equivalent) and the warnings.
pub(crate) struct Page {
    pub setup: String,
    pub body: String,
    pub custom: Vec<(String, [f32; 4])>,
    pub warnings: Vec<String>,
    /// The glyphs shown through the print job's fonts ([`Scene::with_fonts`]).
    pub fonts: UsedGlyphs,
}

/// The colour spaces process colours are written in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Space {
    Gray,
    Rgb,
    Cmyk,
}

impl Space {
    fn name(self) -> &'static str {
        match self {
            Space::Gray => "/DeviceGray",
            Space::Rgb => "/DeviceRGB",
            Space::Cmyk => "/DeviceCMYK",
        }
    }
    fn n(self) -> usize {
        match self {
            Space::Gray => 1,
            Space::Rgb => 3,
            Space::Cmyk => 4,
        }
    }
}

/// The color space a gradient's stops are written in: a process space, or one defined in the
/// setup (a spot color's Separation space or a DeviceN space of spot and process inks) by name,
/// with its number of components.
#[derive(Clone, Debug, PartialEq)]
enum GradientSpace {
    Process(Space),
    Inks(String, usize),
}

impl GradientSpace {
    fn n(&self) -> usize {
        match self {
            GradientSpace::Process(s) => s.n(),
            GradientSpace::Inks(_, n) => *n,
        }
    }

    /// Whether components `c` in this space are paper white: no ink, or full light.
    fn paper(&self, c: &[f32]) -> bool {
        match self {
            GradientSpace::Process(Space::Gray | Space::Rgb) => c.iter().all(|v| *v >= 1.0),
            GradientSpace::Process(Space::Cmyk) | GradientSpace::Inks(..) => c.iter().all(|v| *v <= 0.0),
        }
    }
}

/// A gradient's stops as written: offsets from 0 to 1 and the components in its space.
type Stops = Vec<(f64, Vec<f32>)>;
/// A gradient's stops before their offsets are put in order and run from 0 to 1.
type RawStops = Vec<(f32, Vec<f32>)>;

/// A colour as written: in a process space (its first [`Space::n`] components) or a tint of a
/// spot colour space defined in the setup.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Ink {
    Process(Space, [f32; 4]),
    Spot { space: usize, tint: f32 },
}

/// One drawing of an overprinting colorized image: the name of the color space it is drawn in
/// and the amount of each of that space's inks ([`Scene::overprint_passes`]).
struct InkPass {
    space: String,
    values: Vec<f32>,
}

fn unit(v: f32) -> f32 {
    if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 }
}

/// The color of `stops` (offsets in order, from 0 to 1) at 1, which a diamond gradient without a
/// frame is everywhere, as [`vectorcraft_color::Gradient::sample`] gives it: the first stop at
/// 1, the end of the stretch before it when several stops share that offset.
fn frameless_color(stops: &[(f64, Vec<f32>)]) -> Option<&[f32]> {
    stops.iter().find(|(o, _)| *o >= 1.0).or(stops.last()).map(|(_, c)| c.as_slice())
}

/// Can `a` be drawn through (a finite, non-flat map)?
fn invertible(a: Affine) -> bool {
    let det = a.determinant();
    det.is_finite() && det.abs() > 1e-12
}

fn overlaps(a: Rect, b: Rect) -> bool {
    a.x0 <= b.x1 && b.x0 <= a.x1 && a.y0 <= b.y1 && b.y0 <= a.y1
}

pub(crate) struct Scene<'a> {
    doc: &'a Document,
    /// The art boxes of `doc`'s symbols, which the walk measures symbol instances by when it
    /// leaves out objects outside the region ([`SymbolBoxes::quick`]). The box of an instance whose
    /// art holds brushed strokes grows by as far as their brush art can paint
    /// ([`SymbolBoxes::with_brush_art`]).
    symbols: SymbolBoxes<'a>,
    o: &'a EpsOptions,
    /// RGB and Lab colours are written as CMYK.
    cmyk: bool,
    cms: Arc<vectorcraft_color::cms::Cms>,
    body: String,
    setup: String,
    /// Spot colour spaces defined in the setup (`VCsp<i>`): swatch name, colorant name
    /// ([`Scene::colorant_name`]) and CMYK equivalent.
    spots: Vec<(String, String, [f32; 4])>,
    /// DeviceN color spaces defined in the setup (`VCdn<i>`), as their definitions.
    device_n: Vec<String>,
    warnings: Vec<String>,
    brushes: Option<Vec<vectorcraft_brush::Brush>>,
    /// Image keys → the index in `codes` of what draws them (`None`: left out).
    images: HashMap<String, Option<usize>>,
    /// The source key and solid paint together identify a colorized encoding.
    colorized_images: HashMap<(String, String), Option<usize>>,
    /// Image keys whose encoding didn't fit under the page limit → how much its first placement
    /// adds to the body. The image is encoded again only once the page has room for that.
    too_large_images: HashMap<String, usize>,
    /// The same for colorized encodings, by `colorized_images`' keys.
    too_large_colorized: HashMap<(String, String), usize>,
    /// How many times an image or a colorized image was encoded.
    #[cfg(test)]
    encodes: usize,
    /// Freeform gradients' images by [`ImageCode::content_key`] → their indices in `codes`: a field
    /// encoded the same as one already kept draws from it.
    fields: HashMap<u64, Vec<usize>>,
    /// The encoded images.
    codes: Vec<ImageCode>,
    /// Where in `body` each placement of an encoded image goes (in order) and the index of its code.
    placements: Vec<(usize, usize)>,
    /// What the placements in `placements` add to `body`'s length when the page is finished.
    pending: usize,
    /// The most the page body may grow to ([`MAX_BODY`]).
    max_body: usize,
    /// Symbols and brush art being written inside one another.
    nest: u32,
    /// The document is a separation's plate: every color is the gray of one ink's coverage
    /// ([`Self::on_plate`]).
    plate: bool,
    /// The gradient being painted overprints on a plate: where it is paper white it puts none of
    /// the plate's ink down, and the plate below it is left as it is ([`Self::inked_spans`]).
    keep_below: bool,
    /// Overprinting is on in the page's graphics state ([`Self::overprint_on`]).
    overprinting: bool,
    /// The print job's fonts: type in a solid fill is shown through them (`None`: drawn as
    /// outlines).
    fonts: Option<&'a RefCell<FontJob>>,
    /// The glyphs shown through `fonts`.
    used: UsedGlyphs,
    /// The glyphs `used` gained, in the order it gained them: an object left out for the page
    /// limit takes its own back out of `used` ([`Self::forget_glyphs`]).
    used_order: Vec<(usize, u32)>,
}

impl<'a> Scene<'a> {
    pub fn new(doc: &'a Document, o: &'a EpsOptions) -> Self {
        Self {
            doc,
            symbols: doc.symbol_boxes().quick().with_brush_art(),
            o,
            cmyk: o.cmyk || doc.color_mode == ColorMode::Cmyk,
            cms: vectorcraft_color::cms::active(),
            body: String::new(),
            setup: String::new(),
            spots: vec![],
            device_n: vec![],
            warnings: vec![],
            brushes: None,
            images: HashMap::new(),
            colorized_images: HashMap::new(),
            too_large_images: HashMap::new(),
            too_large_colorized: HashMap::new(),
            #[cfg(test)]
            encodes: 0,
            fields: HashMap::new(),
            codes: vec![],
            placements: vec![],
            pending: 0,
            max_body: MAX_BODY,
            nest: 0,
            plate: false,
            keep_below: false,
            overprinting: false,
            fonts: None,
            used: UsedGlyphs::new(),
            used_order: vec![],
        }
    }

    /// This scene showing type through the print job's fonts `fonts`.
    pub fn with_fonts(mut self, fonts: Option<&'a RefCell<FontJob>>) -> Self {
        self.fonts = fonts;
        self
    }

    /// This scene drawing a separation's plate when `plate` is true: an overprinting gradient
    /// paints only where it puts the plate's ink down, as an overprinting solid color that puts
    /// none down is drawn as nothing on it.
    pub fn on_plate(mut self, plate: bool) -> Self {
        self.plate = plate;
        self
    }

    /// This scene with a page body of at most `max` bytes.
    #[cfg(test)]
    pub fn with_max_body(mut self, max: usize) -> Self {
        self.max_body = max;
        self
    }

    /// [`Self::run`], with how many placements draw each encoded image the scene kept, and the
    /// indices in `codes` that its image, colorized-image and field caches name.
    #[cfg(test)]
    pub fn run_with_codes(mut self) -> (Page, Vec<usize>, Vec<usize>) {
        let doc = self.doc;
        for l in &doc.layers {
            self.node(l, false);
        }
        let uses = self.codes.iter().map(|c| c.uses).collect();
        let cached = self.images.values().chain(self.colorized_images.values()).flatten().chain(self.fields.values().flatten()).copied().collect();
        (self.page(), uses, cached)
    }

    /// [`Self::run`], with how many times the scene encoded an image or a colorized image.
    #[cfg(test)]
    pub fn run_counting_encodes(mut self) -> (Page, usize) {
        let doc = self.doc;
        for l in &doc.layers {
            self.node(l, false);
        }
        let encodes = self.encodes;
        (self.page(), encodes)
    }

    pub fn run(mut self) -> Page {
        let doc = self.doc;
        for l in &doc.layers {
            self.node(l, false);
        }
        self.page()
    }

    /// Write one object (art outside the document: a print job's marks) instead of the layers.
    pub fn run_node(mut self, n: &Node) -> Page {
        self.node(n, false);
        self.page()
    }

    fn page(mut self) -> Page {
        if !self.placements.is_empty() {
            let level = self.o.level;
            let mut body = String::with_capacity(self.body.len().saturating_add(self.pending));
            let mut at = 0;
            for &(offset, i) in &self.placements {
                body.push_str(self.body.get(at..offset).unwrap_or_default());
                at = offset;
                let Some(code) = self.codes.get(i) else { continue };
                if code.uses > 1 {
                    body.push_str(&code.reference(i, level));
                } else {
                    code.push_inline(&mut body);
                }
            }
            body.push_str(self.body.get(at..).unwrap_or_default());
            self.body = body;
            for (i, code) in self.codes.iter().enumerate().filter(|(_, c)| c.uses > 1) {
                self.setup.push_str(&code.resource(i, level));
            }
        }
        let custom = self
            .spots
            .iter()
            .filter(|(n, ..)| n != vectorcraft_color::swatch::REGISTRATION)
            .map(|(_, colorant, cmyk)| (colorant.clone(), *cmyk))
            .collect();
        Page { setup: self.setup, body: self.body, custom, warnings: self.warnings, fonts: self.used }
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

    // ---------- colour ----------

    /// A process colour in the space it is written in.
    fn process(&self, c: &Color) -> Ink {
        match *c {
            Color::Cmyk { c, m, y, k } => Ink::Process(Space::Cmyk, [c, m, y, k].map(unit)),
            // Grey is ink coverage; PostScript's grey is lightness.
            Color::Gray { k } => Ink::Process(Space::Gray, [unit(1.0 - k), 0.0, 0.0, 0.0]),
            _ if self.cmyk => Ink::Process(Space::Cmyk, self.cms.to_cmyk(c, self.cms.settings().intent).map(unit)),
            Color::Rgb { r, g, b } => Ink::Process(Space::Rgb, [r, g, b, 0.0].map(unit)),
            Color::Lab { .. } => {
                let [r, g, b] = c.to_rgb();
                Ink::Process(Space::Rgb, [r, g, b, 0.0].map(unit))
            }
        }
    }

    /// A solid paint: a tint of its spot colour (or of Registration, every plate) when it is linked
    /// to one, else a process colour.
    fn solid(&mut self, c: &Color, link: Option<&str>, tint: f32) -> Ink {
        match link.and_then(|n| self.spot_space(n)) {
            Some(space) => Ink::Spot { space, tint: unit(tint) },
            None => self.process(c),
        }
    }

    /// The index of the Separation colour space of spot swatch (or Registration) `name`, defined
    /// in the setup the first time; `None` when `name` is neither.
    fn spot_space(&mut self, name: &str) -> Option<usize> {
        if let Some(i) = self.spots.iter().position(|(n, ..)| n == name) {
            return Some(i);
        }
        let registration = name == vectorcraft_color::swatch::REGISTRATION;
        let cmyk = if registration {
            [1.0; 4]
        } else {
            let doc = self.doc;
            let sw = doc.swatch(name).filter(|s| s.spot)?;
            let color = doc.linked_color(sw.paint.color()?, true);
            self.cms.to_cmyk(&color, self.cms.settings().intent).map(unit)
        };
        let i = self.spots.len();
        let _ = write!(self.setup, "/VCsp{i} [/Separation ");
        let colorant = if registration {
            self.setup.push_str("/All /DeviceCMYK {dup dup dup}");
            "All".to_string()
        } else {
            let colorant = self.colorant_name(name);
            let [c, m, y, k] = cmyk.map(|v| ps::num(f64::from(v)));
            let _ = write!(self.setup, "{} cvn /DeviceCMYK {{dup {c} mul exch dup {m} mul exch dup {y} mul exch {k} mul}}", ps::string(&colorant));
            colorant
        };
        self.setup.push_str("] def\n");
        self.spots.push((name.to_string(), colorant, cmyk));
        Some(i)
    }

    /// The colorant name spot swatch `name` is written under: its own name, except that `All`
    /// (every plate in PostScript) and `None` (no plate) become `All Spot` and `None Spot`,
    /// numbered `All Spot 2`, `All Spot 3`, … past the names of the document's spot swatches.
    fn colorant_name(&self, name: &str) -> String {
        if name != "All" && name != "None" {
            return name.to_string();
        }
        let taken = |c: &str| self.doc.swatches_iter().any(|s| s.spot && s.name == c);
        let mut colorant = format!("{name} Spot");
        let mut n = 2;
        while taken(&colorant) {
            colorant = format!("{name} Spot {n}");
            n += 1;
        }
        colorant
    }

    fn set_ink(&mut self, ink: Ink) {
        match ink {
            Ink::Process(space, v) => {
                for x in v.iter().take(space.n()) {
                    ps::push_num(&mut self.body, f64::from(*x));
                    self.body.push(' ');
                }
                self.line(match space {
                    Space::Gray => "g",
                    Space::Rgb => "rg",
                    Space::Cmyk => "k",
                });
            }
            Ink::Spot { space, tint } => {
                let _ = writeln!(self.body, "VCsp{space} setcolorspace {} setcolor", ps::num(f64::from(tint)));
            }
        }
    }

    /// Whether a fill or stroke of `paint` marked to overprint (`overprint`) does: not when it is
    /// paper white with Discard White Overprint in Output (Document Setup); a spot white still
    /// overprints ([`Self::spot_ink`]). On a plate the colors are already the plate's coverage, and
    /// the plate's document already left out or kept each overprinting paint by that setting, so
    /// white is not checked there.
    fn overprints(&self, overprint: bool, paint: &Paint) -> bool {
        let discarded = !self.plate && self.doc.setup.discard_white_overprint && vectorcraft_doc::overprint::is_white(paint) && !self.spot_ink(paint);
        overprint && !discarded
    }

    /// Whether `paint` is a tint above 0% of a spot swatch with a solid color, which the page writes
    /// as a Separation ink of its own ([`Self::spot_space`]); a link is judged by the first swatch of
    /// its name.
    fn spot_ink(&self, paint: &Paint) -> bool {
        matches!(paint, Paint::Solid { swatch: Some(name), tint, .. } if *tint > 0.0 && self.doc.swatch(name).is_some_and(|s| s.spot && s.paint.color().is_some()))
    }

    /// Turn overprinting on for one paint (when overprints are preserved); `false` when it stays off
    /// or is already on (a pattern's tiles inside an overprinting pattern fill).
    fn overprint_on(&mut self, overprint: bool) -> bool {
        let on = overprint && self.o.overprint == Overprint::Preserve && !self.overprinting;
        if on {
            self.line("true op");
            self.overprinting = true;
        }
        on
    }

    fn overprint_off(&mut self, on: bool) {
        if on {
            self.line("false op");
            self.overprinting = false;
        }
    }

    // ---------- objects ----------

    fn node(&mut self, n: &Node, force: bool) {
        if !force && !n.visible {
            return;
        }
        if let NodeKind::Layer { template: true, .. } = n.kind {
            return;
        }
        if self.size() >= self.max_body {
            self.warn(TOO_LARGE);
            return;
        }
        // Off the region, an object is still written when brush art painted along its brushed
        // strokes, or those of the objects in it, can reach the region.
        let off = |b: Rect, reach: f64| !overlaps(b.inflate(1.0 + reach, 1.0 + reach), self.o.region);
        match n.visual_bounds_with(&self.symbols) {
            Some(b) if !force && off(b, 0.0) && off(b, vectorcraft_brush::reach_within(n)) => return,
            None if !n.is_container() => return,
            _ => {}
        }
        self.note_losses(n);
        let (was, placed, codes, glyphs) = (self.body.len(), self.placements.len(), self.codes.len(), self.used_order.len());
        match &n.kind {
            NodeKind::Layer { children, clip: false, .. } | NodeKind::Group { children, clip: false } => {
                children.iter().for_each(|c| self.node(c, false));
            }
            NodeKind::Group { children, clip: true } | NodeKind::Layer { children, clip: true, .. } => self.clip_group(children),
            NodeKind::Path { path, rule, guide, .. } => {
                if !*guide {
                    self.shape(n, &path.to_bezpath(), *rule);
                }
            }
            NodeKind::Compound { children, rule } => {
                let mut bp = BezPath::new();
                for p in children.iter().filter_map(|c| c.path_data()) {
                    bp.extend(p.to_bezpath());
                }
                self.shape(n, &bp, *rule);
            }
            NodeKind::Text(t) => self.text(n, t),
            NodeKind::Image(im) if !n.is_distorted_image() => {
                let shown = self.doc.shown_image(n, im);
                let im = &*shown;
                let blob = self.doc.image_blob(im);
                match n.extras().colorize.as_deref() {
                    // On a plate, an overprinting image whose color puts none of the plate's ink
                    // down is drawn as nothing, as such a solid color is ([`Self::on_plate`]).
                    Some(p)
                        if self.plate
                            && self.o.overprint == Overprint::Preserve
                            && n.colorize_overprints()
                            && vectorcraft_doc::overprint::is_white(p) => {}
                    // Off a plate, a white process color knocks out with Discard White Overprint
                    // (a plate's colors come from the separator, which has applied it already); a
                    // spot ink with a white color still overprints, as it is written as that ink.
                    Some(p @ Paint::Solid { .. }) if blob.is_none_or(|b| b.gray_source().is_some()) => {
                        let discarded = !self.plate
                            && vectorcraft_doc::overprint::is_white(p)
                            && vectorcraft_doc::overprint::DiscardWhite::of(self.doc).discards(p);
                        let on = self.overprint_on(n.colorize_overprints() && !discarded);
                        self.colorized_image(im, blob, p, on);
                        self.overprint_off(on);
                    }
                    _ => self.image(&im.key, blob, im.width, im.height, im.xf),
                }
            }
            NodeKind::SymbolInstance { symbol, xf } => {
                if let Some(sym) = self.doc.symbols.iter().find(|s| &s.name == symbol)
                    && self.nest < MAX_NEST
                    && let Some(art) = self.doc.place_symbol_art(symbol, &vectorcraft_doc::symbol_overrides::instance_art(&sym.art, n), *xf)
                {
                    self.nest += 1;
                    self.node(&art, true);
                    self.nest -= 1;
                }
            }
            // Live blends, envelopes, meshes and repeats (and distorted images) are written as
            // their evaluated art.
            NodeKind::Blend { .. }
            | NodeKind::Envelope { .. }
            | NodeKind::Mesh(_)
            | NodeKind::Repeat(_)
            | NodeKind::PlacedDocument(_)
            | NodeKind::Intertwine { .. }
            | NodeKind::Image(_) => {
                let g = vectorcraft_effects::expand_live_deep(Some(self.doc), n);
                for c in g.children().into_iter().flatten() {
                    self.node(c, false);
                }
            }
            // Ruler guides draw on screen only.
            NodeKind::RulerGuide { .. } => {}
        }
        if self.size() > self.max_body {
            self.body.truncate(was);
            self.unplace(placed);
            self.forget_codes(codes);
            self.forget_glyphs(glyphs);
            self.warn(TOO_LARGE);
        }
    }

    /// Take the glyphs `used` gained after its first `n` back out of it.
    fn forget_glyphs(&mut self, n: usize) {
        for (face, gid) in self.used_order.drain(n.min(self.used_order.len())..) {
            if let Some(gids) = self.used.get_mut(&face) {
                gids.remove(&gid);
                if gids.is_empty() {
                    self.used.remove(&face);
                }
            }
        }
    }

    /// How long the page body will be: what is written and what its image placements add.
    fn size(&self) -> usize {
        self.body.len().saturating_add(self.pending)
    }

    /// Take back the image placements after the first `n`, last first.
    fn unplace(&mut self, n: usize) {
        while self.placements.len() > n {
            let Some((_, i)) = self.placements.pop() else { break };
            let Some(code) = self.codes.get_mut(i) else { continue };
            code.uses = code.uses.saturating_sub(1);
            let cost = if code.uses == 0 { code.inline_len } else { code.ref_len };
            self.pending = self.pending.saturating_sub(cost);
        }
    }

    /// A clipping group: the art inside its first child's outline. The clipping path's fills
    /// paint behind the art (clipped) and its strokes over it (not clipped).
    fn clip_group(&mut self, children: &[Arc<Node>]) {
        let Some((clip, rest)) = children.split_first() else { return };
        // Nothing to clip by hides the clipped art.
        let Some((outline, rule)) = vectorcraft_effects::clip_outline(clip) else { return };
        let paint = clip.clip_paint();
        self.line("q");
        push_path(&mut self.body, &outline);
        self.body.push_str(clip_op(rule));
        if let Some(fill) = &paint.fill {
            self.node(fill, false);
        }
        rest.iter().for_each(|c| self.node(c, false));
        self.line("Q");
        if let Some(stroke) = &paint.stroke {
            self.node(stroke, false);
        }
    }

    /// Warn about what EPS can't hold in `n`'s look.
    fn note_losses(&mut self, n: &Node) {
        if n.opacity < 1.0 || n.blend != BlendMode::Normal || n.extras().mask.as_ref().is_some_and(|m| !m.disabled) {
            self.warn(TRANSPARENCY);
        }
        let raster = |fx: &[Effect]| fx.iter().any(|e| e.visible && vectorcraft_effects::is_raster(&e.id));
        if raster(&n.appearance.effects) || n.appearance.items.iter().any(|i| raster(i.effects())) {
            self.warn(RASTER_FX);
        }
    }

    /// Warn when a fill's or stroke's own opacity or blending mode can't be written.
    fn note_item(&mut self, opacity: f32, blend: BlendMode) {
        if opacity < 1.0 || blend != BlendMode::Normal {
            self.warn(TRANSPARENCY);
        }
    }

    /// A path's fills and strokes, in paint order.
    fn shape(&mut self, n: &Node, bp: &BezPath, rule: FillRule) {
        if bp.elements().is_empty() {
            return;
        }
        let bounds = bp.bounding_box();
        for item in &n.appearance.items {
            match item {
                AppearanceItem::Fill(fl) if fl.visible && !fl.paint.is_none() => {
                    self.note_item(fl.opacity, fl.blend);
                    let overprint = self.overprints(fl.overprint, &fl.paint);
                    self.fill(bp, rule, &fl.paint, bounds, overprint);
                }
                AppearanceItem::Stroke(st) if st.visible && !st.paint.is_none() && st.width > 0.0 && st.width.is_finite() => {
                    self.stroke(bp, rule, st, bounds);
                }
                _ => {}
            }
        }
    }

    /// Fill `bp` (by `rule`) with paint `p`; `bounds` places unplaced gradients and patterns.
    fn fill(&mut self, bp: &BezPath, rule: FillRule, p: &Paint, bounds: Rect, overprint: bool) {
        if p.is_none() {
            return;
        }
        let on = self.overprint_on(overprint);
        match p {
            Paint::None => {}
            Paint::Solid { color, swatch, tint } => {
                let ink = self.solid(color, swatch.as_deref(), *tint);
                self.set_ink(ink);
                push_path(&mut self.body, bp);
                self.body.push_str(fill_op(rule));
            }
            _ => {
                self.line("q");
                push_path(&mut self.body, bp);
                self.body.push_str(clip_op(rule));
                self.paint_overprinting(overprint, p, bounds, bp.bounding_box());
                self.line("Q");
            }
        }
        self.overprint_off(on);
    }

    /// [`Self::paint_area`] for a fill or stroke that overprints when `overprint` is true.
    fn paint_overprinting(&mut self, overprint: bool, p: &Paint, bounds: Rect, area: Rect) {
        let keep = std::mem::replace(&mut self.keep_below, self.plate && overprint);
        self.paint_area(p, bounds, area);
        self.keep_below = keep;
    }

    /// Paint all of `area` (inside the current clip) with gradient or pattern `p`, placed on
    /// `bounds`.
    fn paint_area(&mut self, p: &Paint, bounds: Rect, area: Rect) {
        let doc = self.doc;
        match p {
            Paint::Gradient(g) if g.gradient.kind == GradientKind::Freeform => self.field_image(g, bounds, area),
            Paint::Gradient(g) => self.gradient(g, bounds, area),
            Paint::Pattern { pattern, xf } => match doc.pattern(pattern) {
                Some(def) => {
                    for inst in def.instances_in(*xf, area) {
                        self.node(&inst, true);
                    }
                }
                None => {
                    self.warn(MISSING_PATTERN);
                    self.set_ink(Ink::Process(Space::Gray, [0.5, 0.0, 0.0, 0.0]));
                    self.rect_fill(area);
                }
            },
            Paint::Solid { color, swatch, tint } => {
                let ink = self.solid(color, swatch.as_deref(), *tint);
                self.set_ink(ink);
                self.rect_fill(area);
            }
            Paint::None => {}
        }
    }

    /// Fill rectangle `r` (a little larger, so its edges fall outside any clip it fills).
    fn rect_fill(&mut self, r: Rect) {
        let r = r.abs().inflate(1.0, 1.0);
        push_nums(&mut self.body, &[r.x0, r.y0, r.width(), r.height()]);
        self.line("rectfill");
    }

    /// Paint stroke `st` of the shape `bp` (filled by `rule`; geometric bounds `bounds`).
    fn stroke(&mut self, bp: &BezPath, rule: FillRule, st: &StrokeLayer, bounds: Rect) {
        if !stroke::is_plain(st) && self.brush_art(bp, st) {
            return;
        }
        self.note_item(st.opacity, st.blend);
        let overprint = self.overprints(st.overprint, &st.paint);
        let paint_bounds = st.paint_bounds(bounds);
        let area_paint = match &st.paint {
            Paint::Pattern { .. } => true,
            Paint::Gradient(g) => g.gradient.kind == GradientKind::Freeform,
            _ => false,
        };
        if area_paint {
            // The area the stroke paints, as Outline Stroke makes it, painted with the pattern's
            // tiles or the freeform's image.
            let region = stroke::outline_region(&PathData::from_bezpath(bp), rule, st).to_bezpath();
            self.fill(&region, FillRule::NonZero, &st.paint, paint_bounds, overprint);
            return;
        }
        let w = stroke::for_writer(bp, st);
        let sided = match w.side {
            Some(StrokeAlign::Inside) => {
                self.line("q");
                push_path(&mut self.body, bp);
                self.body.push_str(clip_op(rule));
                true
            }
            Some(StrokeAlign::Outside) => {
                // Everything outside the path: a frame around all the stroke reaches plus the
                // path, even-odd.
                let mut outside = w.reach(st, bounds).inflate(1.0, 1.0).to_path(0.1);
                outside.extend(bp.iter());
                self.line("q");
                push_path(&mut self.body, &outside);
                self.body.push_str(clip_op(FillRule::EvenOdd));
                true
            }
            _ => false,
        };
        match &w.shape {
            WrittenShape::Stroke { width } => {
                let on = self.overprint_on(overprint);
                self.line_style(st, *width);
                match &st.paint {
                    Paint::Solid { color, swatch, tint } => {
                        let ink = self.solid(color, swatch.as_deref(), *tint);
                        self.set_ink(ink);
                        push_path(&mut self.body, bp);
                        self.line("S");
                    }
                    p => {
                        // A gradient within the stroke: the stroke's outline as a clip.
                        self.line("q");
                        push_path(&mut self.body, bp);
                        self.line("strokepath W");
                        self.paint_overprinting(overprint, p, paint_bounds, w.reach(st, bounds));
                        self.line("Q");
                    }
                }
                self.overprint_off(on);
            }
            WrittenShape::Fill(outlines) if st.path_gradient().is_some() => {
                // A gradient along or across the stroke: slices clipped to its outlines.
                if let Some(ws) = stroke::written_slices(bp, rule, st, outlines) {
                    self.line("q");
                    push_path(&mut self.body, &ws.clip);
                    self.body.push_str(clip_op(FillRule::NonZero));
                    for (shape, paint) in &ws.slices {
                        self.fill(shape, FillRule::NonZero, paint, shape.bounding_box(), overprint);
                    }
                    self.line("Q");
                }
            }
            WrittenShape::Fill(outlines) => {
                for o in outlines {
                    self.fill(o, FillRule::NonZero, &st.paint, paint_bounds, overprint);
                }
            }
        }
        if sided {
            self.line("Q");
        }
    }

    /// The line width, cap, join, miter limit and dashes of `st` at `width`.
    fn line_style(&mut self, st: &StrokeLayer, width: f64) {
        let cap = match st.cap {
            LineCap::Butt => 0,
            LineCap::Round => 1,
            LineCap::Square => 2,
        };
        let join = match st.join {
            LineJoin::Miter => 0,
            LineJoin::Round => 1,
            LineJoin::Bevel => 2,
        };
        let s = &mut self.body;
        ps::push_num(s, width);
        let _ = write!(s, " w {cap} J {join} j ");
        ps::push_num(s, if st.miter_limit.is_finite() { st.miter_limit.max(1.0) } else { 10.0 });
        s.push_str(" M [");
        let dash = st.dash.as_ref().filter(|d| d.is_dashed());
        if let Some(d) = dash {
            push_nums(s, &d.pattern);
        }
        s.push_str("] ");
        ps::push_num(s, dash.map_or(0.0, |d| d.offset));
        s.push_str(" d\n");
    }

    /// Paint stroke `st` of `bp` with its brush art. False when it has no known brush.
    fn brush_art(&mut self, bp: &BezPath, st: &StrokeLayer) -> bool {
        if self.nest >= MAX_NEST {
            return false;
        }
        let doc = self.doc;
        let brushes = self.brushes.get_or_insert_with(|| vectorcraft_brush::library(doc));
        let Some(b) = st.brush.as_deref().and_then(|name| brushes.iter().find(|b| b.name == name)) else { return false };
        let art = vectorcraft_brush::stroke_pieces(doc, b, bp, st);
        self.note_item(st.opacity, st.blend);
        self.nest += 1;
        for piece in &art {
            self.node(piece, false);
        }
        self.nest -= 1;
        true
    }

    // ---------- type ----------

    /// Type as glyph outlines: the object's own fills and strokes below and above the
    /// characters, each run's fill and stroke in text space.
    fn text(&mut self, n: &Node, t: &TextObject) {
        if !invertible(t.xf) {
            return;
        }
        let layout = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), t);
        let tb = t.xf.transform_rect_bbox(layout.bounds);
        let (below, above) = n.appearance.split_contents();
        let all = (!n.appearance.items.is_empty()).then(|| {
            let mut all = layout.to_bezpath();
            all.apply_affine(t.xf);
            all
        });
        if let Some(bp) = &all {
            self.text_items(below, bp, tb);
        }
        self.line("q");
        let _ = writeln!(self.body, "{} cm", ps::matrix(t.xf));
        for (i, run) in t.runs.iter().enumerate() {
            let mut bp = BezPath::new();
            // With the job's fonts, a run in a solid fill shows its glyphs through them; `art`
            // holds the outlines of those it draws as art.
            let fonts = if matches!(run.style.fill, Paint::Solid { .. }) { self.fonts } else { None };
            let mut shown = vec![];
            let mut art = BezPath::new();
            for g in layout.glyphs.iter().filter(|g| g.run == i) {
                bp.extend(g.outline.iter());
                match fonts.and_then(|f| self.font_of(f, g)) {
                    Some((face, upem)) => shown.push((face, upem, g)),
                    None if fonts.is_some() => art.extend(g.outline.iter()),
                    None => {}
                }
                // Colour bitmap glyphs (emoji): their images, which the run's paint doesn't paint.
                if let Some((image, to_text)) = g.placed_image() {
                    let key = format!("\u{0}glyph\u{0}{}", image.blob.content_key());
                    self.image(&key, Some(&image.blob), image.width, image.height, to_text);
                }
                // COLR glyphs: their layers, as art (in text space, as the images).
                if let Some(art) = g.placed_colour(Affine::IDENTITY) {
                    self.node(&art, true);
                }
            }
            // Ruby (furigana) glyphs in their base text's run's paint, as the canvas draws them.
            for g in layout.ruby.iter().filter(|g| g.run == i) {
                bp.extend(g.outline.iter());
                if fonts.is_some() {
                    art.extend(g.outline.iter());
                }
            }
            if bp.elements().is_empty() {
                continue;
            }
            let overprint = self.overprints(run.style.overprint_fill, &run.style.fill);
            if fonts.is_none() {
                self.fill(&bp, FillRule::NonZero, &run.style.fill, layout.bounds, overprint);
            } else {
                self.show(&shown, &run.style.fill, overprint);
                if !art.elements().is_empty() {
                    self.fill(&art, FillRule::NonZero, &run.style.fill, layout.bounds, overprint);
                }
            }
            if run.style.has_stroke() {
                // Character strokes are drawn in text space, with their cap, join and dashes.
                self.stroke(&bp, FillRule::NonZero, &run.style.stroke_layer(), layout.bounds);
            }
        }
        self.line("Q");
        if let Some(bp) = &all {
            self.text_items(above, bp, tb);
        }
    }

    /// Some of a type object's own fills and strokes on its glyph outlines `bp` (bounds `tb`).
    fn text_items(&mut self, items: &[AppearanceItem], bp: &BezPath, tb: Rect) {
        for item in items {
            match item {
                AppearanceItem::Fill(fl) if fl.visible && !fl.paint.is_none() => {
                    self.note_item(fl.opacity, fl.blend);
                    let overprint = self.overprints(fl.overprint, &fl.paint);
                    self.fill(bp, FillRule::NonZero, &fl.paint, tb, overprint);
                }
                AppearanceItem::Stroke(st) if st.visible && !st.paint.is_none() && st.width > 0.0 && st.width.is_finite() => {
                    self.stroke(bp, FillRule::NonZero, st, tb);
                }
                _ => {}
            }
        }
    }

    /// The job's index of the face of glyph `g` and its units per em when `g` is shown through
    /// the job's fonts `fonts` (recorded in `used`); `None` when it is drawn as an outline: a glyph
    /// with no outline, a color glyph, one bent along a path, or one its face's fonts don't show.
    fn font_of(&mut self, fonts: &RefCell<FontJob>, g: &vectorcraft_text::PositionedGlyph) -> Option<(usize, f64)> {
        if g.outline.elements().is_empty() || g.bend.is_some() || g.image.is_some() || g.colour.is_some() || !invertible(g.xf) {
            return None;
        }
        let (face, upem) = fonts.borrow_mut().glyph(g.font_id, g.gid, g.outline.elements().len())?;
        if self.used.entry(face).or_default().insert(g.gid) {
            self.used_order.push((face, g.gid));
        }
        Some((face, upem))
    }

    /// Show `glyphs` (the job's index of each one's face, its units per em, the glyph) through the
    /// job's fonts in solid paint `p`: each glyph at its own place, through a font made for its
    /// size and slant.
    fn show(&mut self, glyphs: &[(usize, f64, &vectorcraft_text::PositionedGlyph)], p: &Paint, overprint: bool) {
        let Paint::Solid { color, swatch, tint } = p else { return };
        if glyphs.is_empty() {
            return;
        }
        let on = self.overprint_on(overprint);
        let ink = self.solid(color, swatch.as_deref(), *tint);
        self.set_ink(ink);
        let mut current: Option<(String, [f64; 4])> = None;
        for (face, upem, g) in glyphs {
            // Font units (y up) → text space: the glyph's own map after the flip to y down.
            let [a, b, c, d, e, f] = (g.xf * Affine::scale_non_uniform(*upem, -*upem)).as_coeffs();
            let name = font_name(*face, g.gid >> 8);
            let key = (name, [a, b, c, d]);
            if current.as_ref() != Some(&key) {
                let _ = write!(self.body, "/{} findfont [", key.0);
                for v in [a, b, c, d] {
                    ps::push_num_places(&mut self.body, v, 6);
                    self.body.push(' ');
                }
                self.line("0 0] makefont setfont");
                current = Some(key);
            }
            push_nums(&mut self.body, &[e, f]);
            let _ = writeln!(self.body, "moveto <{:02x}> show", g.gid & 0xff);
        }
        self.overprint_off(on);
    }

    // ---------- gradients ----------

    /// The colour space a gradient's stops are written in, and its stops in it from offset 0 to 1
    /// (midpoints as stops of their own).
    fn gradient_stops(&mut self, g: &vectorcraft_color::Gradient) -> (GradientSpace, Stops) {
        if g.stops.iter().any(|s| s.opacity < 1.0) {
            self.warn(TRANSPARENCY);
        }
        let (space, raw) = match self.ink_stops(g) {
            Some(inks) => inks,
            None => {
                let (space, raw) = self.process_stops(g);
                (GradientSpace::Process(space), raw)
            }
        };
        let mut out: Stops = vec![];
        let mut last = 0.0f64;
        for (o, c) in raw {
            let o = if o.is_finite() { f64::from(o).clamp(last, 1.0) } else { last };
            last = o;
            out.push((o, c));
        }
        match (out.first().cloned(), out.last().cloned()) {
            (Some(first), Some(end)) => {
                if first.0 > 0.0 {
                    out.insert(0, (0.0, first.1));
                }
                if end.0 < 1.0 {
                    out.push((1.0, end.1));
                }
            }
            _ => out = vec![(0.0, vec![0.0; space.n()]), (1.0, vec![0.0; space.n()])],
        }
        (space, out)
    }

    /// The stops of a gradient with a spot ink as tints of its inks: a Separation space when every
    /// stop is a tint of one spot ink or paper white, else (at Level 3) a DeviceN space of the
    /// process inks the stops use, in CMYK order, and the spot inks, in the order the stops name
    /// them. Paper white adds no ink, a gray stop is black ink, and a spot stop's process plates
    /// print with its ink. An off-center midpoint is a stop
    /// with the inks halfway between its two stops. Stop opacity is left out, so the stops that
    /// color a stretch ([`vectorcraft_color::Gradient::reached_stops`]) name the inks, a spot ink
    /// only above a tint of 0 or with process plates; the others are written with no ink. `None` for a gradient without a
    /// spot ink, and, with a warning, for one whose inks can't be kept or that is mixed by the
    /// Perceptual method or around the hue wheel; it is then written in process colors.
    fn ink_stops(&mut self, g: &vectorcraft_color::Gradient) -> Option<(GradientSpace, RawStops)> {
        let doc = self.doc;
        let spot_of = |s: &vectorcraft_color::GradientStop| -> Option<String> {
            s.swatch.as_deref().filter(|n| doc.swatch(n).is_some_and(|w| w.spot)).map(str::to_string)
        };
        let reached = g.reached_stops();
        // The spot inks, collected up to one more than a DeviceN space can hold.
        let mut names: Vec<String> = vec![];
        for (s, &r) in g.stops.iter().zip(&reached) {
            if let Some(n) = spot_of(s).filter(|_| r && (s.tint > 0.0 || s.plates.is_some()))
                && !names.contains(&n)
            {
                names.push(n);
                if names.len() > MAX_DEVICE_N {
                    break;
                }
            }
        }
        if names.is_empty() {
            return None;
        }
        if !g.method.is_classic() {
            self.warn(if g.method.is_hue() { SPOT_GRADIENTS_HUE } else { SPOT_GRADIENTS_PERCEPTUAL });
            return None;
        }
        if names.len() > MAX_DEVICE_N {
            self.warn(SPOT_GRADIENTS_SHARED);
            return None;
        }
        // Paper white exactly: any ink at all, however little, is kept.
        let paper = |c: &Color| match *c {
            Color::Gray { k } => k <= 0.0,
            Color::Cmyk { c, m, y, k } => [c, m, y, k].iter().all(|&v| v <= 0.0),
            Color::Rgb { r, g, b } => [r, g, b].iter().all(|&v| v >= 1.0),
            Color::Lab { l, a, b } => l >= 100.0 && a == 0.0 && b == 0.0,
        };
        // Each stop's ink coverage: the four process inks, then one tint per spot ink.
        let mut inks: Vec<Vec<f32>> = Vec::with_capacity(g.stops.len());
        for (s, &r) in g.stops.iter().zip(&reached) {
            let mut v = vec![0.0; 4 + names.len()];
            match spot_of(s) {
                _ if !r => {}
                Some(n) => {
                    if let Some(i) = names.iter().position(|m| *m == n) {
                        v[4 + i] = unit(s.tint);
                    }
                    // Process plates printed with the spot ink (a DeviceN mix from a file).
                    if let Some(p) = s.plates {
                        v[..4].copy_from_slice(&p.map(unit));
                    }
                }
                None if paper(&s.color) => {}
                None => match self.process(&s.color) {
                    Ink::Process(Space::Cmyk, c) => v[..4].copy_from_slice(&c),
                    Ink::Process(Space::Gray, [lightness, ..]) => v[3] = unit(1.0 - lightness),
                    _ => {
                        self.warn(SPOT_GRADIENTS_RGB);
                        return None;
                    }
                },
            }
            inks.push(v);
        }
        let process: Vec<usize> = (0..4).filter(|&i| inks.iter().any(|v| v[i] > 0.0)).collect();
        let space = if let ([name], true) = (names.as_slice(), process.is_empty()) {
            let Some(i) = self.spot_space(name) else {
                self.warn(SPOT_GRADIENTS_SHARED);
                return None;
            };
            GradientSpace::Inks(format!("VCsp{i}"), 1)
        } else if self.o.level != Level::Three {
            self.warn(SPOT_GRADIENTS_L2);
            return None;
        } else {
            let Some(space) = self.device_n_space(&process, &names) else {
                self.warn(SPOT_GRADIENTS_SHARED);
                return None;
            };
            space
        };
        let pick = |v: &[f32]| -> Vec<f32> { process.iter().map(|&i| v[i]).chain(v[4..].iter().copied()).collect() };
        let mut out = vec![];
        for (i, s) in g.stops.iter().enumerate() {
            out.push((s.offset, pick(&inks[i])));
            if let (Some(n), Some(next)) = (g.stops.get(i + 1), inks.get(i + 1))
                && (s.midpoint - 0.5).abs() > 1e-3
            {
                let half: Vec<f32> = inks[i].iter().zip(next).map(|(a, b)| (a + b) / 2.0).collect();
                out.push((s.offset + (n.offset - s.offset) * s.midpoint, pick(&half)));
            }
        }
        Some((space, out))
    }

    /// A DeviceN color space of process inks `process` (indices into CMYK) and spot inks `names`,
    /// defined in the setup the first time, each spot ink under the colorant name and with the
    /// CMYK equivalent its Separation space has. `None` when a spot ink has no Separation space,
    /// two colorants share a name, a spot ink's colorant has the name of a process ink, `All` or
    /// `None`, or there are more than [`MAX_DEVICE_N`] colorants.
    fn device_n_space(&mut self, process: &[usize], names: &[String]) -> Option<GradientSpace> {
        const PROCESS: [&str; 4] = ["Cyan", "Magenta", "Yellow", "Black"];
        if process.len() + names.len() > MAX_DEVICE_N {
            return None;
        }
        let mut colorants: Vec<String> = process.iter().map(|&i| PROCESS[i].to_string()).collect();
        let mut weights: Vec<[f32; 4]> = process.iter().map(|&i| [0, 1, 2, 3].map(|j| if i == j { 1.0 } else { 0.0 })).collect();
        let spots: Vec<String> = names.iter().map(|n| self.colorant_name(n)).collect();
        for (i, name) in spots.iter().enumerate() {
            if PROCESS.contains(&name.as_str()) || name == "All" || name == "None" || spots[..i].contains(name) {
                return None;
            }
        }
        for name in names {
            let i = self.spot_space(name)?;
            let (_, colorant, cmyk) = self.spots.get(i)?;
            colorants.push(colorant.clone());
            weights.push(*cmyk);
        }
        let mut def = String::from("[/DeviceN [");
        for (i, c) in colorants.iter().enumerate() {
            if i < process.len() {
                let _ = write!(def, "/{c} ");
            } else {
                let _ = write!(def, "{} cvn ", ps::string(c));
            }
        }
        // The tint transform: the components into an array, then each CMYK ink the sum of the
        // components times their amounts of it.
        let n = weights.len();
        let _ = write!(def, "] /DeviceCMYK {{{n} array astore");
        for j in 0..4 {
            def.push_str(" 0");
            for (i, w) in weights.iter().enumerate().filter(|(_, w)| w[j] > 0.0) {
                let _ = write!(def, " {} index {i} get {} mul add", j + 1, ps::num(f64::from(w[j])));
            }
        }
        def.push_str(" 5 -1 roll pop}]");
        let i = match self.device_n.iter().position(|d| *d == def) {
            Some(i) => i,
            None => {
                let i = self.device_n.len();
                let _ = writeln!(self.setup, "/VCdn{i} {def} def");
                self.device_n.push(def);
                i
            }
        };
        Some(GradientSpace::Inks(format!("VCdn{i}"), n))
    }

    /// The process color space a gradient's stops are written in, and its stops in it (midpoints
    /// as stops of their own).
    fn process_stops(&mut self, g: &vectorcraft_color::Gradient) -> (Space, RawStops) {
        let stops = g.expanded_stops();
        let space = if self.cmyk || stops.iter().any(|(_, c, _)| matches!(c, Color::Cmyk { .. })) {
            Space::Cmyk
        } else if stops.iter().all(|(_, c, _)| matches!(c, Color::Gray { .. })) {
            Space::Gray
        } else {
            Space::Rgb
        };
        let comps = |c: &Color| -> [f32; 4] {
            match (space, *c) {
                (Space::Cmyk, _) => self.cms.to_cmyk(c, self.cms.settings().intent).map(unit),
                (Space::Gray, Color::Gray { k }) => [unit(1.0 - k), 0.0, 0.0, 0.0],
                (_, Color::Rgb { r, g, b }) => [r, g, b, 0.0].map(unit),
                _ => {
                    let [r, g, b] = c.to_rgb();
                    [r, g, b, 0.0].map(unit)
                }
            }
        };
        let out = stops.iter().map(|(o, c, _)| (*o, comps(c)[..space.n()].to_vec())).collect();
        (space, out)
    }

    /// Paint all of `area` (inside the current clip) with linear, radial, conic or diamond gradient
    /// `g`, placed on `bounds`: a smooth shading at Level 3, else stepped fills (a conic: wedges at
    /// both levels; a diamond: four linear gradients, [`Self::diamond_quadrants`]). Overprinting
    /// on a plate, it is clipped to the levels where it puts ink down ([`Self::inked_spans`]).
    fn gradient(&mut self, g: &GradientPaint, bounds: Rect, area: Rect) {
        let geom = g.resolve(bounds);
        // One mixed around the hue wheel that reflects is repeated from its curve's stops, which
        // keep no swatch links: the warning [`Self::ink_stops`] gives the others.
        if g.gradient.method.is_hue() && g.gradient.is_spread() && g.gradient.spread == vectorcraft_color::GradientSpread::Reflect {
            let doc = self.doc;
            let spot = |s: &vectorcraft_color::GradientStop| {
                (s.tint > 0.0 || s.plates.is_some()) && s.swatch.as_deref().is_some_and(|n| doc.swatch(n).is_some_and(|w| w.spot))
            };
            if g.gradient.stops.iter().zip(g.gradient.reached_stops()).any(|(s, r)| r && spot(s)) {
                self.warn(SPOT_GRADIENTS_HUE);
            }
        }
        // A linear, radial or diamond gradient that repeats or reflects: PostScript's shadings
        // pad past their ends, so its stops are repeated as many times as the area needs.
        if let Some(padded) = g.gradient.padded_over(&geom, area) {
            if padded.capped {
                self.warn(SPREAD_CAPPED);
            }
            let geom = padded.geom;
            let paint = GradientPaint { gradient: padded.gradient, geom: Some(geom), angle: geom.angle_deg(), ..g.clone() };
            return self.gradient(&paint, bounds, area);
        }
        // One whose levels over the area are not finite (a vector of no length): its stops
        // repeated along the vector, padding past it.
        if g.gradient.is_spread() && g.gradient.kind != GradientKind::Conic {
            self.warn(SPREAD_CAPPED);
            let paint = GradientPaint { gradient: g.gradient.unrolled().into_owned(), ..g.clone() };
            return self.gradient(&paint, bounds, area);
        }
        let (space, stops) = self.gradient_stops(&g.gradient);
        if !stops.windows(2).any(|w| w[1].0 > w[0].0) {
            if let Some((_, c)) = stops.first().filter(|(_, c)| !(self.keep_below && space.paper(c))) {
                self.set_comps(&space, c);
                self.rect_fill(area);
            }
            return;
        }
        let space = &space;
        let smooth = self.o.level == Level::Three && !self.o.compatible_gradients;
        let spans = if g.gradient.kind == GradientKind::Conic { None } else { self.inked_spans(space, &stops, smooth) };
        match g.gradient.kind {
            GradientKind::Conic => self.conic_steps(&geom, area, space, &stops, &g.gradient),
            GradientKind::Diamond => {
                // Its levels are diamonds about the center, the square's edge at 1.
                let Some(frame) = geom.diamond_frame() else {
                    // Without a frame it is its color at 1 everywhere; overprinting on a plate,
                    // paper white there leaves the ink below.
                    if !(self.keep_below && frameless_color(&stops).is_some_and(|c| space.paper(c))) {
                        self.diamond_quadrants(&geom, area, space, &stops, smooth);
                    }
                    return;
                };
                if let Some(spans) = &spans {
                    let level = |t: f64| (t > 0.0).then(|| frame * BezPath::from_vec(diamond(t)));
                    self.line("q");
                    let clipped = self.clip_rings(spans, area.inflate(1.0, 1.0), level);
                    if clipped {
                        self.diamond_quadrants(&geom, area, space, &stops, smooth);
                    }
                    self.line("Q");
                    return;
                }
                self.diamond_quadrants(&geom, area, space, &stops, smooth);
            }
            GradientKind::Radial => {
                let r = geom.length().max(1e-6);
                let squash = geom.radial_squash();
                let inv = squash.inverse();
                let (f, c) = (inv * geom.focal_point(), geom.start);
                let local = inv.transform_rect_bbox(area);
                self.line("q");
                let _ = writeln!(self.body, "{} cm", ps::matrix(squash));
                // Its levels are circles from the focal point, at 1 the circle of radius `r`.
                let level = |t: f64| (t > 0.0).then(|| kurbo::Circle::new(f + (c - f) * t, r * t).to_path(0.01));
                if spans.as_ref().is_none_or(|spans| self.clip_rings(spans, local.inflate(1.0, 1.0), level)) {
                    if smooth {
                        self.shading(3, space, &[f.x, f.y, 0.0, c.x, c.y, r], &stops);
                    } else {
                        self.radial_steps(f, c, r, local, space, &stops);
                    }
                }
                self.line("Q");
            }
            _ => {
                let (s, mut e) = (geom.start, geom.end);
                if s.distance(e) < 1e-9 {
                    e = s + Vec2::new(1.0, 0.0);
                }
                if let Some(spans) = &spans {
                    self.line("q");
                    if self.clip_strips(spans, s, e, area) {
                        self.linear(s, e, area, space, &stops, smooth);
                    }
                    self.line("Q");
                } else {
                    self.linear(s, e, area, space, &stops, smooth);
                }
            }
        }
    }

    /// A linear gradient from `s` to `e` over `area`: a smooth shading, else stepped fills.
    fn linear(&mut self, s: Point, e: Point, area: Rect, space: &GradientSpace, stops: &[(f64, Vec<f32>)], smooth: bool) {
        if smooth {
            self.shading(2, space, &[s.x, s.y, e.x, e.y], stops);
        } else {
            self.linear_steps(s, e, area, space, stops);
        }
    }

    /// Where an overprinting gradient with `stops` in `space` puts ink on a plate
    /// ([`Self::keep_below`]): the spans of its levels outside every stretch between two stops of
    /// paper white and outside the padding past an end painted paper white, in order, an end
    /// padding with ink unbounded. The padding takes the end stop's color when stepped and the
    /// end stretch's color when `smooth` ([`Self::shading`]); the two differ when two stops share
    /// the end's offset. `None` when the gradient puts ink everywhere or does not overprint on a
    /// plate; empty when it puts none down.
    fn inked_spans(&self, space: &GradientSpace, stops: &[(f64, Vec<f32>)], smooth: bool) -> Option<Vec<(f64, f64)>> {
        if !self.keep_below {
            return None;
        }
        let stretches: Vec<_> = stops.windows(2).filter(|w| w[1].0 > w[0].0).collect();
        let (first, last) = (stretches.first()?, stretches.last()?);
        let (start, end) = if smooth { (&first[0].1, &last[1].1) } else { (&stops.first()?.1, &stops.last()?.1) };
        let (before, after) = (space.paper(start), space.paper(end));
        let mut blank: Vec<(f64, f64)> = vec![];
        let mut add = |a: f64, b: f64| match blank.last_mut() {
            Some(l) if l.1 >= a => l.1 = l.1.max(b),
            _ => blank.push((a, b)),
        };
        if before {
            add(f64::NEG_INFINITY, first[0].0);
        }
        for w in &stretches {
            if space.paper(&w[0].1) && space.paper(&w[1].1) {
                add(w[0].0, w[1].0);
            }
        }
        if after {
            add(last[1].0, f64::INFINITY);
        }
        if blank.is_empty() {
            return None;
        }
        let mut inked = vec![];
        let mut from = f64::NEG_INFINITY;
        for (a, b) in blank {
            if a > from {
                inked.push((from, a));
            }
            from = b;
        }
        if from < f64::INFINITY {
            inked.push((from, f64::INFINITY));
        }
        Some(inked)
    }

    /// Clip to the parts of `area` where a linear gradient from `s` to `e` has a level in `spans`:
    /// a strip across the gradient for each. False, with nothing written, when no span reaches
    /// into `area`.
    fn clip_strips(&mut self, spans: &[(f64, f64)], s: Point, e: Point, area: Rect) -> bool {
        let d = e - s;
        let len = d.hypot();
        // Gradient space: x along the gradient (0 to 1), y across it in points.
        let frame = Affine::new([d.x, d.y, -d.y / len, d.x / len, s.x, s.y]);
        let local = frame.inverse().transform_rect_bbox(area.inflate(1.0, 1.0));
        let mut bp = BezPath::new();
        for &(a, b) in spans {
            let (a, b) = (a.max(local.x0), b.min(local.x1));
            if a < b {
                bp.extend((frame * Rect::new(a, local.y0, b, local.y1).to_path(0.1)).iter());
            }
        }
        self.clip_path(&bp, FillRule::NonZero)
    }

    /// Clip to the levels in `spans` of a gradient whose level `t` is bounded by `level(t)` (a
    /// closed path holding every lower level; `None` for a level without area): for each span
    /// the ring between its levels, an unbounded end at `outer`, even-odd. False, with nothing
    /// written, when no span has area.
    fn clip_rings(&mut self, spans: &[(f64, f64)], outer: Rect, level: impl Fn(f64) -> Option<BezPath>) -> bool {
        let mut bp = BezPath::new();
        for &(a, b) in spans {
            let edge = if b.is_finite() { level(b) } else { Some(outer.to_path(0.1)) };
            let Some(edge) = edge else { continue };
            bp.extend(edge.iter());
            if let Some(inner) = level(a).filter(|_| a.is_finite()) {
                bp.extend(inner.iter());
            }
        }
        self.clip_path(&bp, FillRule::EvenOdd)
    }

    /// Clip to `bp` by `rule`. False, with nothing written, when `bp` is empty.
    fn clip_path(&mut self, bp: &BezPath, rule: FillRule) -> bool {
        if bp.elements().is_empty() {
            return false;
        }
        push_path(&mut self.body, bp);
        self.body.push_str(clip_op(rule));
        true
    }

    /// A conic gradient as [`CONIC_WEDGE_COUNT`] solid wedges about its center covering `area`
    /// (PostScript has no conic shading), each running on to the seam under the next, so no
    /// hairline shows between neighbors ([`vectorcraft_color::GradientGeom::conic_wedges`]).
    /// Stops of `g` that repeat or reflect get that many wedges for each time they run around.
    fn conic_steps(
        &mut self,
        geom: &vectorcraft_color::GradientGeom,
        area: Rect,
        space: &GradientSpace,
        stops: &[(f64, Vec<f32>)],
        g: &vectorcraft_color::Gradient,
    ) {
        self.warn(CONIC_WEDGES);
        let spread = g.spreader();
        let at = |t: f64| match stops.windows(2).find(|w| t <= w[1].0) {
            Some(w) if w[1].0 > w[0].0 => mix(&w[0].1, &w[1].1, ((t - w[0].0) / (w[1].0 - w[0].0)).clamp(0.0, 1.0) as f32),
            Some(w) => w[1].1.clone(),
            None => stops.last().map_or_else(Vec::new, |s| s.1.clone()),
        };
        let count = g.conic_wedge_count(CONIC_WEDGE_COUNT);
        // Overprinting on a plate: clipped to the wedges that put ink down, each up to the next.
        let clipped = self.keep_below && {
            let wedges = geom.conic_wedges(area, count, false);
            let inked: Vec<_> = wedges.iter().filter(|(_, t)| !space.paper(&at(spread(*t)))).collect();
            if inked.is_empty() {
                return;
            }
            inked.len() < wedges.len() && {
                let mut bp = BezPath::new();
                for (pts, _) in inked {
                    bp.extend(BezPath::from_vec(polygon(pts)).iter());
                }
                self.line("q");
                self.clip_path(&bp, FillRule::NonZero)
            }
        };
        for (pts, t) in geom.conic_wedges(area, count, true) {
            self.set_comps(space, &at(spread(t)));
            for (i, p) in pts.iter().enumerate() {
                push_nums(&mut self.body, &[p.x, p.y]);
                self.line(if i == 0 { "newpath moveto" } else { "lineto" });
            }
            self.line("closepath fill");
        }
        if clipped {
            self.line("Q");
        }
    }

    /// A diamond gradient over `area` as four linear gradients, each clipped to a quadrant of its
    /// frame and equal to the diamond there (PostScript has no diamond shading;
    /// [`vectorcraft_color::GradientGeom::diamond_quadrants`]): smooth axial shadings, else
    /// stepped fills. Each quadrant but the last runs on under the next (stops are opaque here),
    /// so no hairline shows between them. A diamond without an extent is its color at 1
    /// ([`frameless_color`]).
    fn diamond_quadrants(
        &mut self,
        geom: &vectorcraft_color::GradientGeom,
        area: Rect,
        space: &GradientSpace,
        stops: &[(f64, Vec<f32>)],
        smooth: bool,
    ) {
        let Some(quadrants) = geom.diamond_quadrants(area) else {
            if let Some(c) = frameless_color(stops) {
                self.set_comps(space, c);
                self.rect_fill(area);
            }
            return;
        };
        self.warn(DIAMOND_QUADRANTS);
        for (pts, q) in quadrants {
            self.line("q");
            for (i, p) in pts.iter().enumerate() {
                push_nums(&mut self.body, &[p.x, p.y]);
                self.line(if i == 0 { "newpath moveto" } else { "lineto" });
            }
            self.line("closepath clip newpath");
            if smooth {
                self.shading(2, space, &[q.start.x, q.start.y, q.end.x, q.end.y], stops);
            } else {
                self.linear_steps(q.start, q.end, area, space, stops);
            }
            self.line("Q");
        }
    }

    /// A Level 3 smooth shading of `kind` (2 axial, 3 radial) at `coords`, extended both ways.
    fn shading(&mut self, kind: u8, space: &GradientSpace, coords: &[f64], stops: &[(f64, Vec<f32>)]) {
        let kind = if kind == 3 { ps::ShadingKind::Radial } else { ps::ShadingKind::Axial };
        let name = match space {
            GradientSpace::Process(s) => s.name(),
            GradientSpace::Inks(name, _) => name,
        };
        ps::push_shading(&mut self.body, kind, name, coords, space.n(), stops);
    }

    /// The bands of each stretch between stops: how many, for colours `a` to `b` over `length`
    /// points (one per colour level, none thinner than [`MIN_BAND`]).
    fn bands(a: &[f32], b: &[f32], length: f64, segments: usize) -> usize {
        let delta = a.iter().zip(b).map(|(x, y)| (x - y).abs()).fold(0.0f32, f32::max);
        let by_color = (f64::from(delta) * 255.0).ceil().max(1.0) as usize;
        let by_size = (length / MIN_BAND).ceil().max(1.0) as usize;
        by_color.min(by_size).min(MAX_STEPS / segments.max(1)).max(1)
    }

    fn set_comps(&mut self, space: &GradientSpace, c: &[f32]) {
        match space {
            GradientSpace::Process(s) => {
                let mut v = [0.0; 4];
                for (x, y) in v.iter_mut().zip(c) {
                    *x = *y;
                }
                self.set_ink(Ink::Process(*s, v));
            }
            GradientSpace::Inks(name, _) => {
                let _ = write!(self.body, "{name} setcolorspace ");
                push_nums(&mut self.body, &c.iter().map(|v| f64::from(*v)).collect::<Vec<_>>());
                self.line("setcolor");
            }
        }
    }

    /// A linear gradient from `s` to `e` as bands across it, covering `area`: each band painted
    /// from where it starts to the far end, so the next one leaves no seam.
    fn linear_steps(&mut self, s: Point, e: Point, area: Rect, space: &GradientSpace, stops: &[(f64, Vec<f32>)]) {
        let d = e - s;
        let len = d.hypot();
        // Gradient space: x along the gradient (0 to 1), y across it in points.
        let frame = Affine::new([d.x, d.y, -d.y / len, d.x / len, s.x, s.y]);
        let local = frame.inverse().transform_rect_bbox(area.inflate(1.0, 1.0));
        let (t0, t1) = (local.x0.min(0.0), local.x1.max(1.0));
        self.line("q");
        let _ = writeln!(self.body, "{} cm", ps::matrix(frame));
        let band = |sc: &mut Self, from: f64| {
            push_nums(&mut sc.body, &[from, local.y0, t1 - from, local.height()]);
            sc.line("rectfill");
        };
        if let Some((_, c)) = stops.first() {
            self.set_comps(space, c);
            band(self, t0);
        }
        let segments: Vec<_> = stops.windows(2).filter(|w| w[1].0 > w[0].0).collect();
        for w in &segments {
            let ((oa, ca), (ob, cb)) = (&w[0], &w[1]);
            let n = Self::bands(ca, cb, (ob - oa) * len, segments.len());
            for k in 0..n {
                let at = oa + (ob - oa) * k as f64 / n as f64;
                self.set_comps(space, &mix(ca, cb, (k as f32 + 0.5) / n as f32));
                band(self, at);
            }
        }
        if let Some((_, c)) = stops.last() {
            self.set_comps(space, c);
            band(self, 1.0);
        }
        self.line("Q");
    }

    /// A radial gradient from focal point `f` to the circle of radius `r` around `c` as discs,
    /// largest first, over the last colour across `area` (all in the gradient's own space).
    fn radial_steps(&mut self, f: Point, c: Point, r: f64, area: Rect, space: &GradientSpace, stops: &[(f64, Vec<f32>)]) {
        if let Some((_, last)) = stops.last() {
            self.set_comps(space, last);
            self.rect_fill(area);
        }
        let segments: Vec<_> = stops.windows(2).filter(|w| w[1].0 > w[0].0).collect();
        for w in segments.iter().rev() {
            let ((oa, ca), (ob, cb)) = (&w[0], &w[1]);
            let n = Self::bands(ca, cb, (ob - oa) * r, segments.len());
            for k in (0..n).rev() {
                let t = oa + (ob - oa) * (k + 1) as f64 / n as f64;
                self.set_comps(space, &mix(ca, cb, (k as f32 + 0.5) / n as f32));
                let at = f + (c - f) * t;
                push_nums(&mut self.body, &[at.x, at.y, r * t]);
                self.line("0 360 newpath arc fill");
            }
        }
    }

    // ---------- images ----------

    /// Paint all of `area` with an image of freeform gradient `g` placed on `bounds`, sampled at
    /// the document's raster effects resolution (at most [`MAX_FIELD_PX`] a side).
    fn field_image(&mut self, g: &GradientPaint, bounds: Rect, area: Rect) {
        use vectorcraft_color::cms::Model;
        use vectorcraft_color::freeform::{painted_box, spread_scale};
        let Some(b) = painted_box(bounds) else { return };
        let area = area.abs();
        let long = area.width().max(area.height());
        if !(long > 1e-9 && long.is_finite()) {
            return;
        }
        let place = Affine::translate(area.origin().to_vec2()) * Affine::scale_non_uniform(area.width(), area.height());
        if !invertible(place) {
            return;
        }
        if self.size().saturating_add(64) > self.max_body {
            self.warn(TOO_LARGE);
            return;
        }
        let k = (self.doc.raster_effects_ppi / 72.0).min(MAX_FIELD_PX / long);
        let along = |len: f64| (len * k).ceil().clamp(1.0, MAX_FIELD_PX) as u16;
        let (cols, rows) = (along(area.width()), along(area.height()));
        let field = g.freeform_on(b).model_field(spread_scale(b));
        let q = |v: f32| (unit(v) * 255.0).round() as u8;
        let (w, h) = (u32::from(cols), u32::from(rows));
        let in_inks = field.model() == Some(Model::Cmyk) || (self.cmyk && field.model() == Some(Model::Gray));
        // Each cell's samples: its inks or its RGB, then its alpha.
        let (mut samples, paper): (Vec<u8>, u8) = if in_inks {
            let inks = field.grid(area, cols, rows).flat_map(|(color, a)| {
                let [c, m, y, k] = color.to_cmyk();
                [q(c), q(m), q(y), q(k), q(a)]
            });
            (inks.collect(), 0)
        } else {
            (field.display_grid(area, cols, rows).flat_map(|(c, a)| [q(c[0]), q(c[1]), q(c[2]), q(a)]).collect(), 255)
        };
        let n = if in_inks { 5 } else { 4 };
        // Overprinting on a plate: the cells whose samples put none of the ink down once their
        // opacity is applied, as `encode_inks` and `encode_image` write them (RGB converted to
        // CMYK in CMYK output), are left out, as clear pixels at Level 3 and outside a clip of the
        // other cells at Level 2.
        let written = |v: u8, a: u8| {
            let (v, a) = (u32::from(v), u32::from(a));
            ((v * a + u32::from(paper) * (255 - a) + 127) / 255) as u8
        };
        let (cms, to_cmyk) = (self.cms.clone(), !in_inks && self.cmyk);
        let intent = cms.settings().intent;
        let mut converted: HashMap<[u8; 3], bool> = HashMap::new();
        let blank: Option<Vec<bool>> = self.keep_below.then(|| {
            samples
                .chunks(n)
                .map(|s| {
                    let a = s.last().copied().unwrap_or(0);
                    if a == 0 || in_inks {
                        return s.iter().take(n - 1).all(|&v| written(v, a) == paper);
                    }
                    let rgb = [0, 1, 2].map(|i| s.get(i).map_or(paper, |&v| written(v, a)));
                    if !to_cmyk {
                        return rgb == [paper; 3];
                    }
                    *converted.entry(rgb).or_insert_with(|| {
                        let inks = cms.srgb_to_cmyk(rgb.map(|x| f32::from(x) / 255.0), intent);
                        inks.iter().all(|&x| (unit(x) * 255.0).round() as u8 == 0)
                    })
                })
                .collect()
        });
        let blank = blank.filter(|b| b.contains(&true));
        if blank.as_ref().is_some_and(|b| !b.contains(&false)) {
            return;
        }
        let keyed = self.o.level == Level::Three;
        if let Some(b) = blank.as_ref().filter(|_| keyed) {
            for (s, _) in samples.chunks_mut(n).zip(b).filter(|(_, blank)| **blank) {
                if let Some(a) = s.last_mut() {
                    *a = 0;
                }
            }
        }
        let image = if in_inks { self.encode_inks(&samples, w, h) } else { self.encode_image(&samples, w, h) };
        let Some(image) = image else { return };
        let cells = blank.filter(|_| !keyed).map(|b| {
            let (cw, ch) = (area.width() / f64::from(cols), area.height() / f64::from(rows));
            let mut bp = BezPath::new();
            for (row, line) in b.chunks(usize::from(cols)).enumerate() {
                let mut col = 0;
                while col < line.len() {
                    let start = col;
                    while col < line.len() && !line[col] {
                        col += 1;
                    }
                    if col > start {
                        let (y0, y1) = (area.y0 + row as f64 * ch, area.y0 + (row + 1) as f64 * ch);
                        bp.extend(Rect::new(area.x0 + start as f64 * cw, y0, area.x0 + col as f64 * cw, y1).to_path(0.1).iter());
                    }
                    col += 1;
                }
            }
            bp
        });
        if let Some(bp) = &cells {
            // Each row's run of cells is a rectangle of five path elements, about 60 bytes. The
            // image is a reference when one encoded the same is already drawn.
            let cost = self
                .kept_field(&image)
                .and_then(|i| self.codes.get(i))
                .map_or(image.inline_len, |c| if c.uses == 0 { c.inline_len } else { c.ref_len });
            if self.size().saturating_add(bp.elements().len().saturating_mul(12)).saturating_add(cost) > self.max_body {
                self.warn(TOO_LARGE);
                return;
            }
            self.line("q");
            self.clip_path(bp, FillRule::NonZero);
        }
        self.place_field(image, place);
        if cells.is_some() {
            self.line("Q");
        }
    }

    /// Place freeform gradient image `image` on the unit square mapped by `place`, drawn from an
    /// image already kept when one is encoded the same.
    fn place_field(&mut self, image: ImageCode, place: Affine) {
        let key = image.content_key();
        let i = match self.kept_field(&image) {
            Some(i) => i,
            None => {
                // A field that can't be placed isn't kept.
                if self.size().saturating_add(image.inline_len).saturating_add(64) > self.max_body {
                    self.warn(TOO_LARGE);
                    return;
                }
                let i = self.add_code(image);
                self.fields.entry(key).or_default().push(i);
                i
            }
        };
        self.place_image(i, place);
    }

    /// The index in `codes` of a kept freeform gradient image encoded the same as `image`.
    fn kept_field(&self, image: &ImageCode) -> Option<usize> {
        let bucket = self.fields.get(&image.content_key())?;
        bucket.iter().copied().find(|&i| self.codes.get(i).is_some_and(|c| c.same_image(image)))
    }

    /// Forget the encoded images after the first `n` codes that no placement draws any longer, and
    /// the cache entries that name them: placed images, colorized images and fields. A code that a
    /// placement still draws keeps itself and every code before it.
    fn forget_codes(&mut self, n: usize) {
        let keep = self.codes.iter().rposition(|c| c.uses > 0).map_or(n, |last| n.max(last + 1));
        if keep >= self.codes.len() {
            return;
        }
        self.codes.truncate(keep);
        self.images.retain(|_, e| e.is_none_or(|i| i < keep));
        self.colorized_images.retain(|_, e| e.is_none_or(|i| i < keep));
        self.fields.retain(|_, bucket| {
            bucket.retain(|&i| i < keep);
            !bucket.is_empty()
        });
    }

    /// A placed image (`blob`, cached under `key`): its pixel grid of `width` × `height` placed
    /// by `xf` (each image file encoded once).
    fn image(&mut self, key: &str, blob: Option<&ImageBlob>, width: u32, height: u32, xf: Affine) {
        let place = xf * Affine::scale_non_uniform(f64::from(width.max(1)), f64::from(height.max(1)));
        if !invertible(place) {
            return;
        }
        let encoded = match self.images.get(key) {
            Some(e) => *e,
            None => {
                // An image turned away at the page limit is encoded again only once it fits.
                if let Some(&len) = self.too_large_images.get(key) {
                    if self.size().saturating_add(len).saturating_add(64) > self.max_body {
                        self.warn(TOO_LARGE);
                        return;
                    }
                    self.too_large_images.remove(key);
                }
                #[cfg(test)]
                {
                    self.encodes += 1;
                }
                let code = self.decode_image(blob).and_then(|img| self.encode_image(img.as_raw(), img.width(), img.height()));
                // An image that can't be placed isn't kept.
                if let Some(c) = &code
                    && self.size().saturating_add(c.inline_len).saturating_add(64) > self.max_body
                {
                    self.too_large_images.insert(key.to_string(), c.inline_len);
                    self.warn(TOO_LARGE);
                    return;
                }
                let e = code.map(|c| self.add_code(c));
                self.images.insert(key.to_string(), e);
                e
            }
        };
        if let Some(i) = encoded {
            self.place_image(i, place);
        }
    }

    /// Keep `code` for the placements that draw it: its index in `codes`.
    fn add_code(&mut self, mut code: ImageCode) -> usize {
        let i = self.codes.len();
        if self.o.level == Level::Three {
            code.data = vec![];
        }
        code.ref_len = code.reference(i, self.o.level).len();
        self.codes.push(code);
        i
    }

    /// Image `im` (its pixels `blob`) as gray coverage in the selected process or Separation ink.
    /// Binary coverage is a stencil; continuous coverage leaves zero pixels untouched at Level 3
    /// and writes them as paper at Level 2. With `overprint` a CMYK or gray color is written in only the
    /// process inks it puts down ([`Self::overprint_passes`]), since a zero sample in an image
    /// replaces the ink below it.
    fn colorized_image(&mut self, im: &vectorcraft_doc::ImageObject, blob: Option<&ImageBlob>, paint: &Paint, overprint: bool) {
        let place = im.xf * Affine::scale_non_uniform(f64::from(im.width.max(1)), f64::from(im.height.max(1)));
        if !invertible(place) {
            return;
        }
        match overprint.then(|| self.overprint_passes(paint)).flatten() {
            Some(passes) => {
                // A pass turned away (at the page limit) takes back the passes before it, so the
                // image is never written with some of its inks.
                let (was, placed, codes) = (self.body.len(), self.placements.len(), self.codes.len());
                for pass in &passes {
                    let before = self.placements.len();
                    self.colorized_pass(&im.key, blob, place, paint, Some(pass));
                    if self.placements.len() == before {
                        self.body.truncate(was);
                        self.unplace(placed);
                        self.forget_codes(codes);
                        return;
                    }
                }
            }
            None => self.colorized_pass(&im.key, blob, place, paint, None),
        }
    }

    /// The color spaces an overprinting colorized image of `paint` is drawn in, each with the
    /// amounts of its inks, when `paint` is a CMYK or gray process color (gray is black ink): at
    /// Level 3 one DeviceN space of the process inks it puts down, at Level 2 a Separation space
    /// for each of them. No pass when it puts no ink down. `None` for another paint, which is drawn
    /// as it is without overprinting.
    fn overprint_passes(&mut self, paint: &Paint) -> Option<Vec<InkPass>> {
        let Paint::Solid { color, swatch, tint } = paint else { return None };
        let c = match self.solid(color, swatch.as_deref(), *tint) {
            Ink::Process(Space::Cmyk, c) => c,
            // PostScript's gray is lightness.
            Ink::Process(Space::Gray, [lightness, ..]) => [0.0, 0.0, 0.0, unit(1.0 - lightness)],
            _ => return None,
        };
        let process: Vec<usize> = (0..4).filter(|&i| c[i] > 0.0).collect();
        if process.is_empty() {
            return Some(vec![]);
        }
        if self.o.level == Level::Three {
            let GradientSpace::Inks(space, _) = self.device_n_space(&process, &[])? else { return None };
            return Some(vec![InkPass { space, values: process.iter().map(|&i| c[i]).collect() }]);
        }
        Some(process.iter().map(|&i| InkPass { space: self.process_separation(i), values: vec![c[i]] }).collect())
    }

    /// The name of a Separation color space of process ink `i` (an index into CMYK) alone,
    /// defined in the setup the first time.
    fn process_separation(&mut self, i: usize) -> String {
        const SEPARATIONS: [&str; 4] = [
            "/Cyan /DeviceCMYK {0 0 0}",
            "/Magenta /DeviceCMYK {0 exch 0 0}",
            "/Yellow /DeviceCMYK {0 0 3 -1 roll 0}",
            "/Black /DeviceCMYK {0 0 0 4 -1 roll}",
        ];
        let def = format!("[/Separation {}]", SEPARATIONS[i.min(3)]);
        let i = match self.device_n.iter().position(|d| *d == def) {
            Some(i) => i,
            None => {
                let i = self.device_n.len();
                let _ = writeln!(self.setup, "/VCdn{i} {def} def");
                self.device_n.push(def);
                i
            }
        };
        format!("VCdn{i}")
    }

    /// One drawing of a colorized image on the unit square mapped by `place`: in `paint`, or with
    /// `pass` in that pass's inks alone ([`Self::overprint_passes`]).
    fn colorized_pass(&mut self, key: &str, blob: Option<&ImageBlob>, place: Affine, paint: &Paint, pass: Option<&InkPass>) {
        let mut look = format!("{paint:?}");
        if let Some(pass) = pass {
            let _ = write!(look, "\u{0}{} {:?}", pass.space, pass.values);
        }
        let cache = (key.to_string(), look);
        let encoded = match self.colorized_images.get(&cache) {
            Some(e) => *e,
            None => {
                // An image turned away at the page limit is encoded again only once it fits.
                if let Some(&len) = self.too_large_colorized.get(&cache) {
                    if self.size().saturating_add(len).saturating_add(64) > self.max_body {
                        self.warn(TOO_LARGE);
                        return;
                    }
                    self.too_large_colorized.remove(&cache);
                }
                #[cfg(test)]
                {
                    self.encodes += 1;
                }
                let code = self.encode_colorized(blob, paint, pass);
                // An image that can't be placed isn't kept.
                if let Some(c) = &code
                    && self.size().saturating_add(c.inline_len).saturating_add(64) > self.max_body
                {
                    self.too_large_colorized.insert(cache, c.inline_len);
                    self.warn(TOO_LARGE);
                    return;
                }
                let e = code.map(|c| self.add_code(c));
                self.colorized_images.insert(cache, e);
                e
            }
        };
        if let Some(i) = encoded {
            self.place_image(i, place);
        }
    }

    /// The operators and data of a colorized image of `paint`, or with `pass` of that pass's inks.
    fn encode_colorized(&mut self, blob: Option<&ImageBlob>, paint: &Paint, pass: Option<&InkPass>) -> Option<ImageCode> {
        let Some((w, h, coverage)) = blob.and_then(ImageBlob::coverage) else {
            self.warn(UNDECODED);
            return None;
        };
        if w == 0 || h == 0 || u64::from(w) * u64::from(h) > MAX_IMAGE_PIXELS {
            self.warn(LARGE_IMAGE);
            return None;
        }
        let Paint::Solid { color, swatch, tint } = paint else { return None };
        let ink = self.solid(color, swatch.as_deref(), *tint);
        let binary = coverage.iter().all(|&v| v == 0 || v == 255);
        let clear = coverage.contains(&0);
        if coverage.iter().any(|&v| v > 0 && v < 255) {
            self.warn(IMAGE_PARTLY);
        }
        let keyed = !binary && clear && self.o.level == Level::Three;
        if !binary && clear && !keyed {
            self.warn(IMAGE_ALPHA_L2);
        }
        // Zero tint is uncommon for covered spot pixels (and those of a pass's inks); process
        // uses the same rare key as ordinary images. Nudge a covered collision so only clear
        // pixels match the mask.
        let key = match ink {
            _ if pass.is_some() => [0, 0, 0, 0],
            Ink::Spot { .. } => [0, 0, 0, 0],
            Ink::Process(Space::Gray, ..) => [0, 0, 0, 0],
            Ink::Process(..) => [255, 0, 255, 0],
        };
        let n = match (pass, ink) {
            (Some(pass), _) => pass.values.len(),
            (None, Ink::Spot { .. }) => 1,
            (None, Ink::Process(space, ..)) => space.n(),
        };
        let mut data = Vec::new();
        if binary {
            let stride = (w as usize).div_ceil(8);
            data.resize(stride.saturating_mul(h as usize), 0);
            for (i, &v) in coverage.iter().enumerate() {
                if v == 255
                    && let Some(byte) = data.get_mut((i / w as usize) * stride + (i % w as usize) / 8)
                {
                    *byte |= 0x80 >> (i % w as usize % 8);
                }
            }
        } else {
            for &v in &coverage {
                if v == 0 && keyed {
                    data.extend(key.iter().take(n));
                    continue;
                }
                let a = f32::from(v) / 255.0;
                if let Some(pass) = pass {
                    let mut samples: Vec<u8> = pass.values.iter().map(|&v| (255.0 * v * a).round() as u8).collect();
                    if keyed
                        && samples.iter().all(|&s| s == 0)
                        && let Some(first) = samples.first_mut()
                    {
                        *first = 1;
                    }
                    data.extend(samples);
                    continue;
                }
                match ink {
                    Ink::Spot { tint, .. } => {
                        let mut sample = (255.0 * tint * a).round() as u8;
                        if keyed && sample == key[0] {
                            sample = 1;
                        }
                        data.push(sample);
                    }
                    Ink::Process(space, components) => {
                        let paper = if space == Space::Cmyk { 0.0 } else { 1.0 };
                        let n = space.n();
                        let mut samples = [0; 4];
                        for (sample, &component) in samples.iter_mut().zip(components.iter()).take(n) {
                            *sample = (255.0 * (paper + (component - paper) * a)).round() as u8;
                        }
                        if keyed && samples.iter().take(n).eq(key.iter().take(n)) {
                            samples[if n > 1 { 1 } else { 0 }] = 1;
                        }
                        data.extend(samples.iter().take(n));
                    }
                }
            }
        }
        let (filter, encoded) = match self.o.level {
            Level::Three => ("/FlateDecode", ps::deflate(&data)),
            Level::Two => {
                let mut rl = Vec::new();
                ps::packbits(&data, &mut rl);
                rl.push(128);
                ("/RunLengthDecode", rl)
            }
        };
        let mut prelude = String::new();
        if let Some(pass) = pass {
            let _ = write!(prelude, "{} setcolorspace", pass.space);
            if binary {
                // imagemask takes its current ink from the graphics state.
                for v in &pass.values {
                    let _ = write!(prelude, " {}", ps::num(f64::from(*v)));
                }
                prelude.push_str(" setcolor");
            }
            prelude.push('\n');
        } else if binary {
            // imagemask takes its current ink from the graphics state.
            prelude.push_str(&self.ink_code(ink));
        } else {
            match ink {
                Ink::Spot { space, .. } => {
                    let _ = writeln!(prelude, "VCsp{space} setcolorspace");
                }
                Ink::Process(space, ..) => {
                    let _ = writeln!(prelude, "{} setcolorspace", space.name());
                }
            }
        }
        let decode = if binary { "1 0".to_string() } else { "0 1 ".repeat(n).trim_end().to_string() };
        let mut dict = format!(
            "<< /ImageType {} /Width {w} /Height {h} /BitsPerComponent {} /Decode [{decode}] /ImageMatrix [{w} 0 0 {h} 0 0]",
            if keyed { 4 } else { 1 },
            if binary { 1 } else { 8 },
        );
        if keyed {
            let mask: Vec<String> = key.iter().take(n).map(|v| format!("{v} {v}")).collect();
            let _ = write!(dict, " /MaskColor [{}]", mask.join(" "));
        }
        Some(image_code(prelude, dict, filter, if binary { "imagemask" } else { "image" }, encoded))
    }

    /// Ink-setting operators in the cached stencil body, built with the same path as solid fills.
    fn ink_code(&mut self, ink: Ink) -> String {
        let body = std::mem::take(&mut self.body);
        self.set_ink(ink);
        std::mem::replace(&mut self.body, body)
    }

    /// Image `blob` as RGBA pixels; `None` (with a warning) when it can't be written.
    fn decode_image(&mut self, blob: Option<&ImageBlob>) -> Option<image::RgbaImage> {
        let too_large = |w: u32, h: u32| w == 0 || h == 0 || u64::from(w) * u64::from(h) > MAX_IMAGE_PIXELS;
        if blob.is_some_and(|b| vectorcraft_doc::header_size(&b.bytes).is_some_and(|(w, h)| too_large(w, h))) {
            self.warn(LARGE_IMAGE);
            return None;
        }
        let Some(img) = blob.and_then(|blob| {
            vectorcraft_doc::decode::decode_within(&blob.bytes, MAX_IMAGE_PIXELS.saturating_mul(8)).ok().flatten().map(|(img, _)| img)
        }) else {
            self.warn(UNDECODED);
            return None;
        };
        if too_large(img.width(), img.height()) {
            self.warn(LARGE_IMAGE);
            return None;
        }
        Some(img.to_rgba8())
    }

    /// Draw the image of `codes[i]` on the unit square mapped by `place`. Its operators go in when
    /// the page is finished ([`Self::page`]), once it is known how many placements draw it.
    fn place_image(&mut self, i: usize, place: Affine) {
        if !invertible(place) {
            return;
        }
        let Some(code) = self.codes.get(i) else { return };
        let cost = if code.uses == 0 { code.inline_len } else { code.ref_len };
        if self.size().saturating_add(cost).saturating_add(64) > self.max_body {
            self.warn(TOO_LARGE);
            return;
        }
        self.line("q");
        let _ = writeln!(self.body, "{} cm", ps::matrix(place));
        self.placements.push((self.body.len(), i));
        self.pending = self.pending.saturating_add(cost);
        if let Some(code) = self.codes.get_mut(i) {
            code.uses += 1;
        }
        self.line("Q");
    }

    /// Straight RGBA pixels (`w` × `h`, rows from the top) as the operators and data that draw
    /// them on the unit square: in the process colour space, transparent pixels masked out at
    /// Level 3 (a key colour no opaque pixel has), white at Level 2; partly transparent ones over
    /// white. `None` for an empty image.
    fn encode_image(&mut self, rgba: &[u8], w: u32, h: u32) -> Option<ImageCode> {
        let space = if self.cmyk { Space::Cmyk } else { Space::Rgb };
        let mut cache: HashMap<[u8; 3], [u8; 4]> = HashMap::new();
        let cms = self.cms.clone();
        let intent = cms.settings().intent;
        self.encode_samples(rgba.as_chunks::<4>().0, w, h, space, |p: &[u8; 4], al| {
            let rgb = [0, 1, 2].map(|i| ((u32::from(p[i]) * al + 255 * (255 - al) + 127) / 255) as u8);
            match space {
                Space::Cmyk => *cache
                    .entry(rgb)
                    .or_insert_with(|| cms.srgb_to_cmyk(rgb.map(|x| f32::from(x) / 255.0), intent).map(|x| (unit(x) * 255.0).round() as u8)),
                _ => [rgb[0], rgb[1], rgb[2], 0],
            }
        })
    }

    /// Straight CMYK pixels with opacity (five bytes a pixel, `w` × `h`, rows from the top), drawn
    /// as [`Self::encode_image`] draws its pixels and keeping their ink amounts: partly
    /// transparent pixels lay down that share of each ink.
    fn encode_inks(&mut self, cmyka: &[u8], w: u32, h: u32) -> Option<ImageCode> {
        self.encode_samples(cmyka.as_chunks::<5>().0, w, h, Space::Cmyk, |p: &[u8; 5], al| {
            [0, 1, 2, 3].map(|i| ((u32::from(p[i]) * al + 127) / 255) as u8)
        })
    }

    /// Pixels (`w` × `h`, rows from the top, each ending in its opacity) as the operators and data
    /// that draw them in `space` on the unit square. `sample` gives the samples of a pixel that
    /// isn't clear, from the pixel and its opacity.
    fn encode_samples<const N: usize>(
        &mut self,
        px: &[[u8; N]],
        w: u32,
        h: u32,
        space: Space,
        mut sample: impl FnMut(&[u8; N], u32) -> [u8; 4],
    ) -> Option<ImageCode> {
        if w == 0 || h == 0 {
            return None;
        }
        let opacity = |p: &[u8; N]| p[N - 1];
        let clear = px.iter().any(|p| opacity(p) == 0);
        if px.iter().any(|p| opacity(p) > 0 && opacity(p) < 255) {
            self.warn(IMAGE_PARTLY);
        }
        let keyed = clear && self.o.level == Level::Three;
        if clear && !keyed {
            self.warn(IMAGE_ALPHA_L2);
        }
        let n = space.n();
        // The key colour, which opaque pixels that happen to have it are nudged off.
        let key: [u8; 4] = [255, 0, 255, 0];
        // White: no ink, or full light.
        let white: [u8; 4] = if space == Space::Cmyk { [0; 4] } else { [255; 4] };
        let mut data = Vec::with_capacity(px.len() * n);
        for p in px {
            if opacity(p) == 0 {
                data.extend(if keyed { &key } else { &white }.iter().take(n));
                continue;
            }
            let mut v = sample(p, u32::from(opacity(p)));
            if keyed && v.iter().take(n).eq(key.iter().take(n)) {
                v[1] = 1;
            }
            data.extend(v.iter().take(n));
        }
        let (filter, encoded) = match self.o.level {
            Level::Three => ("/FlateDecode", ps::deflate(&data)),
            Level::Two => {
                let mut rl = Vec::with_capacity(data.len() / 2);
                ps::packbits(&data, &mut rl);
                // End of data.
                rl.push(128);
                ("/RunLengthDecode", rl)
            }
        };
        let prelude = format!("{} setcolorspace\n", space.name());
        let decode = "0 1 ".repeat(n);
        let mut dict = format!(
            "<< /ImageType {} /Width {w} /Height {h} /BitsPerComponent 8 /Decode [{}] /ImageMatrix [{w} 0 0 {h} 0 0]",
            if keyed { 4 } else { 1 },
            decode.trim_end()
        );
        if keyed {
            let mask: Vec<String> = key.iter().take(n).map(|v| format!("{v} {v}")).collect();
            let _ = write!(dict, " /MaskColor [{}]", mask.join(" "));
        }
        Some(image_code(prelude, dict, filter, "image", encoded))
    }
}

/// An image of `prelude`, `dict`, `filter` and `op` (see [`ImageCode`]) with compressed samples
/// `data`, not yet placed.
fn image_code(prelude: String, dict: String, filter: &'static str, op: &'static str, data: Vec<u8>) -> ImageCode {
    let mut code = ImageCode { prelude, dict, filter, op, ascii: ps::ascii85(&data), data, uses: 0, inline_len: 0, ref_len: 0 };
    let mut inline = String::new();
    ps::push_inline_image(&mut inline, "VCsrc", &code.prelude, &code.dict, code.filter, code.op, "");
    code.inline_len = inline.len() + code.ascii.len();
    code
}

/// The closed polygon through `pts`.
fn polygon(pts: &[Point]) -> Vec<kurbo::PathEl> {
    let mut els: Vec<kurbo::PathEl> =
        pts.iter().enumerate().map(|(i, p)| if i == 0 { kurbo::PathEl::MoveTo(*p) } else { kurbo::PathEl::LineTo(*p) }).collect();
    els.push(kurbo::PathEl::ClosePath);
    els
}

/// The square with corners (±`t`, 0) and (0, ±`t`).
fn diamond(t: f64) -> Vec<kurbo::PathEl> {
    polygon(&[Point::new(t, 0.0), Point::new(0.0, t), Point::new(-t, 0.0), Point::new(0.0, -t)])
}

/// `a` to `b` at `t`.
fn mix(a: &[f32], b: &[f32], t: f32) -> Vec<f32> {
    a.iter().zip(b).map(|(x, y)| x + (y - x) * t).collect()
}
