//! Edit → Edit Colors → Recolor Artwork: list the colors of the selection (or of the art Select All
//! takes when nothing is selected), reduce them to rows of
//! similar colours, and remap each row to a new colour by a recolour method. Colours are keyed by
//! their identity ([`ColorKey`]: model and exact values), so a CMYK colour stays CMYK and two
//! colours that only look alike stay apart.

use std::collections::{BTreeSet, HashMap};

use serde_json::{Value, json};
use vectorcraft_color::recolor::{ColorKey, GlobalAdjust, Method, Preserve, Rng, RowMap, cluster, same_ink};
use vectorcraft_color::{Color, Paint, Swatch};
use vectorcraft_doc::swatches::{node_colors, node_paints, override_paints};
use vectorcraft_doc::{Document, Node, NodeId, NodeKind};
use vectorcraft_geom::{Point, Rect};

use super::colorcmds::{NotChanged, Recolor, Scope, left_out};
use super::{CommandSpec, Result, Session, always, bad, checked_ids_param, cmd, color_value, has_doc, index, str_param, switch_or, text_param};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "recolor.colors",
            "Artwork Colors",
            [],
            None,
            "{ids?: object ids (default: the selection, or with nothing selected the objects select.all takes, without the hidden and locked objects inside them)} → {colors: [{key: colour identity (\"rgb 255 0 0\", \"cmyk 0 100 100 0\", \"gray 40\", \"lab 55 60 -40\"), hex, count, swatch?: the global swatch it is linked to}]} unique colours used by the objects (fills, strokes, gradient stops, text, mesh points and the tiles of pattern fills and strokes), most used first",
            has_doc,
            colors
        ),
        cmd!(
            "recolor.apply",
            "Recolor Artwork",
            ["Edit", "Edit Colors"],
            None,
            "{map: rows [{from: [colour keys (recolor.colors) or \"#rrggbb\" (every colour shown as that hex)], to: colour (hex, key, {c,m,y,k} or {l,a,b}) | null (in from, to and groupColors, #rrggbbaa and #rgba are refused), exclude?: false (the row's colours are kept), tint?: 100 (percent; with limitTo, art linked to the global or spot swatch the new color is takes this share of its tint)}] | {\"key or #rrggbb\": colour, …} (one row per colour), method?: \"exact\" (default)|\"preserveTints\"|\"scaleTints\"|\"tintsShades\"|\"hueShift\" (how a row's colours take its new colour, relative to the row's darkest, average or most saturated colour), limitTo?: swatch library id or name; or \"document\" (the document's swatches): new colours snap to its nearest colour, group?: colour group to rewrite, groupColors?: [colour] (what `group` is rewritten with; default: the rows' new colours, in order, excluded rows left out), rename?: the group's new name, includeImages?: true, includePatterns?: true, globalAdjust?: {saturation?: 0, brightness?: 0, temperature?: 0, luminosity?: 0} (Global Adjust: percent from -100 to 100, 0 changes nothing; every colour a row that isn't excluded recolours, and the group's new colours, take the same change after the row's new colour: saturation scales HSB saturation by 1 + saturation/100, brightness moves HSB brightness that share of the way to full or to black, temperature adds 0.2 × temperature/100 to red and takes it from blue, luminosity moves Lab lightness that share of the way to 100 or 0 keeping a and b; with limitTo each row's new colour takes the change before it snaps to the library)} recolour the selection (gradients, text, meshes, image pixels and pattern tiles included; each new colour keeps the model of the one it replaces (model?: \"keep\" default | \"new\" (the new colour's own model, so a CMYK colour replacing RGB art gives CMYK art) | \"document\" (the document's colour mode)), except a library swatch's with limitTo: art takes the swatch itself, linked to a global or spot colour at its tints, and the swatch joins the document's swatches; art linked to a global swatch of `group` that is rewritten with its row's new color stays linked to it, at its tint, unless the row converts the color into the art's model; other recolored art, and art in a row linked to a global swatch of `group` that is rewritten with another color, keeps its row's new color and loses its swatch link) and the group, as one undo step → {changed, swatches: [names added to the document's swatches], relinked?: with group, the other paints linked to its global swatches that took their new colors, note?: set when a colour kept a model other than its new colour's (see model), notChanged: {locked, hidden} (as edit.colors.invert), warning?: set when notChanged counts any object}",
            has_doc,
            apply
        ),
        cmd!(
            query "recolor.reduce",
            "Reduce Colors",
            [],
            None,
            "{colors?: n (rows; default: one per colour, with the tints of a global swatch in its row), method?: as recolor.apply; default scaleTints (picks each row's colour), preserve?: {white?: true, black?: true, grays?: false} (left out of the rows), limitTo?: as recolor.apply (new colours snap to it), ids?: as recolor.colors} group the colors of the objects (default: as recolor.colors) into rows of similar colours (k-means in Lab, weighted by use) → {map: [{from: [keys, the row's colour first], to: key}], preserved: [keys], method}",
            has_doc,
            reduce
        ),
        cmd!(
            query "recolor.randomize",
            "Randomize Colors",
            [],
            None,
            "{map: rows as recolor.apply, order?: true (shuffle the new colours among the rows that aren't excluded), saturationBrightness?: false (random saturation and brightness, hue kept), seed?: 0} → {map: the rows with their new colours as keys}",
            always,
            randomize
        ),
        cmd!(
            query "recolor.pickTheme",
            "Color Theme Picker",
            [],
            None,
            "{picks: [{at: [x, y]} (a click: the object a Selection tool click there takes, with everything inside it) | {rect: [x0, y0, x1, y1]} (a marquee: every painted object it touches, and the part of each image inside it)] (1 to 256, in document points), colors?: 5 (1 to 64)} → {colors: [colour keys, the most used first], objects: the number of objects read} the colour palette of the art picked, for Recolor Artwork's Color Theme Picker: visible fills and strokes, gradient stops, text, mesh points, pattern tiles (with the patterns they use), the art of symbol instances (placed through their 9-slice guides), the colorize colour of an image it applies to and other images' pixels (at most 65,536 pixels of each pick of an image, spread evenly, and 1,048,576 pixels of at most 64 images in all; pixels less than half opaque and images that can't be decoded are left out). In a clip group, or under an opacity mask that clips, only what shows inside the clipping path or the mask art is read; a blend, envelope or repeat a marquee touches is read whole. The symbol instances read place at most 16,000,000 objects in all (an instance whose art would pass that is left out), and the pixels read inside clip groups are tested against at most 4,000,000 clipping-path elements in all (the picks of an image inside clip groups that would pass that are left out). Hidden or fully transparent objects and fill or stroke items are left out; locked objects are read. Each object read weighs the same, shared among its colours (an image's among its pixels; the vector art a symbol instance places counts as one object, each image it places on its own); similar colours merge (k-means in Lab) into at most `colors`, each shown by its most used colour in its own model. Picks that touch no art give no colours. Changes nothing",
            has_doc,
            pick_theme
        ),
    ]
}

/// The pixels one read of an image by the Color Theme Picker takes, at most, the pixels all its
/// reads share, and the images one pick reads, at most.
const THEME_PIXELS: usize = 65_536;
const THEME_TOTAL_PIXELS: usize = 1 << 20;
const MAX_THEME_IMAGES: usize = 64;
/// The image reads one pick queues, at most.
const MAX_THEME_READS: usize = 4096;
/// The stroke segments the opacity masks one pick reads are outlined along, at most.
const MASK_OUTLINE_SEGMENTS: usize = 20_000;
/// The point-in-region tests the picks make to find whether outlines show through clip groups,
/// and the clipping-path elements the pixels of their images are tested against, at most.
const THEME_WORK: usize = 4_000_000;
/// The picks one `recolor.pickTheme` takes, and the colours it gives, at most.
const MAX_PICKS: usize = 256;
const MAX_THEME_COLORS: u64 = 64;
/// The objects the Color Theme Picker reads through, one inside another, at most.
const MAX_THEME_DEPTH: usize = 256;

/// A pick of the Color Theme Picker: a click at a point or a marquee.
enum Pick {
    At(Point),
    Area(Rect),
}

impl Pick {
    /// `{at: [x, y]}` or `{rect: [x0, y0, x1, y1]}`, finite numbers.
    fn param(v: &Value, cmd: &str) -> Result<Self> {
        let numbers = |a: &Value, n: usize| -> Option<Vec<f64>> {
            let a = a.as_array().filter(|a| a.len() == n)?;
            a.iter().map(|x| x.as_f64().filter(|x| x.is_finite())).collect()
        };
        match (v.get("at"), v.get("rect")) {
            (Some(at), None) => {
                numbers(at, 2).map(|p| Pick::At(Point::new(p[0], p[1]))).ok_or_else(|| bad(cmd, format!("`at` must be [x, y] (got {at})")))
            }
            (None, Some(r)) => numbers(r, 4)
                .map(|r| Pick::Area(Rect::new(r[0], r[1], r[2], r[3]).abs()))
                .ok_or_else(|| bad(cmd, format!("`rect` must be [x0, y0, x1, y1] (got {r})"))),
            _ => Err(bad(cmd, format!("each pick is {{at: [x, y]}} or {{rect: [x0, y0, x1, y1]}} (got {v})"))),
        }
    }
}

/// A filled region: its outline and fill rule.
type Region = (vectorcraft_geom::BezPath, vectorcraft_geom::FillRule);

/// An image the Color Theme Picker reads: its pixels, where it lies, the marquee it is read in
/// (`None`: all of it) and the region the clip groups around it show (`None`: unclipped). An
/// image its colorize paint applies to ([`vectorcraft_doc::ImageBlob::takes_colorize`]) gives that
/// paint's colours where a pixel read shows ink (opaque and darker than white).
#[derive(Clone)]
struct ImageRead<'a> {
    blob: &'a vectorcraft_doc::ImageBlob,
    xf: vectorcraft_geom::Affine,
    size: (f64, f64),
    area: Option<Rect>,
    clip: Option<Vec<Region>>,
    colorize: Option<Vec<Color>>,
}

/// A pattern's colours, the image reads of its tiles, and the depth its tiles were read at when
/// [`MAX_THEME_DEPTH`] cut them short.
type PatternRead<'a> = (Vec<Color>, Vec<ImageRead<'a>>, Option<usize>);

/// The colours the Color Theme Picker has read: each object weighs 1, shared among its colours.
struct Theme<'a> {
    d: &'a Document,
    boxes: vectorcraft_doc::SymbolBoxes<'a>,
    samples: Vec<(Color, f32)>,
    /// The images to read, each image decoded once for all its reads ([`Self::read_images`]),
    /// and the images they read.
    images: Vec<ImageRead<'a>>,
    blobs: std::collections::HashSet<*const vectorcraft_doc::ImageBlob>,
    /// What the clip groups being read show, in document space: the union of these regions
    /// (none: nothing shows); `None` outside clip groups.
    clip: Option<Vec<Region>>,
    /// The colours of the patterns read so far (with the depth their tiles were read at when
    /// [`MAX_THEME_DEPTH`] cut them short), and the patterns whose tiles are being read.
    pattern_colors: HashMap<String, PatternRead<'a>>,
    pattern_stack: Vec<String>,
    /// Whether art was left out for passing [`MAX_THEME_DEPTH`].
    cut: bool,
    /// The region tests [`Self::shows`] has made.
    work: std::cell::Cell<usize>,
    /// The objects the symbol instances read have placed, of [`vectorcraft_doc::MAX_SYMBOL_NODES`]
    /// ([`vectorcraft_doc::charge_symbol_art`]).
    symbol_nodes: u64,
    /// The object isolated in isolation mode.
    scope: Option<NodeId>,
    /// The stroke segments opacity masks may still be outlined along ([`mask_regions`]).
    mask_budget: usize,
    objects: usize,
}

/// The region both `a` and `b` (each a union of regions) cover, filled non-zero.
fn intersect_regions(a: &[Region], b: &[Region]) -> Vec<Region> {
    let paths: Vec<vectorcraft_geom::PathData> = a.iter().chain(b).map(|(bp, _)| vectorcraft_geom::PathData::from_bezpath(bp)).collect();
    let inputs: Vec<(&vectorcraft_geom::PathData, vectorcraft_geom::FillRule)> =
        paths.iter().zip(a.iter().chain(b)).map(|(p, (_, rule))| (p, *rule)).collect();
    let split = a.len();
    let both = vectorcraft_pathops::boolean_n(&inputs, |inside| inside.iter().any(|i| *i < split) && inside.iter().any(|i| *i >= split));
    if both.subpaths.is_empty() { vec![] } else { vec![(both.to_bezpath(), vectorcraft_geom::FillRule::NonZero)] }
}

impl<'a> Theme<'a> {
    fn new(d: &'a Document) -> Self {
        Self {
            d,
            boxes: d.symbol_boxes(),
            samples: vec![],
            images: vec![],
            blobs: std::collections::HashSet::new(),
            clip: None,
            pattern_colors: HashMap::new(),
            cut: false,
            pattern_stack: vec![],
            work: std::cell::Cell::new(0),
            symbol_nodes: 0,
            scope: None,
            mask_budget: MASK_OUTLINE_SEGMENTS,
            objects: 0,
        }
    }

    fn add(&mut self, colors: Vec<Color>) {
        if colors.is_empty() {
            return;
        }
        let w = 1.0 / colors.len() as f32;
        self.samples.extend(colors.into_iter().map(|c| (c, w)));
        self.objects += 1;
    }

    /// Read the images queued, each image's pixels decoded once, as colours: the pixels a read
    /// gives that fall in one cell of 16 levels a channel merge into their average, weighted by how
    /// many there are, and the cells of all the reads add up, so the images give at most 16³
    /// colours. Images that can't be decoded give none. The first [`MAX_THEME_IMAGES`] images
    /// are read, and all the reads share [`THEME_TOTAL_PIXELS`] pixels (each read at least one, at
    /// most [`THEME_PIXELS`]). The pixels read inside clip groups are tested against
    /// [`THEME_WORK`] clipping-path elements in all; the reads of an image inside clip groups
    /// that would pass it are left out. Gives the clipping-path elements tested.
    fn read_images(&mut self) -> usize {
        let reads = std::mem::take(&mut self.images);
        // The reads of each image, in the order its first read came.
        let mut by_image: Vec<(*const vectorcraft_doc::ImageBlob, Vec<&ImageRead>)> = vec![];
        for r in &reads {
            let key: *const vectorcraft_doc::ImageBlob = r.blob;
            match by_image.iter().position(|(k, _)| *k == key) {
                Some(i) => by_image[i].1.push(r),
                None if by_image.len() < MAX_THEME_IMAGES => by_image.push((key, vec![r])),
                None => {}
            }
        }
        let count: usize = by_image.iter().map(|(_, g)| g.len()).sum();
        let budget = (THEME_TOTAL_PIXELS / count.max(1)).clamp(1, THEME_PIXELS);
        let mut pixels: HashMap<[u8; 3], ([f64; 3], f64)> = HashMap::new();
        // The clipping-path elements the pixels read have been tested against.
        let tested = std::cell::Cell::new(0usize);
        let elements =
            |r: &ImageRead| r.clip.as_ref().map_or(0, |rs| rs.iter().map(|(bp, _)| bp.elements().len().max(1)).fold(0, usize::saturating_add));
        for (_, mut group) in by_image {
            // Each pixel sampled for a read inside clip groups is tested against its clipping
            // regions: the clipped reads of one image take at most `clipped` pixels each, so all
            // of them keep within the work left, and the other reads take `budget`. When the work
            // left can't test one pixel of each, the clipped reads are kept in order while their
            // pixel fits, and the rest are left out.
            let left = THEME_WORK.saturating_sub(tested.get());
            if group.iter().map(|r| elements(r)).fold(0, usize::saturating_add) > left {
                let mut room = left;
                group.retain(|r| {
                    let c = elements(r);
                    let fits = c <= room;
                    room = room.saturating_sub(if fits { c } else { 0 });
                    fits
                });
            }
            let cost = group.iter().map(|r| elements(r)).fold(0, usize::saturating_add);
            let clipped = left.checked_div(cost).map_or(budget, |w| budget.min(w));
            if group.is_empty() {
                continue;
            }
            let keeps: Vec<Box<dyn Fn(f64, f64) -> bool + '_>> = group
                .iter()
                .map(|r| {
                    let (xf, (w, h), area, clip, cost, tested) = (r.xf, r.size, r.area, &r.clip, elements(r), &tested);
                    Box::new(move |u: f64, v: f64| {
                        let p = xf * Point::new(u * w, v * h);
                        area.is_none_or(|a| a.contains(p))
                            && clip.as_ref().is_none_or(|rs| {
                                tested.set(tested.get().saturating_add(cost));
                                rs.iter().any(|(bp, rule)| vectorcraft_geom::hit::fill_contains(bp, *rule, p))
                            })
                    }) as Box<dyn Fn(f64, f64) -> bool + '_>
                })
                .collect();
            let picks: Vec<vectorcraft_doc::PixelPick> = group
                .iter()
                .zip(&keeps)
                .map(|(r, keep)| {
                    use vectorcraft_geom::kurbo::Shape;
                    let (w, h) = r.size;
                    let inv = r.xf.inverse();
                    // The box sampled: the marquee, cut to the clipping paths' bounds, so a small
                    // region a clip shows of a large image is still sampled densely.
                    let clip_box = r.clip.as_ref().map(|rs| rs.iter().map(|(bp, _)| bp.bounding_box()).reduce(|a, b| a.union(b)).unwrap_or_default());
                    let region = match (r.area, clip_box) {
                        (Some(a), Some(c)) => Some(a.intersect(c)),
                        (a, c) => a.or(c),
                    };
                    let box_uv = region.map(|a| {
                        let uv: Vec<Point> =
                            [(a.x0, a.y0), (a.x1, a.y0), (a.x0, a.y1), (a.x1, a.y1)].into_iter().map(|(x, y)| inv * Point::new(x, y)).collect();
                        let low = |f: fn(&Point) -> f64| uv.iter().map(f).fold(f64::INFINITY, f64::min);
                        let high = |f: fn(&Point) -> f64| uv.iter().map(f).fold(f64::NEG_INFINITY, f64::max);
                        [low(|p| p.x) / w, low(|p| p.y) / h, high(|p| p.x) / w, high(|p| p.y) / h]
                    });
                    (box_uv, if r.clip.is_some() { clipped } else { budget }, keep.as_ref())
                })
                .collect();
            let Some(each) = group[0].blob.sample_pixels(&picks) else { continue };
            for (px, read) in each.into_iter().zip(&group) {
                if let Some(colors) = &read.colorize {
                    // Ink: an opaque pixel darker than white takes the colorize paint.
                    if !colors.is_empty() && px.iter().any(|p| p[3] >= 128 && p[..3].iter().any(|v| *v < 255)) {
                        let w = 1.0 / colors.len().max(1) as f32;
                        self.samples.extend(colors.iter().map(|c| (*c, w)));
                        self.objects += 1;
                    }
                    continue;
                }
                let mut cells: HashMap<[u8; 3], ([u64; 3], u64)> = HashMap::new();
                for [r, g, b, _] in px.into_iter().filter(|p| p[3] >= 128) {
                    let cell = cells.entry([r >> 4, g >> 4, b >> 4]).or_default();
                    for (sum, v) in cell.0.iter_mut().zip([r, g, b]) {
                        *sum += u64::from(v);
                    }
                    cell.1 += 1;
                }
                let total: u64 = cells.values().map(|c| c.1).sum();
                if total == 0 {
                    continue;
                }
                for (key, (sum, n)) in cells {
                    let cell = pixels.entry(key).or_default();
                    for (acc, s) in cell.0.iter_mut().zip(sum) {
                        *acc += s as f64 / total as f64;
                    }
                    cell.1 += n as f64 / total as f64;
                }
                self.objects += 1;
            }
        }
        let mut cells: Vec<_> = pixels.into_iter().collect();
        // The same order on every run.
        cells.sort_unstable_by_key(|c| c.0);
        for (_, (sum, w)) in cells {
            let [r, g, b] = sum.map(|s| (s / w).round().clamp(0.0, 255.0) as u8);
            self.samples.push((Color::rgb8(r, g, b), w as f32));
        }
        tested.get()
    }

    /// Read `n` and everything visible inside it: each image (its part inside `area` when given,
    /// and inside the clipping paths around it) and each other object with colours of its own
    /// (that touches `area` when given, and shows inside the clipping paths around it). The art a
    /// symbol instance shows (with a dynamic instance's own paints), placed where the instance
    /// draws it, and the objects a blend, an envelope or a repeat draws elsewhere than they lie,
    /// are read whole when the instance or the live object touches `area`. `depth` counts the
    /// objects passed through: deeper art is left out. An enabled opacity mask that clips (and is
    /// not inverted) shows the object only where its mask art is lighter than black
    /// ([`mask_regions`]), as a clip group would.
    fn read(&mut self, n: &Node, area: Option<Rect>, depth: usize) {
        let Some(mask) = n.extras().mask.as_deref().filter(|m| !m.disabled && m.clip && !m.invert) else {
            return self.read_unmasked(n, area, depth);
        };
        let regions = mask_regions(&mask.art, &mut self.mask_budget);
        let outer = self.clip.clone();
        self.clip = Some(match &outer {
            None => regions,
            Some(o) => intersect_regions(o, &regions),
        });
        self.read_unmasked(n, area, depth);
        self.clip = outer;
    }

    /// [`Self::read`] without `n`'s opacity mask.
    fn read_unmasked(&mut self, n: &Node, area: Option<Rect>, depth: usize) {
        if depth > MAX_THEME_DEPTH {
            self.cut = true;
            return;
        }
        // Hidden and fully transparent objects show nothing.
        if !n.visible || n.opacity <= 0.0 || matches!(n.kind, NodeKind::Path { guide: true, .. }) {
            return;
        }
        let touches = area.is_none_or(|r| self.touches(n, r));
        match &n.kind {
            NodeKind::Image(im) => {
                if !touches {
                    return;
                }
                // A document keeps its images' pixels in its image store.
                let Some(blob) = self.d.images.get(&im.key) else { return };
                // An image with a colorize paint shows that paint's color, not its own pixels;
                // other paints its appearance has (with effects that draw them) count as well.
                let colorized = n.extras().colorize.is_some() && blob.takes_colorize();
                let own = if colorized || n.extras().colorize.is_none() {
                    self.own(n, true, depth)
                } else {
                    // A colorize paint the image does not take (a linked image's preview)
                    // shows nothing.
                    let mut plain = n.clone();
                    plain.extras_mut().colorize = None;
                    self.own(&plain, true, depth)
                };
                let colorize = if colorized {
                    Some(own)
                } else {
                    if self.shows(n, area) {
                        self.add(own);
                    }
                    None
                };
                let size = (f64::from(im.width.max(1)), f64::from(im.height.max(1)));
                self.queue_image(ImageRead { blob, xf: im.xf, size, area, clip: self.clip.clone(), colorize });
                return;
            }
            NodeKind::SymbolInstance { symbol, xf } => {
                if touches
                    && self.shows(n, area)
                    && let Some(s) = self.d.symbols.iter().find(|s| s.name == *symbol)
                {
                    // The art as the instance places it: through the symbol's 9-slice guides when
                    // it has them, else by the instance's transform. All the instances read share
                    // one limit on the objects they place, counted as the renderer counts them: an
                    // instance whose art would pass it shows nothing and is left out.
                    let dynamic = (!n.extras().symbol_overrides.is_empty()).then(|| vectorcraft_doc::symbol_overrides::dynamic_art(&s.art, n));
                    let (d, art) = (self.d, dynamic.as_ref().unwrap_or(&s.art));
                    let placed = match d.symbol_slice_map(symbol, *xf) {
                        Some(m) => vectorcraft_doc::charge_symbol_art(&mut self.symbol_nodes, m.most_placed(art, d), || {
                            let placed = d.place_symbol_art(symbol, art, *xf);
                            let n = placed.as_ref().map_or(0, |a| a.count_with_masks() as u64);
                            (placed, n)
                        }),
                        None => {
                            let count = art.count() as u64;
                            vectorcraft_doc::charge_symbol_art(&mut self.symbol_nodes, count, || (d.place_symbol_art(symbol, art, *xf), count))
                        }
                    };
                    let Some(Some(placed)) = placed else { return };
                    // The instance's own paints; its overrides show in its art.
                    let own = self.own(n, false, depth);
                    self.add(own);
                    // The vector art counts as one object, each of its colours once, so an
                    // object the 9-slice guides cut into several copies does not count more.
                    // Each image placed is read on its own, as images are elsewhere.
                    let (samples, objects) = (self.samples.len(), self.objects);
                    self.read(&placed, None, depth + 1);
                    let added = self.samples.split_off(samples);
                    self.objects = objects;
                    let mut seen = std::collections::HashSet::new();
                    let colors: Vec<Color> = added.into_iter().map(|(c, _)| c).filter(|c| seen.insert(ColorKey::of(c))).collect();
                    self.add(colors);
                }
                return;
            }
            NodeKind::Blend { .. } | NodeKind::Envelope { .. } | NodeKind::Repeat(_) => {
                if touches && self.shows(n, area) {
                    let own = self.own(n, true, depth);
                    self.add(own);
                    // The copies are drawn elsewhere than the objects lie: the clipping paths
                    // around the live object, tested above, do not apply to the objects.
                    let clip = self.clip.take();
                    for c in n.children().into_iter().flatten() {
                        self.read(c, None, depth + 1);
                    }
                    self.clip = clip;
                }
                return;
            }
            _ => {}
        }
        if touches && self.shows(n, area) {
            let own = self.own(n, true, depth);
            self.add(own);
        }
        // A compound path's members are its geometry: the compound paints them.
        if matches!(n.kind, NodeKind::Compound { .. }) {
            return;
        }
        // A clip group's clipping path paints whole, over the art it clips: it is read in the
        // marquee as given and under the clip groups around it alone.
        let clip_child = n.children().and_then(|c| c.first()).filter(|_| n.clips());
        if let Some(first) = clip_child {
            self.read(first, area, depth + 1);
        }
        let inner = match area {
            Some(r) => match vectorcraft_doc::hit::marquee_in(n, r) {
                Some(inner) => Some(inner),
                None => return,
            },
            None => None,
        };
        // A clip group shows its art where its clipping path's regions and the clip groups around
        // it all show.
        let outer = self.clip.clone();
        if n.clips()
            && let Some(first) = n.children().and_then(|c| c.first())
        {
            let regions: Vec<Region> = vectorcraft_doc::hit::clip_regions(first);
            self.clip = Some(match &outer {
                None => regions,
                Some(o) => intersect_regions(o, &regions),
            });
        }
        // A Shaper Group's original shapes, which the group keeps but does not show, are left out.
        for (i, c) in n.children().into_iter().flatten().enumerate().skip(usize::from(clip_child.is_some())) {
            if !vectorcraft_doc::hit::shaper_hides(n, i, self.scope) {
                self.read(c, inner, depth + 1);
            }
        }
        self.clip = outer;
    }

    /// Does `n` show inside the region the clip groups around it show, in `area` when given? Its
    /// box (cut to `area`) meets a region of it: the region's outline crosses the box, or a point
    /// on a 9 × 9 grid over the part of the box inside the region's bounds lies in the region.
    fn shows(&self, n: &Node, area: Option<Rect>) -> bool {
        use vectorcraft_geom::kurbo::Shape;
        let Some(regions) = &self.clip else { return true };
        let Some(b) = n.visual_bounds_with(&self.boxes) else { return false };
        let b = area.map_or(b, |r| b.intersect(r));
        let valid = |r: &Rect| r.width() >= 0.0 && r.height() >= 0.0;
        if !valid(&b) {
            return false;
        }
        // A path or compound path paints along its outline, and inside it where it has a visible
        // fill: a point along the outline, or (filled) a point of a 9 × 9 grid over the part of the
        // box inside a region's bounds that lies in both its fill and the region, must show.
        let filled = has_fill(n);
        let outline: Option<(Vec<&vectorcraft_geom::PathData>, vectorcraft_geom::FillRule)> = match &n.kind {
            NodeKind::Path { path, rule, .. } => Some((vec![path], *rule)),
            NodeKind::Compound { children, rule } => Some((children.iter().filter(|c| c.visible).filter_map(|c| c.path_data()).collect(), *rule)),
            _ => None,
        };
        if let Some((paths, own_rule)) = outline {
            use vectorcraft_geom::ParamCurve;
            use vectorcraft_geom::kurbo::ParamCurveArclen;
            // The work all these tests do is bounded: past it, objects count as shown.
            if self.work.get() > THEME_WORK {
                return true;
            }
            let half = stroke_reach(n);
            let cost: usize = regions.iter().map(|(bp, _)| bp.elements().len().max(1)).sum();
            let in_region = |p: Point| {
                // Past the work allowed, the outline counts as shown.
                if self.work.get() > THEME_WORK {
                    return true;
                }
                self.work.set(self.work.get() + cost);
                regions.iter().any(|(bp, rule)| {
                    vectorcraft_geom::hit::fill_contains(bp, *rule, p) || half > 0.0 && vectorcraft_geom::hit::stroke_contains(bp, 0.0, half, p)
                })
            };
            let mut fill = vectorcraft_geom::BezPath::new();
            for p in &paths {
                fill.extend(p.to_bezpath().elements().iter().copied());
            }
            // At most 4,096 points along the outline, shared among its segments by length, no
            // farther apart than half the smallest region's width or height where that allows.
            let step = regions
                .iter()
                .map(|(bp, _)| bp.bounding_box())
                .chain(area)
                .map(|rb| rb.width().min(rb.height()))
                .fold(f64::INFINITY, f64::min)
                .max(0.01)
                / 2.0;
            // With a marquee, only the outline inside it (within the stroke's reach) counts.
            let picked = area.map(|r| r.inflate(half, half));
            let segs: Vec<(vectorcraft_geom::PathSeg, f64)> = fill
                .segments()
                .take(4096)
                .map(|s| {
                    let len = s.arclen(0.5);
                    (s, if len.is_finite() { len } else { 0.0 })
                })
                .collect();
            let total: f64 = segs.iter().map(|(_, l)| l).sum();
            let points = (total / step).ceil().clamp(64.0, 4096.0);
            let along = segs.iter().any(|(seg, len)| {
                let n = if total > 0.0 { (points * len / total).round().clamp(1.0, 4096.0) as u32 } else { 1 };
                (0..=n).any(|i| {
                    let p = seg.eval(f64::from(i) / f64::from(n));
                    picked.is_none_or(|r| r.contains(p)) && in_region(p)
                })
            });
            return along
                || filled
                    && regions.iter().any(|(bp, _)| {
                        let g = b.intersect(bp.bounding_box());
                        valid(&g)
                            && (0..81).any(|i| {
                                let p = Point::new(g.x0 + g.width() * f64::from(i % 9) / 8.0, g.y0 + g.height() * f64::from(i / 9) / 8.0);
                                in_region(p) && vectorcraft_geom::hit::fill_contains(&fill, own_rule, p)
                            })
                    });
        }
        regions.iter().any(|(bp, rule)| {
            let g = b.intersect(bp.bounding_box());
            valid(&g)
                && (vectorcraft_geom::hit::intersects_rect(&vectorcraft_geom::PathData::from_bezpath(bp), b)
                    || (0..81).any(|i| {
                        let p = Point::new(g.x0 + g.width() * f64::from(i % 9) / 8.0, g.y0 + g.height() * f64::from(i / 9) / 8.0);
                        vectorcraft_geom::hit::fill_contains(bp, *rule, p)
                    }))
        })
    }

    /// Does the marquee `r` touch what `n` paints? A path or a compound path is touched where its
    /// outline comes within half its widest visible stroke of `r` or, when it has a visible fill,
    /// where `r`'s centre lies in that fill; any other object is touched by its box.
    fn touches(&self, n: &Node, r: Rect) -> bool {
        let boxed = n.visual_bounds_with(&self.boxes).is_some_and(|b| b.x0 <= r.x1 && r.x0 <= b.x1 && b.y0 <= r.y1 && r.y0 <= b.y1);
        let filled = || has_fill(n);
        let half = stroke_reach(n);
        let near = r.inflate(half, half);
        let crosses = |paths: &[&vectorcraft_geom::PathData], rule: vectorcraft_geom::FillRule| {
            paths.iter().any(|p| vectorcraft_geom::hit::intersects_rect(p, near))
                || filled() && {
                    let mut bp = vectorcraft_geom::BezPath::new();
                    for p in paths {
                        bp.extend(p.to_bezpath().elements().iter().copied());
                    }
                    // A point on a 5 × 5 grid over the part of the marquee in the path's box lies
                    // in its fill.
                    use vectorcraft_geom::kurbo::Shape;
                    let g = r.intersect(bp.bounding_box());
                    g.width() >= 0.0
                        && g.height() >= 0.0
                        && (0..25).any(|i| {
                            let p = Point::new(g.x0 + g.width() * f64::from(i % 5) / 4.0, g.y0 + g.height() * f64::from(i / 5) / 4.0);
                            vectorcraft_geom::hit::fill_contains(&bp, rule, p)
                        })
                }
        };
        match &n.kind {
            NodeKind::Path { path, rule, .. } => boxed && crosses(&[path], *rule),
            NodeKind::Compound { children, rule } => {
                let paths: Vec<&vectorcraft_geom::PathData> = children.iter().filter(|c| c.visible).filter_map(|c| c.path_data()).collect();
                boxed && crosses(&paths, *rule)
            }
            _ => boxed,
        }
    }
}

/// The regions where opacity mask art `art` lets the masked object show: the regions
/// ([`vectorcraft_doc::hit::clip_regions`]) of its visible paths and compound paths filled lighter
/// than black (a solid color, a gradient with a stop lighter than black, or a pattern), the
/// outlines of the strokes lighter than black they have (dashes read as solid), the frames of its
/// images, the boxes of its type painted lighter than black, the boxes of art with effects (with
/// what the effects reach), and the regions of its clip groups and live objects whole. Black,
/// unfilled and unstroked paths without effects hide; how light the art is beyond that is not
/// read. Strokes are outlined along at most `budget` segments in all (it counts down); past that
/// a stroke counts by its box grown by half its width.
fn mask_regions(art: &Node, budget: &mut usize) -> Vec<Region> {
    // Any color but black lets the masked art show.
    fn light(p: &Paint) -> bool {
        let lit = |c: &Color| c.to_rgba8(1.0)[..3].iter().any(|v| *v > 0);
        match p {
            Paint::None => false,
            Paint::Solid { color, .. } => lit(color),
            Paint::Gradient(g) => g.gradient.stops.iter().any(|s| lit(&s.color)),
            _ => true,
        }
    }
    fn walk(n: &Node, depth: usize, out: &mut Vec<Region>, budget: &mut usize) {
        use vectorcraft_geom::kurbo::Shape;
        // The mask art's root masks even when it is hidden.
        // Guides draw nothing.
        if depth > MAX_THEME_DEPTH || depth > 0 && (!n.visible || n.opacity <= 0.0) || matches!(n.kind, NodeKind::Path { guide: true, .. }) {
            return;
        }
        // Art with effects can show light anywhere near it (a glow on black art): it counts
        // whole, as a clipping path would.
        let effects = !n.appearance.effects.is_empty()
            || n.appearance.items.iter().any(|it| match it {
                vectorcraft_doc::AppearanceItem::Fill(l) => l.visible && !l.effects.is_empty(),
                vectorcraft_doc::AppearanceItem::Stroke(l) => l.visible && !l.effects.is_empty(),
            });
        if effects {
            if let Some(b) = n.visual_bounds() {
                out.push((b.to_path(0.1), vectorcraft_geom::FillRule::NonZero));
            }
            return;
        }
        let lit = n
            .appearance
            .items
            .iter()
            .any(|it| matches!(it, vectorcraft_doc::AppearanceItem::Fill(l) if l.visible && l.opacity > 0.0 && light(&l.paint)));
        let stroke = n
            .appearance
            .items
            .iter()
            .filter_map(|it| match it {
                vectorcraft_doc::AppearanceItem::Stroke(l) if l.visible && l.opacity > 0.0 && l.width > 0.0 && light(&l.paint) => Some(l.width / 2.0),
                _ => None,
            })
            .fold(None, |a: Option<f64>, w| Some(a.map_or(w, |a| a.max(w))));
        let boxed = |out: &mut Vec<Region>| {
            if let Some(b) = n.geometric_bounds() {
                out.push((b.to_path(0.1), vectorcraft_geom::FillRule::NonZero));
            }
        };
        match &n.kind {
            NodeKind::Path { .. } | NodeKind::Compound { .. } => {
                if lit {
                    out.extend(vectorcraft_doc::hit::clip_regions(n));
                }
                // A stroke shows the art along it: its outline with round ends and corners, a
                // dashed stroke read as solid (the most it can show).
                if let Some(half) = stroke {
                    let paths: Vec<&vectorcraft_geom::PathData> = match &n.kind {
                        NodeKind::Path { path, .. } => vec![path],
                        NodeKind::Compound { children, .. } => children.iter().filter_map(|c| c.path_data()).collect(),
                        _ => vec![],
                    };
                    for p in paths {
                        // Past MASK_OUTLINE_SEGMENTS outlined segments, a stroke counts by its
                        // box grown by half its width.
                        let segments = p.to_bezpath().elements().len();
                        if *budget < segments {
                            if let Some(b) = p.bounds().filter(|_| half.is_finite()) {
                                out.push((b.inflate(half, half).to_path(0.1), vectorcraft_geom::FillRule::NonZero));
                            }
                            continue;
                        }
                        *budget -= segments;
                        let region = vectorcraft_pathops::outline_stroke(
                            p,
                            half * 2.0,
                            vectorcraft_pathops::Cap::Round,
                            vectorcraft_pathops::Join::Round,
                            10.0,
                        );
                        if !region.subpaths.is_empty() {
                            out.push((region.to_bezpath(), vectorcraft_geom::FillRule::NonZero));
                        }
                    }
                }
            }
            NodeKind::Image(_) => out.extend(vectorcraft_doc::hit::clip_regions(n)),
            NodeKind::Text(t) => {
                if lit || stroke.is_some() || t.runs.iter().any(|r| light(&r.style.fill) || r.style.stroke_width > 0.0 && light(&r.style.stroke)) {
                    boxed(out);
                }
            }
            NodeKind::Group { clip: false, .. } | NodeKind::Layer { clip: false, .. } => {
                // A Shaper Group's original shapes are kept but not drawn.
                for (i, c) in n.children().into_iter().flatten().enumerate() {
                    if !vectorcraft_doc::hit::shaper_hides(n, i, None) {
                        walk(c, depth + 1, out, budget);
                    }
                }
            }
            // Clip groups, live objects and placed documents count by the regions they clip to.
            _ => out.extend(vectorcraft_doc::hit::clip_regions(n)),
        }
    }
    let mut out = vec![];
    walk(art, 0, &mut out, budget);
    out
}

/// Does `n` have a fill item that paints: visible, not fully transparent and not None?
fn has_fill(n: &Node) -> bool {
    n.appearance
        .items
        .iter()
        .any(|it| matches!(it, vectorcraft_doc::AppearanceItem::Fill(l) if l.visible && l.opacity > 0.0 && !matches!(l.paint, Paint::None)))
}

/// How far `n`'s widest painting stroke reaches from its path: half its width (0 without one).
fn stroke_reach(n: &Node) -> f64 {
    let paints = |l: &vectorcraft_doc::StrokeLayer| l.visible && l.opacity > 0.0 && !matches!(l.paint, Paint::None) && l.width.is_finite();
    n.appearance
        .items
        .iter()
        .filter_map(|it| match it {
            vectorcraft_doc::AppearanceItem::Stroke(l) if paints(l) => Some(l.width.max(0.0) / 2.0),
            _ => None,
        })
        .fold(0.0, f64::max)
}

/// The paints `m` shows: its visible fill items and its visible strokes wider than 0, its text's
/// fills and strokes wider than 0, and its colorize paint.
fn shown_paints(m: &Node) -> impl Iterator<Item = &Paint> {
    let runs = match &m.kind {
        NodeKind::Text(t) => t.runs.as_slice(),
        _ => &[],
    };
    let items = m.appearance.items.iter().filter_map(|it| match it {
        vectorcraft_doc::AppearanceItem::Fill(l) if l.visible && l.opacity > 0.0 => Some(&l.paint),
        vectorcraft_doc::AppearanceItem::Stroke(l) if l.visible && l.opacity > 0.0 && l.width > 0.0 => Some(&l.paint),
        _ => None,
    });
    let text = runs.iter().flat_map(|r| std::iter::once(&r.style.fill).chain((r.style.stroke_width > 0.0).then_some(&r.style.stroke)));
    items.chain(text).chain(m.extras().colorize.as_deref())
}

/// Add the colours `m` shows itself to `out` (solid colours, gradient stops, its text's colours
/// and its mesh points; the paints a dynamic symbol instance keeps for objects of its art only
/// when `overrides` is set), and the names of the patterns it paints with to `patterns`, leaving
/// out names already there.
fn paint_colors_of(m: &Node, overrides: bool, out: &mut Vec<Color>, patterns: &mut Vec<String>) {
    let kept = override_paints(m).filter(|_| overrides);
    for p in shown_paints(m).chain(kept) {
        match p {
            Paint::Solid { color, .. } => out.push(*color),
            Paint::Gradient(g) => out.extend(g.gradient.stops.iter().map(|s| s.color)),
            Paint::Pattern { pattern, .. } if !patterns.contains(pattern) => patterns.push(pattern.clone()),
            _ => {}
        }
    }
    if let NodeKind::Mesh(mesh) = &m.kind {
        out.extend(mesh.points.iter().map(|p| p.color));
    }
}

impl<'a> Theme<'a> {
    /// Queue image read `r`, within the limits of a pick: at most [`MAX_THEME_READS`] reads of
    /// [`MAX_THEME_IMAGES`] images; nothing inside clip groups that show nothing.
    fn queue_image(&mut self, r: ImageRead<'a>) {
        if self.images.len() >= MAX_THEME_READS || r.clip.as_ref().is_some_and(Vec::is_empty) {
            return;
        }
        let key: *const vectorcraft_doc::ImageBlob = r.blob;
        if !self.blobs.contains(&key) {
            if self.blobs.len() >= MAX_THEME_IMAGES {
                return;
            }
            self.blobs.insert(key);
        }
        self.images.push(r);
    }

    /// The colours `m` paints itself ([`paint_colors_of`]; a dynamic symbol instance's paints for
    /// objects of its art only with `overrides`), and the colours its pattern tiles show, read as
    /// document art is read (clip groups and symbols included; the tiles' images join this pick's
    /// image reads). Each pattern's colours
    /// are read once; a pattern whose tile uses itself, directly or through other patterns, adds
    /// nothing more the second time. `depth` is `m`'s ([`Self::read`]): the tiles' art is read one
    /// deeper, and a pattern whose tiles [`MAX_THEME_DEPTH`] cut short is read again when it is
    /// used less deep.
    fn own(&mut self, m: &Node, overrides: bool, depth: usize) -> Vec<Color> {
        let mut out = vec![];
        let mut patterns: Vec<String> = vec![];
        paint_colors_of(m, overrides, &mut out, &mut patterns);
        for name in patterns {
            if let Some((colors, reads, cut)) = self.pattern_colors.get(&name).filter(|e| e.2.is_none_or(|at| depth >= at)) {
                self.cut |= cut.is_some();
                out.extend(colors);
                // The tiles' images count again for each use, within the pick's limits.
                let reads = reads.clone();
                for r in reads {
                    self.queue_image(r);
                }
                continue;
            }
            if self.pattern_stack.contains(&name) || self.pattern_stack.len() >= MAX_THEME_DEPTH {
                continue;
            }
            let Some(def) = self.d.pattern(&name) else { continue };
            let mut sub = Theme::new(self.d);
            sub.pattern_stack = std::mem::take(&mut self.pattern_stack);
            sub.pattern_stack.push(name.clone());
            sub.pattern_colors = std::mem::take(&mut self.pattern_colors);
            // The tiles' images join this pick's image reads, within its limits.
            sub.images = std::mem::take(&mut self.images);
            sub.blobs = std::mem::take(&mut self.blobs);
            // The tiles share this pick's limits on work too.
            sub.work.set(self.work.get());
            sub.symbol_nodes = self.symbol_nodes;
            sub.mask_budget = self.mask_budget;
            let before = sub.images.len();
            for a in &def.art {
                sub.read(a, None, depth + 1);
            }
            sub.pattern_stack.pop();
            self.pattern_stack = std::mem::take(&mut sub.pattern_stack);
            self.pattern_colors = std::mem::take(&mut sub.pattern_colors);
            let reads: Vec<ImageRead<'a>> = sub.images.get(before..).unwrap_or_default().to_vec();
            self.images = std::mem::take(&mut sub.images);
            self.blobs = std::mem::take(&mut sub.blobs);
            self.work.set(sub.work.get());
            self.symbol_nodes = sub.symbol_nodes;
            self.mask_budget = sub.mask_budget;
            self.cut |= sub.cut;
            let colors: Vec<Color> = sub.samples.iter().map(|(c, _)| *c).collect();
            out.extend(&colors);
            self.pattern_colors.insert(name, (colors, reads, sub.cut.then_some(depth)));
        }
        out
    }
}

fn pick_theme(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "recolor.pickTheme";
    let picks = match p.get("picks") {
        Some(Value::Array(a)) if !a.is_empty() && a.len() <= MAX_PICKS => a.iter().map(|v| Pick::param(v, C)).collect::<Result<Vec<_>>>()?,
        Some(Value::Array(a)) => return Err(bad(C, format!("`picks` must hold 1 to {MAX_PICKS} picks (got {})", a.len()))),
        Some(v) => return Err(bad(C, format!("`picks` must be an array (got {v})"))),
        None => return Err(bad(C, "missing `picks`")),
    };
    let n = super::menucmds::whole_or(p, "colors", 5, C)?;
    if !(1..=MAX_THEME_COLORS).contains(&n) {
        return Err(bad(C, format!("`colors` must be between 1 and {MAX_THEME_COLORS} (got {n})")));
    }
    let st = s.doc()?;
    let d = &st.doc;
    let mut theme = Theme::new(d);
    theme.scope = st.isolation;
    // In isolation mode: the isolated object, everything inside it and the containers around it.
    let scope: Option<std::collections::HashSet<NodeId>> = st.isolation.and_then(|iso| d.node(iso)).map(|iso| {
        let mut allowed = std::collections::HashSet::new();
        iso.walk(&mut |m| {
            allowed.insert(m.id);
        });
        let mut up = d.parent_of(iso.id);
        while let Some(p) = up {
            allowed.insert(p);
            up = d.parent_of(p);
        }
        allowed
    });
    for pick in picks {
        match pick {
            Pick::At(pt) => {
                // In isolation mode only the isolated art (and the groups around it) can be hit;
                // isolated Shaper Group originals are hittable then.
                let opt = vectorcraft_doc::hit::HitOptions { scope: st.isolation, ..Default::default() };
                let hit = match &scope {
                    Some(allowed) => vectorcraft_doc::hit::hit_test_visible_skipping(d, pt, opt, &|id| !allowed.contains(&id)),
                    None => vectorcraft_doc::hit::hit_test_visible(d, pt, opt),
                };
                if let Some(n) = hit.and_then(|h| d.node(h.top_object(st.isolation))) {
                    theme.read(n, None, 0);
                }
            }
            Pick::Area(r) => match st.isolation.and_then(|iso| d.node(iso)) {
                // In isolation mode a marquee reads the isolated art alone.
                Some(iso) => theme.read(iso, Some(r), 0),
                None => {
                    for l in &d.layers {
                        theme.read(l, Some(r), 0);
                    }
                }
            },
        }
    }
    theme.read_images();
    let colors = vectorcraft_color::recolor::theme_palette(&theme.samples, index(n));
    Ok(json!({"colors": colors.iter().map(key_str).collect::<Vec<_>>(), "objects": theme.objects}))
}

/// A colour the art uses: how often, and the global swatch it is linked to.
struct Used {
    color: Color,
    count: usize,
    swatch: Option<String>,
}

/// The unique colours of the objects `ids` and the objects inside them, most used first (ties by
/// key): fills, strokes, gradient stops, text, mesh points and the tiles of the patterns used as
/// fills or strokes. With `only`, an object not in it is left out with everything it holds.
fn used_colors(d: &Document, ids: &[NodeId], only: Option<&BTreeSet<NodeId>>) -> Vec<Used> {
    fn read<'a>(n: &'a Node, only: Option<&BTreeSet<NodeId>>, out: &mut Vec<&'a Node>) {
        if only.is_some_and(|o| !o.contains(&n.id)) {
            return;
        }
        out.push(n);
        for c in n.children().into_iter().flatten() {
            read(c, only, out);
        }
    }
    let mut index: HashMap<ColorKey, usize> = HashMap::new();
    let mut used: Vec<(ColorKey, Used)> = vec![];
    let mut count = |c: &Color, link: Option<&str>| {
        let k = ColorKey::of(c);
        let i = *index.entry(k).or_insert_with(|| {
            used.push((k, Used { color: *c, count: 0, swatch: None }));
            used.len() - 1
        });
        let u = &mut used[i].1;
        u.count += 1;
        if u.swatch.is_none() {
            u.swatch = link.filter(|l| d.swatch(l).is_some_and(|w| w.global && w.paint.color().is_some())).map(str::to_string);
        }
    };
    // The patterns used as fills or strokes: their tiles count once each.
    let mut patterns: Vec<&str> = vec![];
    let mut nodes = vec![];
    for n in ids.iter().filter_map(|id| d.node(*id)) {
        read(n, only, &mut nodes);
    }
    for m in nodes {
        for p in node_paints(m).chain(override_paints(m)) {
            match p {
                Paint::Solid { color, swatch, .. } => count(color, swatch.as_deref()),
                Paint::Gradient(g) => g.gradient.stops.iter().for_each(|s| count(&s.color, s.swatch.as_deref())),
                Paint::Pattern { pattern, .. } if !patterns.contains(&pattern.as_str()) => patterns.push(pattern),
                _ => {}
            }
        }
        if let NodeKind::Mesh(mesh) = &m.kind {
            mesh.points.iter().for_each(|p| count(&p.color, None));
        }
    }
    for def in patterns.iter().filter_map(|p| d.pattern(p)) {
        def.art.iter().for_each(|n| node_colors(n, &mut count));
    }
    used.sort_by(|a, b| b.1.count.cmp(&a.1.count).then(a.0.cmp(&b.0)));
    used.into_iter().map(|(_, u)| u).collect()
}

/// The colors `n` shows, in paint order ([`node_colors`]), then the colors of the tiles of the
/// patterns it uses as fills or strokes, and of the patterns those tiles use (each pattern once).
fn shown_colors<'a>(d: &'a Document, n: &'a Node) -> Vec<Color> {
    let mut colors = vec![];
    node_colors(n, &mut |c, _| colors.push(*c));
    let mut patterns: Vec<&str> = vec![];
    pattern_names(n, &mut patterns);
    let mut next = 0;
    while let Some(&name) = patterns.get(next) {
        next += 1;
        for a in d.pattern(name).map(|def| def.art.as_slice()).unwrap_or_default() {
            node_colors(a, &mut |c, _| colors.push(*c));
            pattern_names(a, &mut patterns);
        }
    }
    colors
}

/// Adds the names of the patterns `n` and the objects inside it use as fills or strokes to `names`,
/// leaving out names already there.
fn pattern_names<'a>(n: &'a Node, names: &mut Vec<&'a str>) {
    n.walk(&mut |m| {
        for p in node_paints(m) {
            if let Paint::Pattern { pattern, .. } = p
                && !names.contains(&pattern.as_str())
            {
                names.push(pattern);
            }
        }
    });
}

/// The objects whose colors `recolor.colors` and `recolor.reduce` read, for [`used_colors`]: `ids`
/// (each checked) or else the selection, with everything they hold; with nothing selected, the
/// objects Select All takes and the objects inside them, leaving out every hidden or locked object
/// with what it holds and the objects of hidden, locked and template layers (the second value
/// lists the objects read). Objects listed twice or inside another one listed are dropped, so
/// their colors count once.
fn scope_ids(s: &Session, p: &Value, cmd: &str) -> Result<(Vec<NodeId>, Option<BTreeSet<NodeId>>)> {
    let st = s.doc()?;
    let ids = match p.get("ids") {
        Some(_) => checked_ids_param(s, p, "ids", cmd)?,
        None if st.selection.is_empty() => {
            let only = super::select::selectable_ids(&st.doc, st.isolation);
            return Ok((super::select::selectable(&st.doc, st.isolation), Some(only)));
        }
        None => st.selection.objects.clone(),
    };
    let mut seen = BTreeSet::new();
    Ok((super::edit::outermost(&st.doc, ids).into_iter().filter(|id| seen.insert(*id)).collect(), None))
}

fn colors(s: &mut Session, p: &Value) -> Result<Value> {
    let (ids, only) = scope_ids(s, p, "recolor.colors")?;
    let st = s.doc()?;
    let v: Vec<Value> = used_colors(&st.doc, &ids, only.as_ref())
        .iter()
        .map(|u| {
            let mut v = json!({"key": key_str(&u.color), "hex": u.color.to_hex(), "count": u.count});
            if let Some(w) = &u.swatch {
                v["swatch"] = json!(w);
            }
            v
        })
        .collect();
    Ok(json!({ "colors": v }))
}

/// The `method` parameter (`default` without one, or for null). A `method` that is not a string,
/// such as `5`, names no method.
fn method_param(p: &Value, cmd: &str, default: Method) -> Result<Method> {
    let (m, named) = match p.get("method") {
        None | Some(Value::Null) => return Ok(default),
        Some(Value::String(m)) => (m.clone(), true),
        Some(v) => (v.to_string(), false),
    };
    Method::parse(&m).filter(|_| named).ok_or_else(|| {
        let ids: Vec<&str> = Method::ALL.iter().map(|m| m.id()).collect();
        bad(cmd, format!("unknown method `{m}` ({})", ids.join("|")))
    })
}

fn key_str(c: &Color) -> Value {
    json!(ColorKey::of(c).to_string())
}

fn reduce(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "recolor.reduce";
    let method = method_param(p, C, Method::ScaleTints)?;
    let palette = super::swatchlib::limit_param(s, p, C)?;
    let def = Preserve::default();
    let given = match p.get("preserve") {
        None | Some(Value::Null) => &Value::Null,
        Some(v @ Value::Object(_)) => v,
        Some(_) => return Err(bad(C, "`preserve` must be an object")),
    };
    let flag = |k: &str, d: bool| switch_or(given, k, d, C);
    let preserve = Preserve { white: flag("white", def.white)?, black: flag("black", def.black)?, grays: flag("grays", def.grays)? };
    let colors = match p.get("colors") {
        None | Some(Value::Null) => None,
        Some(_) => Some(index(super::menucmds::whole_or(p, "colors", 0, C)?)),
    };
    let (ids, only) = scope_ids(s, p, C)?;
    let st = s.doc()?;
    let (kept, used): (Vec<Used>, Vec<Used>) = used_colors(&st.doc, &ids, only.as_ref()).into_iter().partition(|u| preserve.keeps(&u.color));
    // Units that stay together: the tints of a global swatch, else one colour each.
    let mut units: Vec<Vec<&Used>> = vec![];
    for u in &used {
        match units.iter_mut().find(|g| u.swatch.is_some() && g[0].swatch == u.swatch) {
            Some(g) => g.push(u),
            None => units.push(vec![u]),
        }
    }
    let unit_colors = |g: &[&Used]| g.iter().map(|u| u.color).collect::<Vec<_>>();
    let rows: Vec<Vec<&Used>> = match colors {
        Some(n) if n < units.len() => {
            let keys: Vec<Color> = units.iter().map(|g| method.key_color(&unit_colors(g)).unwrap_or_default()).collect();
            let weights: Vec<f32> = units.iter().map(|g| g.iter().map(|u| u.count as f32).sum()).collect();
            cluster(&keys, &weights, n).into_iter().map(|ix| ix.into_iter().flat_map(|i| units[i].iter().copied()).collect()).collect()
        }
        _ => units,
    };
    let map: Vec<Value> = rows
        .iter()
        .map(|row| {
            let mut from = unit_colors(row);
            let key = method.key_color(&from).unwrap_or_default();
            // The row's colour first.
            if let Some(i) = from.iter().position(|c| *c == key) {
                from[..=i].rotate_right(1);
            }
            let to = palette.as_ref().map_or(key, |pl| pl.nearest(key));
            json!({"from": from.iter().map(key_str).collect::<Vec<_>>(), "to": key_str(&to)})
        })
        .collect();
    Ok(json!({"map": map, "preserved": kept.iter().map(|u| key_str(&u.color)).collect::<Vec<_>>(), "method": method.id()}))
}

/// The `globalAdjust` parameter: `{saturation?, brightness?, temperature?, luminosity?}`, each a
/// percentage from -100 to 100 (0 when left out or null), as amounts from -1 to 1.
fn adjust_param(p: &Value, cmd: &str) -> Result<GlobalAdjust> {
    let given = match p.get("globalAdjust") {
        None | Some(Value::Null) => return Ok(GlobalAdjust::default()),
        Some(v @ Value::Object(_)) => v,
        Some(_) => return Err(bad(cmd, "`globalAdjust` must be an object")),
    };
    let amount = |key: &str| -> Result<f32> {
        let x = super::number_or(given, key, 0.0, cmd)?;
        let (min, max) = (-100, 100);
        if !(f64::from(min)..=f64::from(max)).contains(&x) {
            return Err(bad(cmd, format!("`{key}` must be between {min} and {max} (got {x})")));
        }
        Ok((x / 100.0) as f32)
    };
    Ok(GlobalAdjust {
        saturation: amount("saturation")?,
        brightness: amount("brightness")?,
        temperature: amount("temperature")?,
        luminosity: amount("luminosity")?,
    })
}

/// A current colour of a row: an exact colour, or every colour shown as a hex.
enum Match {
    Key(ColorKey),
    Hex(String),
}

impl Match {
    /// A colour key, a hex (`#rrggbb`) or a colour value. `None` for a hex color with an alpha
    /// part (`#rrggbbaa` or `#rgba`).
    fn of(v: &Value) -> Option<Self> {
        if let Some(s) = v.as_str() {
            if let Some(k) = ColorKey::parse(s) {
                return Some(Match::Key(k));
            }
            return Color::from_hex(s).filter(|_| !super::hex_with_alpha(s)).map(|c| Match::Hex(c.to_hex()));
        }
        color_value(v).map(|c| Match::Key(ColorKey::of(&c)))
    }
    fn color(&self) -> Color {
        match self {
            Match::Key(k) => k.color(),
            Match::Hex(h) => Color::from_hex(h).unwrap_or_default(),
        }
    }
    fn to_json(&self) -> Value {
        match self {
            Match::Key(k) => json!(k.to_string()),
            Match::Hex(h) => json!(h),
        }
    }
}

/// A row of the map: its current colours and its new colour; an excluded row keeps its colours.
struct Row {
    from: Vec<Match>,
    to: Option<Color>,
    exclude: bool,
    /// The tint (0..1) of a global or spot swatch the new color is (with `limitTo`): art linked to
    /// the swatch takes this share of the tint the method gives it.
    tint: f32,
    /// The row's own `model`, which takes the place of the command's for its colors.
    model: Option<ModelRule>,
}

/// The color model a row's colors take (`model`): each color's own, the new color's, or the
/// document's.
#[derive(Clone, Copy, PartialEq)]
enum ModelRule {
    Keep,
    New,
    Document,
}

impl ModelRule {
    /// The `model` of `v` (a row or the command's parameters); a given `model` must be a string.
    fn param(v: &Value, cmd: &str) -> Result<Option<Self>> {
        match v.get("model") {
            None => Ok(None),
            Some(Value::String(m)) => match m.as_str() {
                "keep" => Ok(Some(Self::Keep)),
                "new" => Ok(Some(Self::New)),
                "document" => Ok(Some(Self::Document)),
                other => Err(bad(cmd, format!("unknown model `{other}` (keep, new or document)"))),
            },
            Some(other) => Err(bad(cmd, format!("bad model {other} (keep, new or document)"))),
        }
    }

    fn id(self) -> &'static str {
        match self {
            Self::Keep => "keep",
            Self::New => "new",
            Self::Document => "document",
        }
    }
}

impl Row {
    /// The new colour, unless the row is excluded.
    fn new_color(&self) -> Option<Color> {
        self.to.filter(|_| !self.exclude)
    }
}

/// The rows of a `map` parameter, in order: an array of `{from, to, exclude?}` or an object of
/// `current: new` pairs.
fn parse_rows(p: &Value, cmd: &str) -> Result<Vec<Row>> {
    let new = |v: Option<&Value>| -> Result<Option<Color>> {
        match v {
            None | Some(Value::Null) => Ok(None),
            Some(v) => color_value(v).map(Some).ok_or_else(|| bad(cmd, format!("bad new colour {v}"))),
        }
    };
    let cur = |v: &Value| Match::of(v).ok_or_else(|| bad(cmd, format!("bad current colour {v}")));
    match p.get("map") {
        Some(Value::Object(m)) => {
            m.iter().map(|(k, v)| Ok(Row { from: vec![cur(&json!(k))?], to: new(Some(v))?, exclude: false, tint: 1.0, model: None })).collect()
        }
        Some(Value::Array(rows)) => rows
            .iter()
            .map(|r| {
                let from = match r.get("from") {
                    Some(Value::Array(a)) => a.iter().map(cur).collect::<Result<Vec<_>>>()?,
                    Some(v) => vec![cur(v)?],
                    None => return Err(bad(cmd, "each row needs `from`")),
                };
                let tint = (super::number_or(r, "tint", 100.0, cmd)? / 100.0).clamp(0.0, 1.0) as f32;
                let model = ModelRule::param(r, cmd)?;
                Ok(Row { from, to: new(r.get("to"))?, exclude: switch_or(r, "exclude", false, cmd)?, tint, model })
            })
            .collect(),
        _ => Err(bad(cmd, "missing `map` (rows [{from, to}] or {current: new})")),
    }
}

/// The document swatch a row's new colour is: its name, whether it is global (art links to it,
/// with tints) and the colour art linked to it shows (a process colour's own colour).
#[derive(Clone)]
struct Link {
    name: String,
    global: bool,
    color: Color,
    /// The row's tint of the swatch (0..1).
    tint: f32,
}

/// A global swatch of the color group `recolor.apply` rewrites: its name, the color it is
/// rewritten with and the color art linked to it shows afterwards.
struct GroupSwatch {
    name: String,
    color: Color,
    shown: Color,
}

/// A parsed map ready to recolour: the row of each current colour, and the swatch of each row's
/// new colour.
struct ColorMap {
    rows: Vec<RowMap>,
    links: Vec<Option<Link>>,
    /// The new color of each of `rows`.
    tos: Vec<Color>,
    /// The global swatches of the color group being rewritten.
    group: Vec<GroupSwatch>,
    keys: HashMap<ColorKey, usize>,
    hexes: HashMap<String, usize>,
    /// Set when a colour took a new colour of another model in its own model.
    converted: std::cell::Cell<bool>,
    /// The Global Adjust every mapped colour takes after its row's new colour.
    adjust: GlobalAdjust,
}

impl ColorMap {
    /// `links`: the swatch of each of `rows`' new colours (as many as `rows`, or none); `group`: the
    /// global swatches of the color group being rewritten; `model`: the model rule of a row without
    /// its own; `adjust`: the Global Adjust of each mapped colour.
    fn new(rows: &[Row], method: Method, model: ModelRule, links: &[Option<Link>], group: Vec<GroupSwatch>, adjust: GlobalAdjust) -> Self {
        let mut out = Self {
            rows: vec![],
            links: vec![],
            tos: vec![],
            group,
            keys: HashMap::new(),
            hexes: HashMap::new(),
            converted: Default::default(),
            adjust,
        };
        for (n, r) in rows.iter().enumerate().filter(|(_, r)| !r.from.is_empty()) {
            let Some(to) = r.new_color() else { continue };
            let i = out.rows.len();
            let colors: Vec<Color> = r.from.iter().map(Match::color).collect();
            let link = links.get(n).cloned().flatten();
            // Colors that can't link to the swatch take the row's tint of it too.
            let mapped = link.as_ref().filter(|l| l.global && l.tint < 1.0).map_or(to, |l| to.tinted(l.tint));
            out.rows.push(RowMap::new(method, &colors, mapped).with_own_model(r.model.unwrap_or(model) != ModelRule::Keep));
            // Compared with the group's new colours, which take the adjustment too.
            out.tos.push(adjust.apply(to));
            out.links.push(link);
            for m in &r.from {
                match m {
                    Match::Key(k) => out.keys.entry(*k).or_insert(i),
                    Match::Hex(h) => out.hexes.entry(h.clone()).or_insert(i),
                };
            }
        }
        out
    }

    /// The row of current colour `c`.
    fn row_of(&self, c: Color) -> Option<usize> {
        self.keys.get(&ColorKey::of(&c)).or_else(|| if self.hexes.is_empty() { None } else { self.hexes.get(&c.to_hex()) }).copied()
    }

    fn map(&self, c: Color) -> Color {
        let Some(r) = self.row_of(c).and_then(|i| self.rows.get(i)) else { return c };
        if r.converts(c) {
            self.converted.set(true);
        }
        self.adjust.apply(r.map(c))
    }

    /// Recolour a colour that can link to a swatch: a row whose new colour is a global or spot
    /// swatch links its colours to it (at their tints times the row's tint; shades and other hues
    /// stay unlinked), a
    /// process swatch gives its colour in its own model; a color linked to a global swatch of the
    /// rewritten group that takes the row's new color keeps its link (at its tint in the row) when
    /// the row gives it the new color in that color's model and the Global Adjust gives it the
    /// swatch's new color at that tint;
    /// other rows map the colour, which loses its link when it changes or when the group's rewrite
    /// would change it. True when anything changed.
    fn map_link(&self, c: &mut Color, link: &mut Option<String>, tint: &mut f32) -> bool {
        let Some(i) = self.row_of(*c) else { return false };
        let (Some(row), Some(l)) = (self.rows.get(i), self.links.get(i)) else { return false };
        if l.is_none()
            && let Some(g) = link.as_deref().and_then(|n| self.group.iter().find(|g| g.name == n))
            && self.tos.get(i) == Some(&g.color)
            && !row.converts(*c)
            && let Some(t) = row.tint(*c).map(|t| t.clamp(0.0, 1.0))
            && (self.adjust.is_identity() || same_ink(self.adjust.apply(row.map(*c)), g.shown.tinted(t)))
        {
            let shown = g.shown.tinted(t);
            let changed = *tint != t || *c != shown;
            (*c, *tint) = (shown, t);
            return changed;
        }
        let new = match (l, row.tint(*c)) {
            (Some(l), Some(t)) if l.global => {
                let t = (t * l.tint).clamp(0.0, 1.0);
                let shown = l.color.tinted(t);
                let changed = link.as_deref() != Some(l.name.as_str()) || *tint != t || *c != shown;
                (*c, *link, *tint) = (shown, Some(l.name.clone()), t);
                return changed;
            }
            (Some(l), Some(t)) => l.color.tinted(t),
            _ => {
                if row.converts(*c) {
                    self.converted.set(true);
                }
                self.adjust.apply(row.map(*c))
            }
        };
        // A color its row leaves as it is still loses its link to a swatch of the rewritten group
        // when the rewrite would give it another color.
        let rewritten = link.as_deref().and_then(|n| self.group.iter().find(|g| g.name == n)).is_some_and(|g| g.shown.tinted(*tint) != new);
        if new == *c && !rewritten {
            return false;
        }
        *c = new;
        *link = None;
        true
    }
}

fn apply(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "recolor.apply";
    let method = method_param(p, C, Method::Exact)?;
    let palette = super::swatchlib::limit_param(s, p, C)?;
    let mut rows = parse_rows(p, C)?;
    let adjust = adjust_param(p, C)?;
    // With Limit to Library a row's new colour takes the Global Adjust before it snaps, so the art
    // still takes the library's colours; otherwise every colour the rows map takes it.
    let mapped_adjust = if palette.is_some() { GlobalAdjust::default() } else { adjust };
    if let Some(pl) = &palette {
        for r in &mut rows {
            r.to = r.to.map(|c| pl.nearest(adjust.apply(c)));
        }
    }
    // With Limit to Library, a row's new colour is a swatch of the library: the art takes that
    // swatch (a spot colour stays a spot colour), which joins the document's swatches.
    let lib_swatches = match str_param(p, "limitTo").filter(|k| !k.is_empty()) {
        Some(k) => super::swatchlib::library_swatches(s, k).unwrap_or_default(),
        None => vec![],
    };
    let row_swatches: Vec<Option<Swatch>> =
        rows.iter().map(|r| r.new_color().and_then(|c| lib_swatches.iter().find(|w| w.paint.color() == Some(c)).cloned())).collect();
    // While an object of a dynamic symbol instance's art is selected, the selection is refused, as
    // `selected_roots` refuses it.
    super::edit::refuse_symbol_part(s)?;
    let ids = s.doc()?.selection.objects.clone();
    let left = {
        let st = s.doc()?;
        left_out(&st.doc, st.isolation, &super::edit::roots_of(&st.doc, ids.clone()))
    };
    let group = text_param(p, "group", C)?;
    if ids.is_empty() && group.is_none() {
        return Err(bad(C, "select artwork to recolour, or give a colour `group`"));
    }
    let scope =
        Scope { fill: true, stroke: true, images: switch_or(p, "includeImages", true, C)?, patterns: switch_or(p, "includePatterns", true, C)? };
    let label = if ids.is_empty() { "Edit Color Group" } else { "Recolor Artwork" };
    // `model`: keep each colour's model (default), take the new colour's own, or the document's.
    // A given `model` must be a string; only a missing one means "keep". A row's own `model`
    // takes its place for that row.
    let model = ModelRule::param(p, C)?.unwrap_or(ModelRule::Keep);
    let mode_model = s.doc()?.doc.color_mode.model();
    for r in rows.iter_mut().filter(|r| r.model.unwrap_or(model) == ModelRule::Document) {
        r.to = r.to.map(|c| c.in_model(mode_model));
    }
    let doc_model = (model == ModelRule::Document).then_some(mode_model);
    // The colours a colour group takes follow the model too (`document` converts them).
    let in_doc_model = |c: Color| doc_model.map_or(c, |m| c.in_model(m));
    let group_colors = match p.get("groupColors") {
        None | Some(Value::Null) => None,
        Some(Value::Array(cs)) => Some(cs),
        Some(_) => return Err(bad(C, "`groupColors` must be an array")),
    };
    let new: Vec<Color> = match group_colors {
        Some(cs) => cs
            .iter()
            .map(|v| {
                color_value(v)
                    .map(|c| adjust.apply(c))
                    .map(|c| in_doc_model(palette.as_ref().map_or(c, |pl| pl.nearest(c))))
                    .ok_or_else(|| bad(C, format!("bad colour {v} in groupColors")))
            })
            .collect::<Result<_>>()?,
        None => rows.iter().filter_map(Row::new_color).map(|c| mapped_adjust.apply(c)).collect(),
    };
    let rename = text_param(p, "rename", C)?;
    let colorized_ok = super::paint::check_colorized(&s.doc()?.doc).is_ok();
    let (changed, edit, added, converted, not_changed) = s.edit(label, |d, _| {
        // The rows' library swatches join the document (one it has with that name and colour is
        // used as it is).
        let mut added = vec![];
        let links: Vec<Option<Link>> = row_swatches
            .iter()
            .zip(&rows)
            .map(|(w, row)| {
                let w = w.as_ref()?;
                let name = match d.swatch(&w.name) {
                    Some(x) if x.paint == w.paint => w.name.clone(),
                    _ => {
                        let name = d.free_swatch_name(&w.name);
                        d.swatches.push(Swatch { name: name.clone(), ..w.clone() });
                        added.push(name.clone());
                        name
                    }
                };
                let color = d.global_color(&name).filter(|_| w.global).or(w.paint.color())?;
                Some(Link { name, global: w.global, color, tint: row.tint })
            })
            .collect();
        // The group's global swatches that its rewrite gives a color: art linked to one of them
        // whose row's new color is that color stays linked.
        let kept: Vec<GroupSwatch> = group
            .and_then(|g| d.swatch_groups.iter().find(|x| x.name == g))
            .map(|g| {
                g.swatches
                    .iter()
                    .zip(&new)
                    .filter(|(w, _)| w.global && !w.is_reserved())
                    .map(|(w, c)| GroupSwatch { name: w.name.clone(), color: *c, shown: d.linked_color(*c, w.spot) })
                    .collect()
            })
            .unwrap_or_default();
        let map = ColorMap::new(&rows, method, model, &links, kept, mapped_adjust);
        let f = |c: Color| map.map(c);
        let lf = |c: &mut Color, link: &mut Option<String>, tint: &mut f32| map.map_link(c, link, tint);
        // The objects left out whose colors the pass would change, with the colors they show now.
        // The trial pass does not count towards the note on color models.
        let before: Vec<(NodeId, Vec<Color>, bool)> = Recolor::with_links(scope, &f, &lf)
            .would_change(d, &left)
            .into_iter()
            .filter_map(|(id, hidden)| Some((id, shown_colors(d, d.node(id)?), hidden)))
            .collect();
        map.converted.set(false);
        let changed = Recolor::with_links(scope, &f, &lf).run(d, &ids);
        let edit = group.map(|g| super::swatch::rewrite_group(d, g, &new, rename, C)).transpose()?;
        super::paint::check_colorized_kept(colorized_ok, d).map_err(|e| bad(C, e))?;
        // Rewriting the group's swatches can change the colors of art linked to them, left out or
        // not, and of the patterns it uses. Art that only loses its link keeps its colors and is
        // counted.
        let not_changed = NotChanged::of(
            before.iter().filter(|(id, colors, _)| d.node(*id).is_some_and(|now| shown_colors(d, now) == *colors)).map(|(_, _, hidden)| *hidden),
        );
        Ok((changed, edit, added, map.converted.get(), not_changed))
    })?;
    let mut out = json!({ "changed": changed, "swatches": added });
    not_changed.report(&mut out);
    if let Some(e) = edit {
        e.defaults(s);
        out["relinked"] = json!(e.relinked());
    }
    if converted {
        out["note"] = json!(
            "some colours kept their own colour model, so the new colour was converted into it and may look different; \
             pass model \"new\" (the new colour's model) or \"document\" (the document's colour mode) to change the model"
        );
    }
    Ok(out)
}

fn randomize(_: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "recolor.randomize";
    let rows = parse_rows(p, C)?;
    let mut rng = Rng::new(super::menucmds::whole_or(p, "seed", 0, C)?);
    let (order, saturation_brightness) = (switch_or(p, "order", true, C)?, switch_or(p, "saturationBrightness", false, C)?);
    // Each new color with its row's tint and model: a tint moves with its color, and goes when
    // the color changes; a model moves with its color.
    let mut news: Vec<(Color, f32, Option<ModelRule>)> = rows.iter().filter_map(|r| r.new_color().map(|c| (c, r.tint, r.model))).collect();
    if order {
        rng.shuffle(&mut news);
    }
    if saturation_brightness {
        news = news
            .into_iter()
            .map(|(c, tint, model)| {
                let n = rng.saturation_brightness(c);
                (n, if ColorKey::of(&n) == ColorKey::of(&c) { tint } else { 1.0 }, model)
            })
            .collect();
    }
    let mut news = news.into_iter();
    let map: Vec<Value> = rows
        .iter()
        .map(|r| {
            let (to, tint, model) =
                if r.new_color().is_some() { news.next().map_or((None, 1.0, None), |(c, t, m)| (Some(c), t, m)) } else { (r.to, r.tint, r.model) };
            let mut row = json!({"from": r.from.iter().map(Match::to_json).collect::<Vec<_>>(), "to": to.as_ref().map(key_str)});
            if r.exclude {
                row["exclude"] = json!(true);
            }
            if tint < 1.0 {
                row["tint"] = json!(f64::from((tint * 10_000.0).round()) / 100.0);
            }
            if let Some(m) = model {
                row["model"] = json!(m.id());
            }
            row
        })
        .collect();
    Ok(json!({ "map": map }))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use vectorcraft_doc::{Appearance, ImageBlob, MAX_SYMBOL_NODES, PatternDef, Symbol};
    use vectorcraft_geom::{Affine, BezPath, FillRule, shapes};

    use super::*;

    const RED: Color = Color::Rgb { r: 1.0, g: 0.0, b: 0.0 };
    const BLUE: Color = Color::Rgb { r: 0.0, g: 0.0, b: 1.0 };

    /// A 10 pt square from (`x`, 0) painted `fill`.
    fn square(d: &mut Document, x: f64, fill: Paint) -> Arc<Node> {
        let p = shapes::rectangle(Rect::new(x, 0.0, x + 10.0, 10.0));
        Arc::new(Node::path(d.alloc_id(), p, Appearance::basic(fill, Paint::None, 0.0)))
    }

    /// Symbol `name` whose art is a group of `art`, and an instance of it.
    fn symbol(d: &mut Document, name: &str, art: Vec<Arc<Node>>) -> Arc<Node> {
        let group = Node::group(d.alloc_id(), art);
        d.symbols.push(Symbol { dynamic: false, name: name.into(), art: Arc::new(group) });
        Arc::new(Node::new(d.alloc_id(), NodeKind::SymbolInstance { symbol: name.into(), xf: Affine::IDENTITY }))
    }

    fn has(t: &Theme, c: Color) -> bool {
        t.samples.iter().any(|(s, _)| ColorKey::of(s) == ColorKey::of(&c))
    }

    /// The symbol instances read share one limit on the objects they place: with room for one
    /// instance's art (a group and a square, 2 objects), the first instance is read and the
    /// second, of another symbol, is left out.
    #[test]
    fn symbol_instances_share_one_limit_on_placed_objects() {
        let mut d = Document::new(100.0, 100.0);
        let red = square(&mut d, 0.0, Paint::solid(RED));
        let a = symbol(&mut d, "Red", vec![red]);
        let blue = square(&mut d, 20.0, Paint::solid(BLUE));
        let b = symbol(&mut d, "Blue", vec![blue]);
        let mut t = Theme::new(&d);
        t.symbol_nodes = MAX_SYMBOL_NODES - 3;
        t.read(&a, None, 0);
        t.read(&b, None, 0);
        assert!(has(&t, RED), "the first instance fits");
        assert!(!has(&t, BLUE), "the second does not");
        assert_eq!(t.symbol_nodes, MAX_SYMBOL_NODES - 1);
    }

    /// An instance left out at the limit shows nothing, so its own paints are left out too.
    #[test]
    fn an_instance_past_the_limit_gives_none_of_its_own_paints() {
        let mut d = Document::new(100.0, 100.0);
        let red = square(&mut d, 0.0, Paint::solid(RED));
        let mut inst = (*symbol(&mut d, "Red", vec![red])).clone();
        inst.appearance = Appearance::basic(Paint::solid(BLUE), Paint::None, 0.0).into();
        let mut t = Theme::new(&d);
        t.read(&inst, None, 0);
        assert!(has(&t, BLUE), "the instance's own fill, with room for its art");
        let mut t = Theme::new(&d);
        t.symbol_nodes = MAX_SYMBOL_NODES - 1;
        t.read(&inst, None, 0);
        assert!(t.samples.is_empty() && t.objects == 0, "nothing past the limit: {:?}", t.samples);
    }

    /// An instance without 9-slice guides is counted as the renderer counts it, without the art of
    /// its opacity masks: a square with a mask of 6 objects places 2.
    #[test]
    fn an_instance_is_counted_without_its_mask_art() {
        let mut d = Document::new(100.0, 100.0);
        let leaf = square(&mut d, 0.0, Paint::solid(BLUE));
        let mut mask = vectorcraft_doc::OpacityMask::new(Node::group(d.alloc_id(), vec![leaf; 5]), true);
        mask.disabled = true;
        let mut red = (*square(&mut d, 0.0, Paint::solid(RED))).clone();
        red.extras_mut().mask = Some(Box::new(mask));
        let inst = symbol(&mut d, "Masked", vec![Arc::new(red)]);
        let mut t = Theme::new(&d);
        t.symbol_nodes = MAX_SYMBOL_NODES - 3;
        t.read(&inst, None, 0);
        assert!(has(&t, RED));
        assert_eq!(t.symbol_nodes, MAX_SYMBOL_NODES - 1);
    }

    /// Nested instances charge the limit at every level: a tree of four leaf instances (5
    /// objects) whose leaves place 2 objects each places 13 objects in all, and with 9 objects of
    /// room it places the tree and two of its leaves.
    #[test]
    fn nested_symbol_instances_charge_every_level() {
        let mut d = Document::new(100.0, 100.0);
        let red = square(&mut d, 0.0, Paint::solid(RED));
        let leaf = symbol(&mut d, "Leaf", vec![red]);
        let tree = symbol(&mut d, "Tree", vec![leaf; 4]);
        let mut t = Theme::new(&d);
        t.read(&tree, None, 0);
        assert_eq!(t.symbol_nodes, 13);
        let mut t = Theme::new(&d);
        t.symbol_nodes = MAX_SYMBOL_NODES - 9;
        t.read(&tree, None, 0);
        assert_eq!(t.symbol_nodes, MAX_SYMBOL_NODES, "the tree and two leaves");
        assert!(has(&t, RED));
    }

    /// Patterns `P0` to `P{n}`: each tile a group holding a square filled with the next pattern,
    /// the last a red square. Gives a square filled with `P0`.
    fn pattern_chain(d: &mut Document, n: usize) -> Arc<Node> {
        let fill = |i: usize| Paint::Pattern { pattern: format!("P{i}"), xf: Affine::IDENTITY };
        for i in 0..=n {
            let inner = if i == n { square(d, 0.0, Paint::solid(RED)) } else { square(d, 0.0, fill(i + 1)) };
            let tile = Arc::new(Node::group(d.alloc_id(), vec![inner]));
            d.patterns.push(PatternDef::new(&format!("P{i}"), vec![tile]));
        }
        square(d, 0.0, fill(0))
    }

    /// Pattern tiles are read one level deeper than the object their pattern fills, so a chain of
    /// patterns shares one limit on depth with the art around it: the end of a chain of 50 is
    /// read, the end of a chain of 200 (two levels for each tile) is not.
    #[test]
    fn pattern_tiles_share_one_depth_limit() {
        let mut d = Document::new(100.0, 100.0);
        let short = pattern_chain(&mut d, 50);
        let mut t = Theme::new(&d);
        t.read(&short, None, 0);
        assert!(has(&t, RED), "a chain of 50");
        let mut d = Document::new(100.0, 100.0);
        let long = pattern_chain(&mut d, 200);
        let p100 = square(&mut d, 0.0, Paint::Pattern { pattern: "P100".into(), xf: Affine::IDENTITY });
        let mut t = Theme::new(&d);
        t.read(&long, None, 0);
        assert!(!has(&t, RED), "a chain of 200 passes {MAX_THEME_DEPTH} levels");
        // The patterns cut short there are read again when used less deep: from P100 the end is
        // 100 patterns away.
        t.read(&p100, None, 0);
        assert!(has(&t, RED), "P100 read again at the top");
    }

    /// A pattern tile's symbol instances charge the same limit as the art around it.
    #[test]
    fn pattern_tiles_charge_the_symbol_limit() {
        let mut d = Document::new(100.0, 100.0);
        let red = square(&mut d, 0.0, Paint::solid(RED));
        let inst = symbol(&mut d, "Red", vec![red]);
        d.patterns.push(PatternDef::new("Dots", vec![inst]));
        let art = square(&mut d, 0.0, Paint::Pattern { pattern: "Dots".into(), xf: Affine::IDENTITY });
        let mut t = Theme::new(&d);
        t.read(&art, None, 0);
        assert!(has(&t, RED));
        assert_eq!(t.symbol_nodes, 2, "the tile's instance placed a group and a square");
        let mut t = Theme::new(&d);
        t.symbol_nodes = MAX_SYMBOL_NODES - 1;
        t.read(&art, None, 0);
        assert!(!has(&t, RED), "no room for the tile's instance");
    }

    /// A `size` px image of one color `rgba`.
    fn blob(rgba: [u8; 4], size: u32) -> ImageBlob {
        let mut png = Vec::new();
        image::RgbaImage::from_pixel(size, size, image::Rgba(rgba)).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        ImageBlob::png(png)
    }

    /// A read of all of `blob`, placed at the origin 1 pt a pixel, inside a clipping path over
    /// it of `elements` path elements.
    fn clipped(blob: &ImageBlob, size: f64, elements: usize) -> ImageRead<'_> {
        let mut bp = BezPath::new();
        bp.move_to((0.0, 0.0));
        let steps = elements.saturating_sub(4).max(1);
        for i in 1..=steps {
            bp.line_to((size * i as f64 / steps as f64, 0.0));
        }
        bp.line_to((size, size));
        bp.line_to((0.0, size));
        bp.close_path();
        ImageRead { blob, xf: Affine::IDENTITY, size: (size, size), area: None, clip: Some(vec![(bp, FillRule::NonZero)]), colorize: None }
    }

    /// The reads of one image inside clip groups share the work limit: three reads of a 64 px
    /// image, each inside a clipping path of 3,000 elements, test at most [`THEME_WORK`] elements
    /// in all and still read the image.
    #[test]
    fn reads_of_one_image_share_the_clip_work_limit() {
        let d = Document::new(100.0, 100.0);
        let red = blob([255, 0, 0, 255], 64);
        let mut t = Theme::new(&d);
        t.images = (0..3).map(|_| clipped(&red, 64.0, 3000)).collect();
        let tested = t.read_images();
        assert!(tested > 0 && tested <= THEME_WORK, "{tested} elements tested");
        assert!(has(&t, RED));
    }

    /// A read of an image outside clip groups keeps its own pixel budget when another read of the
    /// same image inside a clip group gets only two pixels: the blue 8 px block in the corner of a
    /// red 64 px image is still read.
    #[test]
    fn unclipped_reads_keep_their_budget_beside_clipped_reads() {
        let d = Document::new(100.0, 100.0);
        let red = blob([255, 0, 0, 255], 64);
        let mut png = Vec::new();
        image::RgbaImage::from_fn(64, 64, |x, y| image::Rgba(if x >= 56 && y >= 56 { [0, 0, 255, 255] } else { [255, 0, 0, 255] }))
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let corner = ImageBlob::png(png);
        let mut t = Theme::new(&d);
        let whole = ImageRead { clip: None, ..clipped(&corner, 64.0, 4) };
        // The red read leaves 6,400 elements of work; 3,200 elements a pixel leave two pixels.
        t.images = vec![clipped(&red, 64.0, 3900), whole, clipped(&corner, 64.0, 3200)];
        let tested = t.read_images();
        assert!(tested <= THEME_WORK, "{tested} elements tested");
        assert!(has(&t, BLUE), "the read outside clip groups samples the whole image");
    }

    /// When the work left can't test a pixel of every clipped read of an image, the reads that fit
    /// are still read: with 6,400 elements left, a read of a blue image inside a clipping path
    /// of 5,000 elements is read beside one of 2,000 elements, which is left out.
    #[test]
    fn clipped_reads_that_fit_the_work_left_are_read() {
        let d = Document::new(100.0, 100.0);
        let (red, blue) = (blob([255, 0, 0, 255], 64), blob([0, 0, 255, 255], 64));
        let mut t = Theme::new(&d);
        t.images = vec![clipped(&red, 64.0, 3900), clipped(&blue, 64.0, 5000), clipped(&blue, 64.0, 2000)];
        let tested = t.read_images();
        assert!(tested <= THEME_WORK, "{tested} elements tested");
        assert!(has(&t, BLUE), "the read that fits is read");
    }

    /// Past the work limit, the reads of an image inside clip groups are left out: a red image
    /// inside a clipping path of 3,900 elements takes nearly all of it, and a blue image after it,
    /// inside one of 100,000 elements, is not read.
    #[test]
    fn clipped_image_reads_past_the_work_limit_are_left_out() {
        let d = Document::new(100.0, 100.0);
        let (red, blue) = (blob([255, 0, 0, 255], 64), blob([0, 0, 255, 255], 64));
        let mut t = Theme::new(&d);
        t.images = vec![clipped(&red, 64.0, 3900), clipped(&blue, 64.0, 100_000)];
        let tested = t.read_images();
        assert!(tested <= THEME_WORK, "{tested} elements tested");
        assert!(has(&t, RED));
        assert!(!has(&t, BLUE), "the blue image is past the limit");
    }
}
