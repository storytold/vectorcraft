//! EMF (with or without EMF+ records) and WMF files: `craft_metafile` plays the records into a
//! picture, and the picture's items become document objects here.
//!
//! - A filled or stroked path becomes a path with a fill and a stroke (caps, joins, miter limit
//!   and dashes from the pen). Opaque solid strokes with round caps and round joins drawn one after
//!   another, each starting where the last one ended, with the same pen and clip, join into one
//!   path. Other strokes stay apart, each with its own caps and its own dashes.
//! - A path filled with a pattern or hatched brush is filled with the average color of the
//!   brush's tile.
//! - A bitmap becomes an image (its pixels as PNG) placed where the picture draws it, turned or
//!   skewed with it.
//! - Text becomes point type in the font the file names, underlined or struck through as the font
//!   is, and clipped to its rectangle when the file clips it. Characters the file gives advances
//!   for are spaced by them: tracking on each character, with pair kerning off. On a single left
//!   to right line, the characters after an advance that tracking can't give (further than its
//!   range reaches, or a tab) are point type of their own, and an underline or strikethrough is carried across the space between.
//!   It is also carried on to the end of the last character's advance. Type on more than one line
//!   is placed a line at a time: each line is aligned by its own advances and split as a single
//!   line is.
//! - Clip regions and clip paths become clipping groups around what they clip; a rectangle around
//!   the whole picture clips nothing and is left out.
//!
//! A WMF without a placeable header is read at 1440 units an inch, and one whose placeable box is
//! empty or inverted at the units an inch the header gives. Either one that draws in a metric map
//! mode (MM_LOMETRIC to MM_TWIPS) is read at 96 pixels an inch, as `craft_metafile` plays those
//! modes. An EMF without a usable frame is read at its device's pixels a millimetre (96
//! pixels an inch when the header doesn't say).

use std::collections::HashMap;
use std::sync::{Arc, Weak};

use craft_metafile::{Cap, ClipPath, HAlign, Item, Join, LineStyle, Seg, VAlign};
use vectorcraft_color::Paint;
use vectorcraft_doc::clipnest::{Clip, Drawn, MAX_NEST, deep_clips, nest};
use vectorcraft_doc::{
    Appearance, AppearanceItem, CharStyle, Dash, Document, FillLayer, ImageBlob, ImageObject, Justify, LayerColor, LineCap, LineJoin, Node, NodeId,
    NodeKind, StrokeLayer, TextObject, TextRun,
};
use vectorcraft_geom::{Affine, BezPath, FillRule, PathData, PathEl, Point, Rect, Shape, shapes};

use super::{BAD_IMAGE, FAR_AWAY, HAIRLINE, LIMIT, MAX_NODES, around, rgb, sane};
use crate::dib::Rgba;
use crate::wmf::PLACEABLE_KEY;
use crate::{Imported, Kind};

const PATTERNS: &str = "hatched and pattern brushes are filled with their average color";
const ENDS_EARLY: &str = "the file ends early: what it held was imported";
const TOO_MANY: &str = "objects past the most one file may hold were left out";
const AT_LIMIT: &str = "the file or a picture inside it reached a limit on what one file may hold: what came after the limit was not read";

/// Points a unit of a WMF without a placeable header (1440 units an inch).
const WMF_UNIT: f64 = 72.0 / 1440.0;
/// The SAVEDC levels `craft_metafile` keeps; it stops reading at a SAVEDC past them.
const SAVED_LEVELS: usize = 1024;
/// Points a pixel at 96 pixels an inch.
const PT_PER_PX: f64 = 72.0 / 96.0;
/// Points a millimetre.
const PT_PER_MM: f64 = 72.0 / 25.4;

/// WMF records that draw lines and shapes, which `craft_metafile` plays only when the mix mode
/// lets them draw.
const WMF_SHAPES: [u16; 10] = [0x0213, 0x0324, 0x0325, 0x0418, 0x041B, 0x0538, 0x061C, 0x0817, 0x081A, 0x0830];
/// WMF records that draw text and bitmaps, which `craft_metafile` plays whatever the mix mode.
const WMF_DRAWS: [u16; 5] = [0x0521, 0x0940, 0x0A32, 0x0B41, 0x0F43];
/// SETROP2 mix modes under which `craft_metafile` draws no lines, shapes or brush fills: R2_XORPEN
/// and R2_NOP.
const WMF_MIX_SKIPS: [u16; 2] = [7, 11];
/// The mix mode a WMF starts in (R2_COPYPEN).
const WMF_MIX_START: u16 = 13;
/// PATBLT raster operations `craft_metafile` draws: PATCOPY (with the brush, as the mix mode
/// allows), BLACKNESS and WHITENESS.
const PATCOPY: u32 = 0x00F0_0021;
const BLACKNESS: u32 = 0x0000_0042;
const WHITENESS: u32 = 0x00FF_0062;

/// Does a WMF draw in a metric map mode (MM_LOMETRIC to MM_TWIPS)? `craft_metafile` plays those
/// at 96 pixels an inch, whatever the window. The mode that more drawing records are played in
/// decides, counting only records the reader draws (not lines and shapes under R2_NOP or
/// R2_XORPEN, nor a PATBLT it does not fill); a file that draws nothing goes by its last mode.
/// `start` is where the standard header begins.
fn wmf_metric(bytes: &[u8], start: usize) -> bool {
    let u16_at = |at: usize| bytes.get(at..at.checked_add(2)?).map(|b| u16::from_le_bytes([b[0], b[1]]));
    let u32_at = |at: usize| bytes.get(at..at.checked_add(4)?).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    let Some(words) = start.checked_add(2).and_then(u16_at) else { return false };
    let mut at = start.saturating_add(usize::from(words) * 2);
    let mut mode = 1;
    let mut mix = WMF_MIX_START;
    let metric = |mode: u16| (2..=6).contains(&mode);
    // Drawing records played in a metric mode, and in any other.
    let (mut in_metric, mut in_other) = (0usize, 0usize);
    // The map and mix modes SAVEDC kept, which RESTOREDC brings back as the reader does.
    let mut saved: Vec<(u16, u16)> = Vec::new();
    // The records as the reader walks them: it stops at the end record, a record shorter than its
    // header, one that runs past the file and a SAVEDC past its 1,024 levels.
    while let (Some(words), Some(function)) = (u32_at(at), u16_at(at.saturating_add(4))) {
        let size = (words as usize).saturating_mul(2);
        let Some(next) = at.checked_add(size) else { break };
        if function == 0 || size < 6 || next > bytes.len() {
            break;
        }
        match function {
            // SETMAPMODE, when the record holds its mode.
            0x0103 if size >= 8 => mode = u16_at(at + 6).unwrap_or(mode),
            // SETROP2, when the record holds its mix mode.
            0x0104 if size >= 8 => mix = u16_at(at + 6).unwrap_or(mix),
            // SAVEDC.
            0x001E => {
                if saved.len() >= SAVED_LEVELS {
                    break;
                }
                saved.push((mode, mix));
            }
            // RESTOREDC: a negative count pops that many levels, a positive one goes back to that
            // level. One that names no saved level changes nothing.
            0x0127 if size >= 8 => {
                let n = u16_at(at + 6).map_or(0, |v| i32::from(i16::from_le_bytes(v.to_le_bytes())));
                let level = if n < 0 { saved.len().checked_sub(n.unsigned_abs() as usize) } else { (n as usize).checked_sub(1) };
                if let Some(level) = level
                    && let Some((m, x)) = saved.get(level).copied()
                {
                    mode = m;
                    mix = x;
                    saved.truncate(level);
                }
            }
            f => {
                let draws = match f {
                    _ if WMF_DRAWS.contains(&f) => true,
                    _ if WMF_SHAPES.contains(&f) => !WMF_MIX_SKIPS.contains(&mix),
                    // PATBLT: its raster operation, when the record holds one.
                    0x061D => match u32_at(at + 6).filter(|_| size >= 10) {
                        Some(PATCOPY) => !WMF_MIX_SKIPS.contains(&mix),
                        Some(rop) => rop == BLACKNESS || rop == WHITENESS,
                        None => false,
                    },
                    _ => false,
                };
                if draws && metric(mode) {
                    in_metric += 1;
                } else if draws {
                    in_other += 1;
                }
            }
        }
        at = next;
    }
    if in_metric == in_other { metric(mode) } else { in_metric > in_other }
}

/// Points a picture unit across and down. `craft_metafile` gives a placeable WMF with a box and an
/// EMF with a frame in points, and other files in their logical units (WMF; pixels in a metric map
/// mode) or device units (EMF).
fn unit(bytes: &[u8], kind: Kind) -> (f64, f64) {
    let u16_at = |at: usize| bytes.get(at..at.checked_add(2)?).map(|b| u16::from_le_bytes([b[0], b[1]]));
    let i16_at = |at: usize| bytes.get(at..at.checked_add(2)?).map(|b| i16::from_le_bytes([b[0], b[1]]));
    let u32_at = |at: usize| bytes.get(at..at.checked_add(4)?).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    let i32_at = |at: usize| bytes.get(at..at.checked_add(4)?).map(|b| i32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    let positive = |v: Option<i32>| v.filter(|v| *v > 0).map(f64::from);
    match kind {
        Kind::Wmf => {
            let placeable = bytes.get(..4) == Some(&PLACEABLE_KEY.to_le_bytes()[..]);
            if placeable
                && let (Some(l), Some(t), Some(r), Some(b)) = (i16_at(6), i16_at(8), i16_at(10), i16_at(12))
                && r > l
                && b > t
            {
                return (1.0, 1.0);
            }
            if wmf_metric(bytes, if placeable { 22 } else { 0 }) {
                return (PT_PER_PX, PT_PER_PX);
            }
            if !placeable {
                return (WMF_UNIT, WMF_UNIT);
            }
            let k = 72.0 / u16_at(14).filter(|v| *v > 0).map_or(1440.0, f64::from);
            (k, k)
        }
        Kind::Emf => {
            // Device pixels a millimetre on each axis: szlDevice over szlMicrometers when the header
            // has them, else over szlMillimeters (both sizes must be positive).
            let device = positive(i32_at(72)).zip(positive(i32_at(76)));
            let micro = (u32_at(4).is_some_and(|s| s >= 108))
                .then(|| u32_at(100).zip(u32_at(104)).filter(|(w, h)| *w > 0 && *h > 0).map(|(w, h)| (f64::from(w) / 1000.0, f64::from(h) / 1000.0)))
                .flatten();
            let mm = micro.or_else(|| positive(i32_at(80)).zip(positive(i32_at(84))));
            let px_mm = device.zip(mm).map(|((w, h), (mw, mh))| (w / mw, h / mh));
            let frame = matches!((i32_at(24), i32_at(28), i32_at(32), i32_at(36)), (Some(l), Some(t), Some(r), Some(b)) if r > l && b > t);
            let ok = |v: f64| v.is_finite() && v > 0.0;
            match px_mm {
                Some(_) if frame => (1.0, 1.0),
                Some((x, y)) if ok(x) && ok(y) => (PT_PER_MM / x, PT_PER_MM / y),
                _ => (PT_PER_PX, PT_PER_PX),
            }
        }
    }
}

/// What a stroke is drawn with, to tell whether two strokes join into one path.
#[derive(Clone, PartialEq)]
struct Pen {
    color: [u8; 4],
    width: f32,
    line: LineStyle,
    even_odd: bool,
    chain: Vec<u32>,
}

/// Strokes drawn one after another, not added yet: their pen, clips and path (document space).
struct Pending {
    pen: Pen,
    clips: Vec<Arc<Clip>>,
    path: BezPath,
}

/// What `craft_metafile` counted as not read or not played as the file asks.
struct Lost {
    /// Records of kinds it doesn't read.
    skipped: u32,
    /// Images, and pattern brushes (a pen's included), whose bitmap it couldn't read.
    images: u32,
    /// Clip changes it didn't apply as the file sets them.
    clips: u32,
    /// Raster operations drawn as a plain copy or left out.
    rops: u32,
}

/// Clip paths already made, by address (the `Arc` keeps the address from being reused); None for
/// one that clips nothing in the picture.
type MadeClips = HashMap<*const ClipPath, (Arc<ClipPath>, Option<Arc<Clip>>)>;
/// The last clip region and the clip made for it.
type MadeRegion = (Vec<[f32; 4]>, Option<Arc<Clip>>);
/// Brush tiles already averaged, by address: items filled with the same brush share its tile. The
/// `Weak` keeps the address from being reused, and the tile's pixels are freed once the last item
/// that uses it has been read.
type Averaged = HashMap<*const craft_metafile::Tile, (Weak<craft_metafile::Tile>, Option<[u8; 4]>)>;

struct Mapper {
    /// Picture units → points.
    xf: Affine,
    /// How much lengths (stroke widths, dashes) grow from the picture to points.
    k: f64,
    frame: Rect,
    doc: Document,
    out: Drawn,
    warnings: Vec<String>,
    next_clip: u32,
    paths: MadeClips,
    region: Option<MadeRegion>,
    pending: Option<Pending>,
    averaged: Averaged,
}

fn color(c: [u8; 4]) -> Paint {
    rgb([c[0], c[1], c[2]])
}

/// The average color of a tile's pixels, weighted by their alpha, and their average alpha; None
/// for a clear tile.
fn average(rgba: &[u8]) -> Option<[u8; 4]> {
    let pixels = rgba.as_chunks::<4>().0;
    let mut sum = [0u64; 4];
    for p in pixels {
        let a = u64::from(p[3]);
        sum[0] += u64::from(p[0]) * a;
        sum[1] += u64::from(p[1]) * a;
        sum[2] += u64::from(p[2]) * a;
        sum[3] += a;
    }
    let a = sum[3];
    let alpha = (a / pixels.len().max(1) as u64) as u8;
    (a > 0).then(|| [(sum[0] / a) as u8, (sum[1] / a) as u8, (sum[2] / a) as u8, alpha.max(1)])
}

impl Mapper {
    fn new(width: f64, height: f64, (kx, ky): (f64, f64)) -> Self {
        let side = |v: f64| if v.is_finite() { v.clamp(1.0, LIMIT) } else { 1.0 };
        let (w, h) = (side(width * kx), side(height * ky));
        let mut doc = Document::new(w, h);
        doc.layers.clear();
        Self {
            xf: Affine::scale_non_uniform(kx, ky),
            // The mean of the two, as the reader scales stroke lengths.
            k: (kx + ky) / 2.0,
            frame: Rect::new(0.0, 0.0, w, h),
            doc,
            out: vec![],
            warnings: vec![],
            next_clip: 0,
            paths: HashMap::new(),
            region: None,
            pending: None,
            averaged: HashMap::new(),
        }
    }

    fn warn(&mut self, w: &str) {
        if !self.warnings.iter().any(|x| x == w) {
            self.warnings.push(w.to_string());
        }
    }

    fn path(&self, segs: &[Seg]) -> BezPath {
        let mut bp = craft_metafile::bez_path(segs);
        bp.apply_affine(self.xf);
        bp
    }

    /// A clip of `path` (document space), or None when it clips nothing in the picture.
    fn clip(&mut self, path: BezPath, rule: FillRule) -> Option<Arc<Clip>> {
        if around(&path, self.frame) {
            return None;
        }
        self.next_clip += 1;
        Some(Arc::new(Clip { id: self.next_clip, region: Some((path, rule)) }))
    }

    /// The clips an item is drawn under, outermost first; None when it shows nowhere.
    fn chain(&mut self, region: Option<Vec<[f32; 4]>>, clips: &[Arc<ClipPath>]) -> Option<Vec<Arc<Clip>>> {
        let mut chain = vec![];
        if let Some(rects) = region {
            if rects.is_empty() {
                return None;
            }
            let made = match &self.region {
                Some((last, made)) if *last == rects => made.clone(),
                _ => {
                    let mut bp = BezPath::new();
                    for [x0, y0, x1, y1] in &rects {
                        bp.extend(shapes::rectangle(Rect::new(f64::from(*x0), f64::from(*y0), f64::from(*x1), f64::from(*y1))).to_bezpath());
                    }
                    bp.apply_affine(self.xf);
                    if !sane(bp.bounding_box()) {
                        self.warn(FAR_AWAY);
                        return None;
                    }
                    let made = self.clip(bp, FillRule::NonZero);
                    self.region = Some((rects, made.clone()));
                    made
                }
            };
            chain.extend(made);
        }
        for c in clips {
            let key = Arc::as_ptr(c);
            let made = match self.paths.get(&key) {
                Some((_, made)) => made.clone(),
                None => {
                    let bp = self.path(&c.segs);
                    if !sane(bp.bounding_box()) {
                        self.warn(FAR_AWAY);
                        return None;
                    }
                    let made = self.clip(bp, if c.even_odd { FillRule::EvenOdd } else { FillRule::NonZero });
                    self.paths.insert(key, (c.clone(), made.clone()));
                    made
                }
            };
            chain.extend(made);
        }
        if chain.len() > MAX_NEST {
            chain.truncate(MAX_NEST);
            self.warn(&deep_clips());
        }
        Some(chain)
    }

    fn push(&mut self, clips: Vec<Arc<Clip>>, mut node: Node) {
        if self.out.len() >= MAX_NODES {
            return self.warn(TOO_MANY);
        }
        if !node.visual_bounds().is_none_or(sane) {
            return self.warn(FAR_AWAY);
        }
        node.id = self.doc.alloc_id();
        self.out.push((clips, node));
    }

    /// Add the strokes waiting to be joined.
    fn flush(&mut self) {
        if let Some(p) = self.pending.take() {
            let node = self.path_node(p.path, None, Some((p.pen.color, p.pen.width, &p.pen.line)), p.pen.even_odd);
            if let Some(n) = node {
                self.push(p.clips, n);
            }
        }
    }

    fn stroke_layer(&self, c: [u8; 4], width: f32, line: &LineStyle) -> StrokeLayer {
        let w = f64::from(width) * self.k;
        let mut st = StrokeLayer::new(color(c), if w > 0.0 && w.is_finite() { w } else { HAIRLINE });
        st.opacity = f32::from(c[3]) / 255.0;
        st.cap = match line.cap {
            Cap::Round => LineCap::Round,
            Cap::Square => LineCap::Square,
            Cap::Flat => LineCap::Butt,
        };
        st.join = match line.join {
            Join::Round => LineJoin::Round,
            Join::Bevel => LineJoin::Bevel,
            Join::Miter => LineJoin::Miter,
        };
        let miter = f64::from(line.miter_limit);
        if miter.is_finite() && miter >= 1.0 {
            st.miter_limit = miter;
        }
        let dash = Dash { pattern: line.dash.iter().map(|d| f64::from(*d) * self.k).collect(), offset: 0.0, align_corners: false };
        st.dash = dash.is_dashed().then_some(dash);
        st
    }

    /// A path node painted with `fill` and `stroke`; None when neither shows.
    fn path_node(&self, bp: BezPath, fill: Option<[u8; 4]>, stroke: Option<([u8; 4], f32, &LineStyle)>, even_odd: bool) -> Option<Node> {
        if bp.elements().is_empty() {
            return None;
        }
        let mut items = vec![];
        if let Some(c) = fill.filter(|c| c[3] > 0) {
            let mut f = FillLayer::new(color(c));
            f.opacity = f32::from(c[3]) / 255.0;
            items.push(AppearanceItem::Fill(f));
        }
        if let Some((c, w, line)) = stroke.filter(|s| s.0[3] > 0) {
            items.push(AppearanceItem::Stroke(self.stroke_layer(c, w, line)));
        }
        if items.is_empty() {
            return None;
        }
        let mut n = Node::path(NodeId(0), PathData::from_bezpath(&bp), Appearance { items, ..Appearance::default() });
        if let NodeKind::Path { rule, .. } = &mut n.kind {
            *rule = if even_odd { FillRule::EvenOdd } else { FillRule::NonZero };
        }
        Some(n)
    }

    fn item(&mut self, item: Item) {
        match item {
            Item::Path { segs, fill, stroke, line, even_odd, region, clips } => {
                let Some(chain) = self.chain(region, &clips) else { return self.flush() };
                let bp = self.path(&segs);
                // An open stroke with nothing filled waits, to join the next one.
                if let (None, Some((c, w))) = (fill, stroke)
                    && !segs.iter().any(|s| matches!(s, Seg::Close))
                {
                    // A stroke that reaches too far is left out here, before it joins the strokes
                    // waiting, so they are still added.
                    let half = f64::from(w) * self.k;
                    let half = if half > 0.0 && half.is_finite() { half / 2.0 } else { HAIRLINE / 2.0 };
                    if !sane(bp.bounding_box().inflate(half, half)) {
                        self.flush();
                        return self.warn(FAR_AWAY);
                    }
                    let pen = Pen { color: c, width: w, line, even_odd, chain: chain.iter().map(|c| c.id).collect() };
                    let start = bp.elements().first().and_then(|e| match e {
                        PathEl::MoveTo(p) => Some(*p),
                        _ => None,
                    });
                    // A translucent stroke stays apart, so where strokes overlap each one shows. So
                    // does a dashed one, whose dashes start at its own start, and one whose caps
                    // or joins aren't round: a join where two strokes meet is not their two caps.
                    if let Some(p) = &mut self.pending
                        && p.pen == pen
                        && pen.color[3] == u8::MAX
                        && pen.line.dash.is_empty()
                        && matches!((&pen.line.cap, &pen.line.join), (Cap::Round, Join::Round))
                        && start.is_some()
                        && p.path.elements().last().and_then(|e| e.end_point()) == start
                    {
                        p.path.extend(bp.elements().iter().skip(1).copied());
                        return;
                    }
                    self.flush();
                    self.pending = Some(Pending { pen, clips: chain, path: bp });
                    return;
                }
                self.flush();
                let stroke = stroke.map(|(c, w)| (c, w, &line));
                if let Some(n) = self.path_node(bp, fill, stroke, even_odd) {
                    self.push(chain, n);
                }
            }
            Item::Pattern { segs, even_odd, tile, region, clips } => {
                self.flush();
                let Some(chain) = self.chain(region, &clips) else { return };
                let Some(c) = self.averaged.entry(Arc::as_ptr(&tile)).or_insert_with(|| (Arc::downgrade(&tile), average(&tile.rgba))).1 else {
                    return;
                };
                self.warn(PATTERNS);
                let bp = self.path(&segs);
                if let Some(n) = self.path_node(bp, Some(c), None, even_odd) {
                    self.push(chain, n);
                }
            }
            Item::Bitmap { transform, width, height, rgba, region, clips, .. } => {
                self.flush();
                let Some(chain) = self.chain(region, &clips) else { return };
                self.image(Rgba { width, height, pixels: rgba }, Affine::new(transform), chain);
            }
            Item::Text(t) => {
                self.flush();
                let Some(mut chain) = self.chain(t.region.clone(), &t.clips) else { return };
                // The text's own clip rectangle, inside the others.
                if let Some([x0, y0, x1, y1]) = t.clip {
                    let mut bp = shapes::rectangle(Rect::new(f64::from(x0), f64::from(y0), f64::from(x1), f64::from(y1)).abs()).to_bezpath();
                    bp.apply_affine(self.xf);
                    if !sane(bp.bounding_box()) {
                        return self.warn(FAR_AWAY);
                    }
                    if let Some(own) = self.clip(bp, FillRule::NonZero) {
                        // Past the stack, the text's own clip takes the innermost place kept.
                        if chain.len() >= MAX_NEST {
                            chain.truncate(MAX_NEST.saturating_sub(1));
                            self.warn(&deep_clips());
                        }
                        chain.push(own);
                    }
                }
                self.text(&t, chain);
            }
        }
    }

    /// An image whose unit square `unit` places in the picture.
    fn image(&mut self, img: Rgba, unit: Affine, chain: Vec<Arc<Clip>>) {
        if img.width == 0 || img.height == 0 {
            return;
        }
        let xf = self.xf * unit * Affine::scale_non_uniform(1.0 / f64::from(img.width), 1.0 / f64::from(img.height));
        if !xf.as_coeffs().iter().all(|v| v.is_finite()) || xf.determinant().abs() < 1e-12 {
            return;
        }
        let Ok(png) = img.to_png() else { return self.warn(BAD_IMAGE) };
        let blob = ImageBlob::new("image/png", png);
        let key = blob.content_key();
        self.doc.images.entry(key.clone()).or_insert(blob);
        let im = ImageObject { key, width: img.width, height: img.height, xf, link: None, placement: Default::default(), pixels: None };
        self.push(chain, Node::new(NodeId(0), NodeKind::Image(Box::new(im))));
    }

    fn text(&mut self, t: &craft_metafile::Text, chain: Vec<Arc<Clip>>) {
        // The characters without NULs, and each one's advance (text units) when the file gives
        // one for every character: a NUL's advance goes to the character before it, or moves the
        // start when the string begins with one.
        let advances = t.dx.len() == t.text.chars().count() && t.dx.iter().all(|d| d.is_finite());
        let (mut s, mut dx, mut lead) = (String::new(), Vec::new(), 0.0f64);
        for (c, d) in t.text.chars().zip(t.dx.iter().map(|d| f64::from(*d)).chain(std::iter::repeat(0.0))) {
            if c != '\0' {
                s.push(c);
                dx.push(d);
            } else if let Some(last) = dx.last_mut() {
                *last += d;
            } else {
                lead += d;
            }
        }
        if !advances {
            dx.clear();
            lead = 0.0;
        }
        if s.trim().is_empty() || t.color[3] == 0 {
            return;
        }
        // Text space → document without the move: its vertical scale gives the type size, and what
        // is left of it (a turn, a stretch across) goes on the text object.
        let [a, b, c, d, ..] = t.transform;
        let linear = self.xf * Affine::new([a, b, c, d, 0.0, 0.0]);
        let [_, _, c, d, ..] = linear.as_coeffs();
        let k = c.hypot(d);
        let shape = if k > 1e-12 && k.is_finite() && linear.determinant().abs() > 1e-24 { Affine::scale(1.0 / k) * linear } else { Affine::IDENTITY };
        let f = &t.font;
        let h = f64::from(f.height);
        let mut style = CharStyle { fill: color(t.color), stroke: Paint::None, ..CharStyle::default() };
        if !f.family.trim().is_empty() {
            style.font_family = f.family.trim().to_string();
        }
        style.font_style = match (f.weight >= 600, f.italic) {
            (true, true) => "Bold Italic",
            (true, false) => "Bold",
            (false, true) => "Italic",
            (false, false) => "Regular",
        }
        .into();
        // The face the font maps to (the fallback face for a font that isn't installed). Characters
        // it has no glyph for are drawn from other faces at the same size and Horizontal Scale.
        let face = vectorcraft_text::FontDb::global().style_face(&style);
        // A negative height is the em. A positive height is the character cell, and the em is
        // the cell times unitsPerEm divided by the face's OS/2 usWinAscent plus usWinDescent at
        // its instance of a variable font (its own ascent plus descent when it has no OS/2 table).
        let cell_em = face.as_ref().and_then(|face| {
            let (a, d) = face.vertical_metrics();
            Some(face.units_per_em() / face.win_cell_height().unwrap_or(a + d)).filter(|r| r.is_finite() && *r > 0.0)
        });
        let em = if h < 0.0 { -h } else { h * cell_em.unwrap_or(0.85) };
        let sized = em > 0.0 && (em * k).is_finite();
        let size = if sized { (em * k).clamp(0.5, 5000.0) } else { 12.0 };
        style.size = size;
        // An average character width, negative or positive, becomes Horizontal Scale: the face's
        // own average width (OS/2 xAvgCharWidth) at the type size is stretched or narrowed to it,
        // also when the size was held to its range. A width is ignored when the height is 0.
        let width = f64::from(f.width).abs();
        if width > 0.0
            && sized
            && let Some(avg) = face.as_ref().and_then(|face| Some(face.average_char_width()? / face.units_per_em()))
        {
            let h_scale = width * k / (avg * size) * 100.0;
            if h_scale.is_finite() {
                style.h_scale = h_scale.clamp(1.0, 10000.0);
            }
        }
        style.underline = f.underline;
        style.strikethrough = f.strike;
        let db = vectorcraft_text::FontDb::global();
        // Characters placed by their advances take no pair kerning: the advances place them.
        if !dx.is_empty() {
            style.kerning = Some(0.0);
        }
        // Where the baseline is below the reference point.
        let drop = match t.v_align {
            VAlign::Baseline => 0.0,
            VAlign::Bottom => -0.2 * size,
            VAlign::Top => 0.8 * size,
        };
        let origin = self.xf * Point::new(t.transform[4], t.transform[5]);
        let base = Affine::translate(origin.to_vec2()) * shape;
        let justify = match t.h_align {
            HAlign::Center => Justify::Center,
            HAlign::Right => Justify::Right,
            HAlign::Left => Justify::Left,
        };
        let point = |s: &str| {
            let mut text = TextObject::point(Point::ZERO, s, style.clone());
            text.para.justify = justify;
            text
        };
        let whole = point(&s);
        let opacity = f32::from(t.color[3]) / 255.0;
        if dx.is_empty() {
            let mut text = whole;
            text.xf = base * Affine::translate((0.0, drop));
            text.cached_bounds = Some(vectorcraft_text::layout(db, &text).bounds);
            let mut n = Node::new(NodeId(0), NodeKind::Text(Box::new(text)));
            n.opacity = opacity;
            self.push(chain, n);
            return;
        }
        // Text units grow by `k` along the baseline in the text object's own space.
        let dx: Vec<f64> = dx.iter().map(|d| d * k).collect();
        let chars: Vec<char> = s.chars().collect();
        let line = Line { base, drop, align: &t.h_align, decorated: f.underline || f.strike, color: t.color, opacity, point: &point, chain: &chain };
        // Type on more than one line is placed a line at a time, each line aligned by its own
        // advances (layout leaves the tracking after a line's last character out of the width it
        // aligns, and each line ends with its own) and its underline carried on to its own end.
        let laid = vectorcraft_text::layout(db, &whole);
        if laid.lines.len() > 1 {
            // Where each character starts (bytes): the character a byte offset starts is found by
            // a search.
            let starts: Vec<usize> = s.char_indices().map(|(at, _)| at).collect();
            let count = |to: usize| starts.partition_point(|at| *at < to);
            let first = laid.lines.first().map_or(0.0, |l| l.baseline);
            for (n, l) in laid.lines.iter().enumerate() {
                let (from, to) = (count(l.start), count(l.end));
                let (Some(cs), Some(dx)) = (chars.get(from..to), dx.get(from..to)) else { continue };
                // A leading NUL's advance comes before the first line only.
                let lead = if n == 0 { lead * k } else { 0.0 };
                let down = l.baseline - first;
                self.line(&line, cs, dx, lead, if down.is_finite() { down } else { 0.0 });
            }
        } else {
            self.line(&line, &chars, &dx, lead * k, 0.0);
        }
    }

    /// One line of text with advances `dx` (points along the baseline) for its characters `chars`,
    /// `down` below the first line's baseline: in pieces, each a text object of its own, a new
    /// piece after each cluster that tracking can't space by its advance.
    fn line(&mut self, line: &Line, chars: &[char], dx: &[f64], lead: f64, down: f64) {
        if chars.is_empty() || (chars.iter().all(|c| c.is_whitespace()) && !line.decorated) {
            return;
        }
        let db = vectorcraft_text::FontDb::global();
        let s: String = chars.iter().collect();
        let (pieces, text, laid) = pieces((line.point)(&s), db, dx);
        let split = pieces.len() > 1;
        let mut tracked = Some((text, laid));
        // Along the baseline (the text object's own space), where the next piece starts from the
        // reference point: the line as long as its advances (a leading NUL's included), where the
        // alignment puts it.
        let mut at = aligned(lead, dx.iter().sum(), line.align);
        let last = pieces.len().saturating_sub(1);
        for (i, (from, to)) in pieces.into_iter().enumerate() {
            let piece: String = chars.get(from..to).unwrap_or_default().iter().collect();
            let dx = dx.get(from..to).unwrap_or_default();
            // Blanks of their own draw nothing unless they are underlined or struck through.
            if piece.trim().is_empty() && !line.decorated {
                at += dx.iter().sum::<f64>();
                continue;
            }
            // The line in one piece was already tracked to its advances.
            let (mut text, laid) = match tracked.take().filter(|_| !split) {
                Some(done) => done,
                None => {
                    let mut text = (line.point)(&piece);
                    let laid = advanced(&mut text, db, dx);
                    (text, laid)
                }
            };
            let x0 = laid.lines.first().map_or(0.0, |l| l.x0);
            let start = if (at - x0).is_finite() { at - x0 } else { 0.0 };
            let span: f64 = dx.iter().sum();
            at += span;
            text.xf = line.base * Affine::translate((start, line.drop + down));
            text.cached_bounds = Some(laid.bounds);
            // An underline or strikethrough runs on across the space an advance leaves before
            // the next piece, or after the line's last character, as one bar per piece from where
            // its own bars end.
            let gap = if (split && i < last) || (i == last && line.decorated) { gaps(&laid, db, &text, x0 + span) } else { BezPath::new() };
            let mut n = Node::new(NodeId(0), NodeKind::Text(Box::new(text.clone())));
            n.opacity = line.opacity;
            self.push(line.chain.to_vec(), n);
            if !gap.elements().is_empty() {
                let mut gap = gap;
                gap.apply_affine(text.xf);
                if let Some(n) = self.path_node(gap, Some(line.color), None, false) {
                    self.push(line.chain.to_vec(), n);
                }
            }
        }
    }

    /// The document: what was drawn on one layer, clipped art in clipping groups.
    fn finish(mut self, kind: Kind, lost: Lost) -> Imported {
        self.flush();
        let s = |n: u32| if n == 1 { "" } else { "s" };
        let was = |n: u32| if n == 1 { "was" } else { "were" };
        let n = lost.skipped;
        if n > 0 {
            self.warnings.push(format!("{n} record{} of kinds VectorCraft doesn't read {} skipped", s(n), was(n)));
        }
        let n = lost.images;
        if n > 0 {
            self.warnings.push(format!(
                "{n} image{} or pattern brush{} that couldn't be read {} left out",
                s(n),
                if n == 1 { "" } else { "es" },
                was(n)
            ));
        }
        let n = lost.clips;
        if n > 0 {
            self.warnings.push(format!(
                "{n} clip{} couldn't be applied as the file sets {}: what is drawn after {} may show more or less than the file does",
                s(n),
                if n == 1 { "it" } else { "them" },
                if n == 1 { "it" } else { "them" }
            ));
        }
        let n = lost.rops;
        if n > 0 {
            self.warnings.push(format!(
                "{n} raster operation{} other than a plain copy (masks, inversions) {} drawn as a plain copy or left out",
                s(n),
                was(n)
            ));
        }
        let out = std::mem::take(&mut self.out);
        let children = nest(&mut self.doc, out);
        let mut layer = Node::layer(self.doc.alloc_id(), "Layer 1", LayerColor::Preset(0));
        if let Some(c) = layer.children_mut() {
            *c = children;
        }
        self.doc.layers = vec![Arc::new(layer)];
        Imported { document: self.doc, kind, warnings: self.warnings }
    }
}

/// Where a line `lead + span` long (along the baseline) starts from the reference point, as
/// `align` puts it, and `lead` on from there: where its first character starts.
fn aligned(lead: f64, span: f64, align: &HAlign) -> f64 {
    let total = lead + span;
    lead + match align {
        HAlign::Left => 0.0,
        HAlign::Center => -total / 2.0,
        HAlign::Right => -total,
    }
}

/// What every piece of a text record's lines shares: the text space (`base`, with the baseline
/// `drop` below the reference point), the alignment, whether the type is underlined or struck
/// through, its color and opacity, a point type object of a string in its style, and its clips.
struct Line<'a> {
    base: Affine,
    drop: f64,
    align: &'a HAlign,
    decorated: bool,
    color: [u8; 4],
    opacity: f32,
    point: &'a dyn Fn(&str) -> TextObject,
    chain: &'a [Arc<Clip>],
}

/// The most times [`advanced`] lays the type out again to correct its tracking.
const ADVANCE_PASSES: usize = 3;

/// Sets `t`'s tracking so that each cluster (a ligature, a letter with its marks) advances by the
/// sum of its characters' entries of `dx` (points along the baseline), and gives its layout.
/// Tracking can change how the type is shaped (it drops standard ligatures past a limit), so the
/// tracking is measured against the type laid out with the last tracking and corrected, a few
/// times at most. A tracking that would pass the attribute's range is held at its end. When the
/// corrections still miss (a ligature that forms at one tracking and breaks at the next), standard
/// ligatures are turned off and the tracking is corrected again.
fn advanced(t: &mut TextObject, db: &vectorcraft_text::FontDb, dx: &[f64]) -> vectorcraft_text::TextLayout {
    let laid = tracked(t, db, dx);
    let (_, clusters) = clusters(t, &laid, dx);
    let settled = clusters.iter().all(|c| (c.want - c.has).abs() <= c.em * 1e-4 || !(TRACKING.0..=TRACKING.1).contains(&c.wanted()));
    if settled {
        return laid;
    }
    for r in &mut t.runs {
        if !r.style.features.iter().any(|f| f == "-liga") {
            r.style.features.push("-liga".to_string());
        }
    }
    tracked(t, db, dx)
}

/// `t` laid out with its tracking corrected towards `dx` at most [`ADVANCE_PASSES`] times.
fn tracked(t: &mut TextObject, db: &vectorcraft_text::FontDb, dx: &[f64]) -> vectorcraft_text::TextLayout {
    let mut laid = vectorcraft_text::layout(db, t);
    for _ in 0..ADVANCE_PASSES {
        match retracked(t, &laid, dx) {
            Some(runs) => t.runs = runs,
            None => break,
        }
        laid = vectorcraft_text::layout(db, t);
    }
    laid
}

/// The tracking range of a character (thousandths of an em), as the Character panel holds it.
const TRACKING: (f64, f64) = (-1000.0, 10000.0);

/// One cluster of characters (a ligature, a letter with its marks): the advance the file wants
/// for it and its advance laid out (points, tracking included), its em as its first character's
/// Horizontal Scale draws it (which tracking is measured in) and its tracking.
struct Cluster {
    want: f64,
    has: f64,
    em: f64,
    tracking: f64,
}

impl Cluster {
    /// The tracking that gives the cluster the advance the file wants, not held to the range.
    fn wanted(&self) -> f64 {
        self.tracking + (self.want - self.has) / self.em * 1000.0
    }
}

/// `t`'s clusters as `laid` sets them, with the sum of their characters' entries of `dx`, and the
/// cluster of each character.
fn clusters(t: &TextObject, laid: &vectorcraft_text::TextLayout, dx: &[f64]) -> (Vec<usize>, Vec<Cluster>) {
    // The advance of the cluster that starts at each byte offset.
    let mut own: HashMap<usize, f64> = HashMap::new();
    for g in &laid.glyphs {
        *own.entry(g.byte).or_insert(0.0) += g.advance;
    }
    let mut cluster = Vec::new();
    let mut clusters: Vec<Cluster> = Vec::new();
    let mut byte = 0;
    for r in &t.runs {
        let em = (r.style.size * vectorcraft_text::spacing_scale(&r.style, false)).max(1e-3);
        for c in r.text.chars() {
            let want = dx.get(cluster.len()).copied().unwrap_or(0.0);
            match (own.get(&byte), clusters.last_mut()) {
                (None, Some(last)) => last.want += want,
                (has, _) => clusters.push(Cluster { want, has: has.copied().unwrap_or(0.0), em, tracking: r.style.tracking }),
            }
            cluster.push(clusters.len().saturating_sub(1));
            byte += c.len_utf8();
        }
    }
    (cluster, clusters)
}

/// `t`'s characters in pieces (first and past the last character index) that tracking can space
/// by their advances `dx`: a piece ends after each cluster whose advance, once tracked, still
/// differs from the one the file gives (further than the tracking range reaches, or a tab, which
/// layout advances to its tab stop). The clusters are the ones the type has once tracked
/// towards its advances, so that a ligature the tracking breaks apart can split between its letters.
/// Type laid out right to left, or on more than one line, stays in one piece: pieces are placed
/// one after another left to right along a single line. Also gives `t` tracked to its advances and
/// its layout, for the type that stays in one piece.
fn pieces(mut t: TextObject, db: &vectorcraft_text::FontDb, dx: &[f64]) -> (Vec<(usize, usize)>, TextObject, vectorcraft_text::TextLayout) {
    let laid = advanced(&mut t, db, dx);
    let (cluster, clusters) = clusters(&t, &laid, dx);
    let mut out = vec![];
    let mut from = 0;
    if laid.lines.len() <= 1 && !laid.glyphs.iter().any(|g| g.rtl) {
        for (i, c) in cluster.iter().enumerate() {
            let ends = cluster.get(i + 1) != Some(c);
            if ends && clusters.get(*c).is_some_and(|c| (c.want - c.has).abs() > c.em * 1e-4) {
                out.push((from, i + 1));
                from = i + 1;
            }
        }
    }
    if from < cluster.len() || out.is_empty() {
        out.push((from, cluster.len()));
    }
    (out, t, laid)
}

/// The underline and strikethrough of `t` (laid out as `laid`) carried on to `end` along the
/// baseline, in text space: the runs of a piece each draw bars of their own, and only the bars
/// that end the piece (furthest along) are carried on.
fn gaps(laid: &vectorcraft_text::TextLayout, db: &vectorcraft_text::FontDb, t: &TextObject, end: f64) -> BezPath {
    let bars: Vec<Rect> = vectorcraft_text::decorations(laid, db, t).iter().map(|(_, bar)| bar.bounding_box()).collect();
    let far = bars.iter().map(|b| b.x1).fold(f64::NEG_INFINITY, f64::max);
    let mut bp = BezPath::new();
    for b in bars.into_iter().filter(|b| b.x1 >= far - 1e-6) {
        // A thousandth of a point is short of anything drawn.
        if end > b.x1 + 1e-3 && end.is_finite() {
            bp.extend(shapes::rectangle(Rect::new(b.x1, b.y0, end, b.y1)).to_bezpath());
        }
    }
    bp
}

/// `t`'s runs with each cluster's tracking corrected by how far its advance in `laid` is from the
/// sum of its characters' entries of `dx`; None when every cluster is already within a ten
/// thousandth of an em of it. Layout adds a cluster's tracking once, so every character of a
/// cluster takes the same value and a cluster stays in one run. Characters with the same tracking
/// share a run.
fn retracked(t: &TextObject, laid: &vectorcraft_text::TextLayout, dx: &[f64]) -> Option<Vec<TextRun>> {
    let (cluster, clusters) = clusters(t, laid, dx);
    if clusters.iter().all(|c| (c.want - c.has).abs() <= c.em * 1e-4) {
        return None;
    }
    let tracking: Vec<f64> = clusters
        .iter()
        .map(|c| {
            let v = c.wanted().clamp(TRACKING.0, TRACKING.1);
            // Hundredths of a thousandth of an em, so that characters spaced alike share a run.
            if v.is_finite() { (v * 100.0).round() / 100.0 } else { 0.0 }
        })
        .collect();
    let mut out: Vec<TextRun> = Vec::with_capacity(t.runs.len());
    let mut i = 0;
    for r in &t.runs {
        for c in r.text.chars() {
            let tracking = cluster.get(i).and_then(|k| tracking.get(*k)).copied().unwrap_or(0.0);
            i += 1;
            let style = CharStyle { tracking, ..r.style.clone() };
            match out.last_mut() {
                Some(last) if last.style == style && last.inline.is_none() && r.inline.is_none() => last.text.push(c),
                _ => out.push(TextRun { text: c.to_string(), style, inline: r.inline.clone() }),
            }
        }
    }
    Some(out)
}

/// Does the file stop before its end-of-file record?
fn ends_early(bytes: &[u8], kind: Kind) -> bool {
    let u32_at = |at: usize| bytes.get(at..at.checked_add(4)?).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    let u16_at = |at: usize| bytes.get(at..at.checked_add(2)?).map(|b| u16::from_le_bytes([b[0], b[1]]));
    let mut at = match kind {
        Kind::Emf => 0,
        Kind::Wmf => {
            let start = if u32_at(0) == Some(PLACEABLE_KEY) { 22 } else { 0 };
            let Some(words) = u16_at(start + 2) else { return true };
            start + usize::from(words) * 2
        }
    };
    loop {
        let size = match kind {
            Kind::Emf => {
                let (Some(kind), Some(size)) = (u32_at(at), u32_at(at.saturating_add(4))) else { return true };
                if kind == 14 {
                    return false;
                }
                // Records are a multiple of 4 bytes long; reading stops at one that isn't.
                if !size.is_multiple_of(4) {
                    return true;
                }
                size as usize
            }
            Kind::Wmf => {
                let (Some(words), Some(function)) = (u32_at(at), u16_at(at.saturating_add(4))) else { return true };
                if function == 0 {
                    return false;
                }
                (words as usize).saturating_mul(2)
            }
        };
        if size < 6 {
            return true;
        }
        at = at.saturating_add(size);
    }
}

/// Read an EMF or WMF file.
pub(crate) fn import(bytes: &[u8]) -> Result<Imported, String> {
    let kind = crate::sniff(bytes).ok_or("not an EMF or WMF file")?;
    let pic = craft_metafile::parse(bytes).map_err(|e| match e {
        craft_metafile::Error::NotMetafile => "not an EMF or WMF file".to_string(),
        craft_metafile::Error::BadHeader => format!("the {} header is damaged", kind.id().to_uppercase()),
        craft_metafile::Error::NoRecords => format!("the {} file holds no records", kind.id().to_uppercase()),
    })?;
    let k = unit(bytes, kind);
    let mut m = Mapper::new(f64::from(pic.width), f64::from(pic.height), k);
    if ends_early(bytes, kind) {
        m.warn(ENDS_EARLY);
    }
    if pic.truncated {
        m.warn(AT_LIMIT);
    }
    let lost = Lost { skipped: pic.skipped_records, images: pic.dropped_images, clips: pic.ignored_clips, rops: pic.unplayed_raster_ops };
    for item in pic.items {
        m.item(item);
    }
    Ok(m.finish(kind, lost))
}
