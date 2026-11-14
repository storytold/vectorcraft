//! EMF (with or without EMF+ records) and WMF files: `craft_metafile` plays the records into a
//! picture, and the picture's items become document objects here.
//!
//! - A filled or stroked path becomes a path with a fill and a stroke (caps, joins, miter limit
//!   and dashes from the pen). Strokes drawn one after another, each starting where the last one
//!   ended, with the same pen and clip, join into one path.
//! - A path filled with a pattern or hatched brush is filled with the average color of the
//!   brush's tile.
//! - A bitmap becomes an image (its pixels as PNG) placed where the picture draws it, turned or
//!   skewed with it.
//! - Text becomes point type in the font the file names, underlined or struck through as the font
//!   is, and clipped to its rectangle when the file clips it.
//! - Clip regions and clip paths become clipping groups around what they clip; a rectangle around
//!   the whole picture clips nothing and is left out.
//!
//! A WMF without a placeable header is read at 1440 units an inch, and one whose placeable box is
//! empty or inverted at the units an inch the header gives. An EMF without a usable frame is read
//! at its device's pixels a millimetre (96 pixels an inch when the header doesn't say).

use std::collections::HashMap;
use std::sync::Arc;

use craft_metafile::{Cap, ClipPath, HAlign, Item, Join, LineStyle, Seg, VAlign};
use vectorcraft_color::Paint;
use vectorcraft_doc::clipnest::{Clip, Drawn, MAX_NEST, deep_clips, nest};
use vectorcraft_doc::{
    Appearance, AppearanceItem, CharStyle, Dash, Document, FillLayer, ImageBlob, ImageObject, Justify, LayerColor, LineCap, LineJoin, Node, NodeId,
    NodeKind, StrokeLayer, TextObject,
};
use vectorcraft_geom::{Affine, BezPath, FillRule, PathData, PathEl, Point, Rect, Shape, shapes};

use super::{BAD_IMAGE, FAR_AWAY, HAIRLINE, LIMIT, MAX_NODES, around, rgb, sane};
use crate::dib::Rgba;
use crate::wmf::PLACEABLE_KEY;
use crate::{Imported, Kind};

const PATTERNS: &str = "hatched and pattern brushes are filled with their average color";
const ENDS_EARLY: &str = "the file ends early: what it held was imported";

/// Points a unit of a WMF without a placeable header (1440 units an inch).
const WMF_UNIT: f64 = 72.0 / 1440.0;
/// Points a millimetre.
const PT_PER_MM: f64 = 72.0 / 25.4;

/// Points a picture unit across and down. `craft_metafile` gives a placeable WMF with a box and an
/// EMF with a frame in points, and other files in their logical (WMF) or device (EMF) units.
fn unit(bytes: &[u8], kind: Kind) -> (f64, f64) {
    let u16_at = |at: usize| bytes.get(at..at.checked_add(2)?).map(|b| u16::from_le_bytes([b[0], b[1]]));
    let i16_at = |at: usize| bytes.get(at..at.checked_add(2)?).map(|b| i16::from_le_bytes([b[0], b[1]]));
    let u32_at = |at: usize| bytes.get(at..at.checked_add(4)?).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    let i32_at = |at: usize| bytes.get(at..at.checked_add(4)?).map(|b| i32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    let positive = |v: Option<i32>| v.filter(|v| *v > 0).map(f64::from);
    match kind {
        Kind::Wmf => {
            if bytes.get(..4) != Some(&PLACEABLE_KEY.to_le_bytes()[..]) {
                return (WMF_UNIT, WMF_UNIT);
            }
            if let (Some(l), Some(t), Some(r), Some(b)) = (i16_at(6), i16_at(8), i16_at(10), i16_at(12))
                && r > l
                && b > t
            {
                return (1.0, 1.0);
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
                _ => (0.75, 0.75),
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

/// Clip paths already made, by address (the `Arc` keeps the address from being reused); None for
/// one that clips nothing in the picture.
type MadeClips = HashMap<*const ClipPath, (Arc<ClipPath>, Option<Arc<Clip>>)>;
/// The last clip region and the clip made for it.
type MadeRegion = (Vec<[f32; 4]>, Option<Arc<Clip>>);

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
            return;
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
                    let pen = Pen { color: c, width: w, line, even_odd, chain: chain.iter().map(|c| c.id).collect() };
                    let start = bp.elements().first().and_then(|e| match e {
                        PathEl::MoveTo(p) => Some(*p),
                        _ => None,
                    });
                    if let Some(p) = &mut self.pending
                        && p.pen == pen
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
                let Some(c) = average(&tile.rgba) else { return };
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
        let s: String = t.text.chars().filter(|c| *c != '\0').collect();
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
        // A negative height is the em; a positive one the cell (the em and the internal leading).
        let em = if h < 0.0 { -h } else { h * 0.85 };
        let size = if em > 0.0 && (em * k).is_finite() { (em * k).clamp(0.5, 5000.0) } else { 12.0 };
        let mut style = CharStyle { size, fill: color(t.color), stroke: Paint::None, ..CharStyle::default() };
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
        style.underline = f.underline;
        style.strikethrough = f.strike;
        // Where the baseline is below the reference point.
        let drop = match t.v_align {
            VAlign::Baseline => 0.0,
            VAlign::Bottom => -0.2 * size,
            VAlign::Top => 0.8 * size,
        };
        let origin = self.xf * Point::new(t.transform[4], t.transform[5]);
        let mut text = TextObject::point(Point::ZERO, &s, style);
        text.xf = Affine::translate(origin.to_vec2()) * shape * Affine::translate((0.0, drop));
        text.para.justify = match t.h_align {
            HAlign::Center => Justify::Center,
            HAlign::Right => Justify::Right,
            HAlign::Left => Justify::Left,
        };
        text.cached_bounds = Some(vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &text).bounds);
        let mut n = Node::new(NodeId(0), NodeKind::Text(Box::new(text)));
        n.opacity = f32::from(t.color[3]) / 255.0;
        self.push(chain, n);
    }

    /// The document: what was drawn on one layer, clipped art in clipping groups.
    fn finish(mut self, kind: Kind, skipped: u32) -> Imported {
        self.flush();
        if skipped > 0 {
            let n = skipped;
            self.warnings.push(format!(
                "{n} record{} of kinds VectorCraft doesn't read {} skipped",
                if n == 1 { "" } else { "s" },
                if n == 1 { "was" } else { "were" }
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
    for item in pic.items {
        m.item(item);
    }
    Ok(m.finish(kind, pic.skipped_records))
}
