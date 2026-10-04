//! PDF → Document (hayro-interpret device that builds a VectorCraft node tree).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use hayro_interpret::font::Glyph;
use hayro_interpret::pattern::Pattern;
use hayro_interpret::shading::ShadingType;
use hayro_interpret::{
    ClipPath, Context, Device, GlyphDrawMode, Image, ImageData, InterpreterCache, InterpreterSettings, InterpreterWarning, LumaData, PathDrawMode,
    SoftMask, StrokeProps, interpret_page,
};
use hayro_syntax::Pdf;
use hayro_syntax::object::Name;
use kurbo::{Affine, BezPath, Point, Rect, Shape, Vec2};
use vectorcraft_color::{BlendMode, Color, Gradient, GradientGeom, GradientKind, GradientPaint, GradientStop, Paint};
use vectorcraft_doc::{
    Appearance, AppearanceItem, Artboard, Dash, Document, FillLayer, ImageBlob, ImageObject, LayerColor, LineCap, LineJoin, Node, NodeId, NodeKind,
    StrokeLayer,
};
use vectorcraft_geom::{FillRule, PathData};

use crate::{ImportOptions, ImportReport, PdfError};

/// Import a PDF (or PDF-compatible `.ai`) with default options.
pub fn import(bytes: &[u8]) -> Result<Document, PdfError> {
    import_with_report(bytes, &ImportOptions::default()).map(|r| r.document)
}

/// Import a PDF, returning the document plus warnings about content that was approximated or skipped.
pub fn import_with_report(bytes: &[u8], opts: &ImportOptions) -> Result<ImportReport, PdfError> {
    let pdf = Pdf::new(bytes.to_vec()).map_err(|e| PdfError::Parse(format!("{e:?}")))?;
    let pages = pdf.pages();
    let n = opts.max_pages.map_or(pages.len(), |m| m.min(pages.len()));
    if n == 0 {
        return Err(PdfError::NoPages);
    }

    let sink: Arc<Mutex<Vec<String>>> = Arc::default();
    let sink2 = sink.clone();
    let settings = InterpreterSettings {
        warning_sink: Arc::new(move |w| {
            let msg = match w {
                InterpreterWarning::UnsupportedFont => "a font could not be read; its text was skipped",
                InterpreterWarning::ImageDecodeFailure => "an image could not be decoded and was skipped",
            };
            if let Ok(mut v) = sink2.lock()
                && !v.iter().any(|x| x == msg)
            {
                v.push(msg.to_string());
            }
        }),
        render_annotations: false,
        ..Default::default()
    };

    let mut doc = Document::new(1.0, 1.0);
    doc.title = "Imported PDF".into();
    doc.artboards.clear();
    doc.layers.clear();
    let mut b = Builder::new(doc.peek_next_id());
    let cache = InterpreterCache::new();
    let mut x = 0.0;
    for (i, page) in pages.iter().take(n).enumerate() {
        let (w, h) = page.render_dimensions();
        let (w, h) = (w as f64, h as f64);
        let ab = Rect::new(x, 0.0, x + w, h);
        x += w + opts.artboard_gap;
        doc.artboards.push(Artboard {
            id: i as u32 + 1,
            name: format!("Artboard {}", i + 1),
            rect: ab,
            show_center_mark: false,
            show_cross_hairs: false,
        });
        let init = Affine::translate((ab.x0, ab.y0)) * Affine::new(page.initial_transform(true).as_coeffs());
        let mut ctx = Context::new(init, ab, &cache, pdf.xref(), settings.clone());
        b.page = ab;
        b.begin_page();
        interpret_page(page, &mut ctx, &mut b);
        let children = b.end_page();
        let mut layer = Node::layer(b.id(), &format!("Page {}", i + 1), LayerColor::Preset((i % 27) as u8));
        if let NodeKind::Layer { children: c, .. } = &mut layer.kind {
            *c = children;
        }
        doc.layers.push(Arc::new(layer));
    }
    for (k, blob) in b.images.drain() {
        doc.images.insert(k, blob);
    }
    doc.fix_next_id();

    let mut warnings = b.warnings;
    if let Ok(v) = sink.lock() {
        for w in v.iter() {
            if !warnings.contains(w) {
                warnings.push(w.clone());
            }
        }
    }
    Ok(ImportReport { document: doc, warnings })
}

/// Decoded RGBA pixels, width, height and hayro's scale factors.
type Decoded = (Vec<u8>, u32, u32, (f32, f32));

enum FrameKind {
    Root,
    /// A clip group whose clip path node is stored here.
    Clip(Box<Node>),
    /// A clip that was redundant (covers the page); children go straight to the parent.
    Skip,
    Group {
        opacity: f32,
        blend: BlendMode,
        masked: bool,
    },
}

struct Frame {
    kind: FrameKind,
    children: Vec<Arc<Node>>,
}

/// Glyphs drawn consecutively with the same paint, merged into one path.
struct GlyphRun {
    path: BezPath,
    paint: Paint,
    opacity: f32,
    stroke: Option<StrokeProps>,
    scale: f64,
}

struct Builder {
    next: u64,
    stack: Vec<Frame>,
    blend: BlendMode,
    page: Rect,
    glyphs: Option<GlyphRun>,
    images: HashMap<String, ImageBlob>,
    image_keys: HashMap<u128, (String, u32, u32)>,
    warnings: Vec<String>,
}

fn blend(b: hayro_interpret::BlendMode) -> BlendMode {
    use hayro_interpret::BlendMode as H;
    match b {
        H::Normal => BlendMode::Normal,
        H::Multiply => BlendMode::Multiply,
        H::Screen => BlendMode::Screen,
        H::Overlay => BlendMode::Overlay,
        H::Darken => BlendMode::Darken,
        H::Lighten => BlendMode::Lighten,
        H::ColorDodge => BlendMode::ColorDodge,
        H::ColorBurn => BlendMode::ColorBurn,
        H::HardLight => BlendMode::HardLight,
        H::SoftLight => BlendMode::SoftLight,
        H::Difference => BlendMode::Difference,
        H::Exclusion => BlendMode::Exclusion,
        H::Hue => BlendMode::Hue,
        H::Saturation => BlendMode::Saturation,
        H::Color => BlendMode::Color,
        H::Luminosity => BlendMode::Luminosity,
    }
}

fn fill_rule(r: hayro_interpret::FillRule) -> FillRule {
    match r {
        hayro_interpret::FillRule::NonZero => FillRule::NonZero,
        hayro_interpret::FillRule::EvenOdd => FillRule::EvenOdd,
    }
}

fn mean_scale(a: Affine) -> f64 {
    a.determinant().abs().sqrt()
}

fn stroke_layer(paint: Paint, opacity: f32, p: &StrokeProps, scale: f64) -> StrokeLayer {
    let mut st = StrokeLayer::new(paint, p.line_width as f64 * scale);
    st.opacity = opacity;
    st.cap = match p.line_cap {
        kurbo::Cap::Butt => LineCap::Butt,
        kurbo::Cap::Round => LineCap::Round,
        kurbo::Cap::Square => LineCap::Square,
    };
    st.join = match p.line_join {
        kurbo::Join::Miter => LineJoin::Miter,
        kurbo::Join::Round => LineJoin::Round,
        kurbo::Join::Bevel => LineJoin::Bevel,
    };
    st.miter_limit = p.miter_limit as f64;
    let dash = Dash { pattern: p.dash_array.iter().map(|v| *v as f64 * scale).collect(), offset: p.dash_offset as f64 * scale, align_corners: false };
    // An invalid dash array (a negative value, or all zeros) strokes solid.
    st.dash = dash.is_dashed().then_some(dash);
    st
}

fn round3(v: f32) -> f32 {
    (v * 1000.0).round() / 1000.0
}

impl Builder {
    fn new(next: u64) -> Self {
        Self {
            next,
            stack: vec![],
            blend: BlendMode::Normal,
            page: Rect::ZERO,
            glyphs: None,
            images: HashMap::new(),
            image_keys: HashMap::new(),
            warnings: vec![],
        }
    }

    fn id(&mut self) -> NodeId {
        let id = NodeId(self.next);
        self.next += 1;
        id
    }

    fn warn(&mut self, w: &str) {
        if !self.warnings.iter().any(|x| x == w) {
            self.warnings.push(w.to_string());
        }
    }

    fn begin_page(&mut self) {
        self.stack = vec![Frame { kind: FrameKind::Root, children: vec![] }];
        self.blend = BlendMode::Normal;
        self.glyphs = None;
    }

    fn end_page(&mut self) -> Vec<Arc<Node>> {
        self.flush_glyphs();
        while self.stack.len() > 1 {
            self.pop_frame();
        }
        self.stack.pop().map(|f| f.children).unwrap_or_default()
    }

    fn push_node(&mut self, mut n: Node) {
        if self.blend != BlendMode::Normal && !n.is_container() {
            n.blend = self.blend;
        }
        if let Some(f) = self.stack.last_mut() {
            f.children.push(Arc::new(n));
        }
    }

    fn pop_frame(&mut self) {
        if self.stack.len() <= 1 {
            return;
        }
        let Some(f) = self.stack.pop() else {
            return;
        };
        let node = match f.kind {
            FrameKind::Root => None,
            FrameKind::Skip => {
                if let Some(p) = self.stack.last_mut() {
                    p.children.extend(f.children);
                }
                None
            }
            FrameKind::Clip(clip) => {
                if f.children.is_empty() {
                    None
                } else {
                    let mut ch = vec![Arc::new(*clip)];
                    ch.extend(f.children);
                    Some(Node::new(self.id(), NodeKind::Group { children: ch, clip: true }))
                }
            }
            FrameKind::Group { opacity, blend, masked } => {
                if f.children.is_empty() {
                    None
                } else if opacity >= 0.999 && blend == BlendMode::Normal && !masked {
                    if let Some(p) = self.stack.last_mut() {
                        p.children.extend(f.children);
                    }
                    None
                } else if f.children.len() == 1 && !f.children[0].is_container() && !masked {
                    // A single object in a group: fold opacity/blend into the object itself.
                    let mut only = (*f.children[0]).clone();
                    only.opacity *= opacity;
                    if blend != BlendMode::Normal {
                        only.blend = blend;
                    }
                    Some(only)
                } else {
                    let mut g = Node::group(self.id(), f.children);
                    g.opacity = opacity;
                    g.blend = blend;
                    Some(g)
                }
            }
        };
        if let Some(n) = node
            && let Some(p) = self.stack.last_mut()
        {
            p.children.push(Arc::new(n));
        }
    }

    fn flush_glyphs(&mut self) {
        let Some(run) = self.glyphs.take() else { return };
        if run.path.elements().is_empty() {
            return;
        }
        let mut ap = Appearance::default();
        match &run.stroke {
            None => {
                let mut f = FillLayer::new(run.paint);
                f.opacity = run.opacity;
                ap.items.push(AppearanceItem::Fill(f));
            }
            Some(p) => ap.items.push(AppearanceItem::Stroke(stroke_layer(run.paint, run.opacity, p, run.scale))),
        }
        let id = self.id();
        let mut n = Node::path(id, PathData::from_bezpath(&run.path), ap);
        n.name = Some("<Text Outlines>".into());
        self.push_node(n);
    }

    /// Convert a hayro paint to a VectorCraft paint and opacity. `bbox` is the painted area in
    /// document coordinates (for patterns).
    fn paint(&mut self, p: &hayro_interpret::Paint<'_>) -> (Paint, f32) {
        match p {
            hayro_interpret::Paint::Color(c) => {
                let [r, g, b, a] = c.to_rgba().components();
                (Paint::solid(Color::rgb(round3(r), round3(g), round3(b))), a)
            }
            hayro_interpret::Paint::Pattern(pat) => match pat.as_ref() {
                Pattern::Shading(sp) => match shading_gradient(sp) {
                    Some(g) => (Paint::Gradient(Box::new(g)), sp.opacity),
                    None => {
                        self.warn("mesh/function shadings are imported as a flat colour");
                        (Paint::solid(Color::rgb(0.5, 0.5, 0.5)), sp.opacity)
                    }
                },
                Pattern::Tiling(_) => {
                    self.warn("tiling patterns are imported as a flat grey");
                    (Paint::solid(Color::rgb(0.5, 0.5, 0.5)), 1.0)
                }
            },
        }
    }

    fn add_image(&mut self, key: u128, make: impl FnOnce() -> Option<(ImageBlob, u32, u32)>, xf: Affine) {
        let (k, w, h) = if let Some(v) = self.image_keys.get(&key).cloned() {
            v
        } else {
            let Some((blob, w, h)) = make() else {
                self.warn("an image could not be decoded and was skipped");
                return;
            };
            let k = format!("pdf-image-{}", self.image_keys.len() + 1);
            self.images.insert(k.clone(), blob);
            self.image_keys.insert(key, (k.clone(), w, h));
            (k, w, h)
        };
        let id = self.id();
        self.push_node(Node::new(id, NodeKind::Image(ImageObject { key: k, width: w, height: h, xf, link: None })));
    }
}

/// Axial/radial shading → gradient paint (stops sampled from the shading function).
fn shading_gradient(sp: &hayro_interpret::pattern::ShadingPattern) -> Option<GradientPaint> {
    let ShadingType::RadialAxial { coords, domain, function, axial, .. } = sp.shading.shading_type.as_ref() else {
        return None;
    };
    let m = sp.matrix;
    let cs = &sp.shading.color_space;
    let sample = |t: f32| -> Option<(Color, f32)> {
        let x = domain[0] + (domain[1] - domain[0]) * t;
        let v = function.eval(&smallvec::smallvec![x])?;
        let [r, g, b, a] = cs.to_rgba(&v, 1.0, false).components();
        Some((Color::rgb(round3(r), round3(g), round3(b)), a))
    };
    // Sample densely, then drop samples that linear interpolation reproduces.
    const N: usize = 64;
    let mut pts: Vec<(f32, [f32; 3])> = Vec::with_capacity(N + 1);
    for i in 0..=N {
        let t = i as f32 / N as f32;
        let (c, _) = sample(t)?;
        pts.push((t, c.to_rgb()));
    }
    let mut keep = vec![0usize];
    for i in 1..N {
        let Some(&a) = keep.last().and_then(|&j| pts.get(j)) else {
            break;
        };
        let b = pts[i + 1];
        let u = (pts[i].0 - a.0) / (b.0 - a.0).max(1e-6);
        let off = (0..3).map(|k| (a.1[k] + (b.1[k] - a.1[k]) * u - pts[i].1[k]).abs()).fold(0.0f32, f32::max);
        if off > 1.5 / 255.0 {
            keep.push(i);
        }
    }
    keep.push(N);
    let stop_at = |t: f32, offset: f32| -> GradientStop {
        let (color, _) = sample(t).unwrap_or((Color::BLACK, 1.0));
        GradientStop { offset, color, opacity: 1.0, midpoint: 0.5 }
    };
    let c = |x: f32, y: f32| m * Point::new(x as f64, y as f64);
    let (kind, geom, stops) = if *axial {
        let geom = GradientGeom { start: c(coords[0], coords[1]), end: c(coords[2], coords[3]), aspect: 1.0 };
        (GradientKind::Linear, geom, keep.iter().map(|&i| stop_at(pts[i].0, pts[i].0)).collect::<Vec<_>>())
    } else {
        let (r0, r1) = (coords[2].max(0.0), coords[5].max(1e-6));
        let centre = c(coords[3], coords[4]);
        let k = m.as_coeffs();
        let ex = Vec2::new(k[0], k[1]) * r1 as f64;
        let ey = Vec2::new(k[2], k[3]) * r1 as f64;
        let aspect = if ex.hypot() > 1e-9 { ey.hypot() / ex.hypot() } else { 1.0 };
        let geom = GradientGeom { start: centre, end: centre + ex, aspect };
        // Offsets are relative to the outer radius; an inner radius shifts them outwards.
        let stops = keep.iter().map(|&i| stop_at(pts[i].0, (r0 + (r1 - r0) * pts[i].0) / r1)).collect();
        (GradientKind::Radial, geom, stops)
    };
    let mut g = GradientPaint::new(Gradient { kind, stops });
    g.geom = Some(geom);
    Some(g)
}

fn rgba_png(rgba: Vec<u8>, w: u32, h: u32) -> Option<Vec<u8>> {
    let img = image::RgbaImage::from_raw(w, h, rgba)?;
    let mut out = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).ok()?;
    Some(out)
}

fn resize_alpha(a: &LumaData, w: u32, h: u32) -> Vec<u8> {
    if a.width == w && a.height == h {
        return a.data.clone();
    }
    match image::GrayImage::from_raw(a.width, a.height, a.data.clone()) {
        Some(g) => image::imageops::resize(&g, w, h, image::imageops::FilterType::Triangle).into_raw(),
        None => vec![255; (w as usize).saturating_mul(h as usize)],
    }
}

impl<'a> Device<'a> for Builder {
    fn set_soft_mask(&mut self, mask: Option<SoftMask<'a>>) {
        if mask.is_some() {
            self.warn("soft masks are not imported (content drawn unmasked)");
        }
    }

    fn set_blend_mode(&mut self, blend_mode: hayro_interpret::BlendMode) {
        self.blend = blend(blend_mode);
    }

    fn draw_path(&mut self, path: &BezPath, transform: Affine, paint: &hayro_interpret::Paint<'a>, draw_mode: &PathDrawMode) {
        self.flush_glyphs();
        let mut bp = path.clone();
        bp.apply_affine(transform);
        if bp.elements().is_empty() {
            return;
        }
        let pd = PathData::from_bezpath(&bp);
        let (paint, opacity) = self.paint(paint);
        match draw_mode {
            PathDrawMode::Fill(rule) => {
                let mut f = FillLayer::new(paint);
                f.opacity = opacity;
                let mut n = Node::path(self.id(), pd, Appearance { items: vec![AppearanceItem::Fill(f)], effects: vec![] });
                if let NodeKind::Path { rule: r, .. } = &mut n.kind {
                    *r = fill_rule(*rule);
                }
                self.push_node(n);
            }
            PathDrawMode::Stroke(props) => {
                let st = stroke_layer(paint, opacity, props, mean_scale(transform));
                // Fill-then-stroke of the same path (the `B` operator) becomes one object.
                let blend = self.blend;
                if let Some(f) = self.stack.last_mut()
                    && let Some(last) = f.children.last_mut()
                    && last.blend == blend
                    && last.path_data() == Some(&pd)
                    && last.appearance.stroke().is_none()
                {
                    Arc::make_mut(last).appearance.items.push(AppearanceItem::Stroke(st));
                    return;
                }
                let n = Node::path(self.id(), pd, Appearance { items: vec![AppearanceItem::Stroke(st)], effects: vec![] });
                self.push_node(n);
            }
        }
    }

    fn push_clip_path(&mut self, clip_path: &ClipPath) {
        self.flush_glyphs();
        let bb = clip_path.path.bounding_box();
        // Clips that contain the whole page do nothing visible; don't create groups for them.
        let redundant = clip_path.path.elements().len() <= 6
            && bb.x0 <= self.page.x0 + 0.01
            && bb.y0 <= self.page.y0 + 0.01
            && bb.x1 >= self.page.x1 - 0.01
            && bb.y1 >= self.page.y1 - 0.01
            && self.page.area() > 0.0;
        let kind = if redundant {
            FrameKind::Skip
        } else {
            let mut clip = Node::new(
                self.id(),
                NodeKind::Path {
                    path: PathData::from_bezpath(&clip_path.path),
                    rule: fill_rule(clip_path.fill),
                    live: None,
                    clipping: true,
                    guide: false,
                },
            );
            clip.name = Some("<Clipping Path>".into());
            FrameKind::Clip(Box::new(clip))
        };
        self.stack.push(Frame { kind, children: vec![] });
    }

    fn push_transparency_group(&mut self, opacity: f32, mask: Option<SoftMask<'a>>, blend_mode: hayro_interpret::BlendMode) {
        self.flush_glyphs();
        let masked = mask.is_some();
        if masked {
            self.warn("soft masks are not imported (content drawn unmasked)");
        }
        self.stack.push(Frame { kind: FrameKind::Group { opacity, blend: blend(blend_mode), masked }, children: vec![] });
        // Blend mode applies to the group as a whole, not to its children.
        self.blend = BlendMode::Normal;
    }

    fn draw_glyph(
        &mut self,
        glyph: &Glyph<'a>,
        transform: Affine,
        glyph_transform: Affine,
        paint: &hayro_interpret::Paint<'a>,
        draw_mode: &GlyphDrawMode,
    ) {
        let stroke = match draw_mode {
            GlyphDrawMode::Invisible => return,
            GlyphDrawMode::Fill => None,
            GlyphDrawMode::Stroke(p) => Some(p.clone()),
        };
        match glyph {
            Glyph::Outline(o) => {
                let mut bp = o.outline();
                bp.apply_affine(transform * glyph_transform);
                let (paint, opacity) = self.paint(paint);
                let scale = mean_scale(transform);
                let same = self.glyphs.as_ref().is_some_and(|r| {
                    r.paint == paint
                        && r.opacity == opacity
                        && r.stroke.as_ref().map(|s| s.line_width) == stroke.as_ref().map(|s| s.line_width)
                        && (r.scale - scale).abs() < 1e-9
                });
                if !same {
                    self.flush_glyphs();
                    self.glyphs = Some(GlyphRun { path: BezPath::new(), paint, opacity, stroke, scale });
                }
                if let Some(r) = &mut self.glyphs {
                    r.path.extend(bp.iter());
                }
                self.warn("text was converted to outlines");
            }
            Glyph::Type3(t3) => {
                self.flush_glyphs();
                t3.interpret(self, transform, glyph_transform, paint);
            }
        }
    }

    fn draw_image(&mut self, image: Image<'a, '_>, transform: Affine) {
        self.flush_glyphs();
        match image {
            Image::Raster(r) => {
                let key = hayro_interpret::CacheKey::cache_key(&r);
                // JPEG passthrough for plain DeviceRGB/DeviceGray DCT images.
                let st = r.stream();
                let dict = st.dict();
                let filters = st.filters();
                let cs_ok = dict.get::<Name<'_>>(b"ColorSpace").is_some_and(|n| matches!(n.as_ref(), b"DeviceRGB" | b"DeviceGray"));
                let jpeg = filters.len() == 1
                    && matches!(filters[0], hayro_syntax::Filter::DctDecode)
                    && cs_ok
                    && !dict.contains_key(b"SMask")
                    && !dict.contains_key(b"Mask")
                    && !dict.contains_key(b"Decode");
                if jpeg {
                    let (w, h) = (r.width(), r.height());
                    let bytes = st.raw_data().to_vec();
                    self.add_image(key, || Some((ImageBlob { mime: "image/jpeg".into(), bytes: Arc::new(bytes) }, w, h)), transform);
                    return;
                }
                let mut decoded: Option<Decoded> = None;
                r.with_rgba(
                    |img, alpha| {
                        let (w, h, sf) = (img.width(), img.height(), img.scale_factors());
                        let a = alpha.map(|a| resize_alpha(&a, w, h)).unwrap_or_else(|| vec![255; (w as usize).saturating_mul(h as usize)]);
                        let rgba: Vec<u8> = match img {
                            ImageData::Rgb(d) => d.data.chunks_exact(3).zip(a).flat_map(|(c, a)| [c[0], c[1], c[2], a]).collect(),
                            ImageData::Luma(d) => d.data.iter().zip(a).flat_map(|(g, a)| [*g, *g, *g, a]).collect(),
                        };
                        decoded = Some((rgba, w, h, sf));
                    },
                    None,
                );
                let Some((rgba, w, h, sf)) = decoded else {
                    self.warn("an image could not be decoded and was skipped");
                    return;
                };
                let xf = transform * Affine::scale_non_uniform(sf.0 as f64, sf.1 as f64);
                self.add_image(key, || rgba_png(rgba, w, h).map(|png| (ImageBlob { mime: "image/png".into(), bytes: Arc::new(png) }, w, h)), xf);
            }
            Image::Stencil(s) => {
                let key = hayro_interpret::CacheKey::cache_key(&s);
                let mut decoded: Option<Decoded> = None;
                s.with_stencil(
                    |luma, paint| {
                        let rgb = match paint {
                            hayro_interpret::Paint::Color(c) => c.to_rgba().to_rgba8(),
                            _ => [0, 0, 0, 255],
                        };
                        let rgba = luma.data.iter().flat_map(|a| [rgb[0], rgb[1], rgb[2], *a]).collect();
                        decoded = Some((rgba, luma.width, luma.height, luma.scale_factors));
                    },
                    None,
                );
                let Some((rgba, w, h, sf)) = decoded else { return };
                let xf = transform * Affine::scale_non_uniform(sf.0 as f64, sf.1 as f64);
                self.add_image(
                    key ^ 0x5_7e9c,
                    || rgba_png(rgba, w, h).map(|png| (ImageBlob { mime: "image/png".into(), bytes: Arc::new(png) }, w, h)),
                    xf,
                );
            }
        }
    }

    fn pop_clip_path(&mut self) {
        self.flush_glyphs();
        self.pop_frame();
    }

    fn pop_transparency_group(&mut self) {
        self.flush_glyphs();
        self.pop_frame();
    }
}
