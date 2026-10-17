//! Affinity documents (`.af`, `.afdesign`, `.afphoto`, `.afpub`): the native layers, curves,
//! shapes, artboards, text and images that `vectorcraft-affinity` reads, mapped to editable
//! VectorCraft art. Whatever the reader could not map is listed in the warnings. A file whose
//! native document can't be read opens as its embedded preview instead, with a warning that says
//! why, and can't be placed.

use std::io::Cursor;
use std::sync::Arc;

use kurbo::{Affine, BezPath, Point, Rect, Vec2};
use vectorcraft_affinity::model::{self as af, Align as AfAlign, Blend, Kind, Pixels};
use vectorcraft_affinity::paint::{self as ap, Cap, GradientKind as AfKind, Join};
use vectorcraft_color::{BlendMode, Color, Gradient, GradientGeom, GradientKind, GradientPaint, GradientStop, Paint};
use vectorcraft_doc::appearance::{Appearance, AppearanceItem, Dash, FillLayer, LineCap, LineJoin, StrokeAlign, StrokeLayer};
use vectorcraft_doc::geom::{FillRule, PathData, shapes};
use vectorcraft_doc::text::{CharStyle, Justify, TextKind, TextObject, TextRun};
use vectorcraft_doc::{Artboard, Document, ImageObject, LayerColor, Node, NodeKind, OpacityMask};

use super::load::{err, raster_doc};
use crate::Result;

/// What an Affinity file opened as.
pub(super) struct Imported {
    pub doc: Document,
    pub warnings: Vec<String>,
    /// Only the embedded preview: never placed, used as a template or mined for styles.
    pub preview_only: bool,
}

pub(super) fn import(name: &str, bytes: &[u8]) -> Result<Imported> {
    match vectorcraft_affinity::read(bytes, vectorcraft_affinity::Limits::default()) {
        Ok(d) => {
            let mut b = Builder { doc: Document::new(0.0, 0.0), warnings: Vec::new(), k: 72.0 / d.dpi, layer: 0 };
            b.document(&d)?;
            b.doc.limit_depth_noting(&mut b.warnings);
            let mut warnings = d.warnings.iter().map(|w| format!("Affinity: not imported or approximated: {w}.")).collect::<Vec<_>>();
            warnings.extend(b.warnings.iter().map(|w| format!("Affinity: {w}.")));
            Ok(Imported { doc: b.doc, warnings, preview_only: false })
        }
        Err(native) => {
            let (doc, w) = preview(name, bytes, &native)?;
            Ok(Imported { doc, warnings: vec![w], preview_only: true })
        }
    }
}

/// The embedded preview, for files whose native document can't be read.
fn preview(name: &str, bytes: &[u8], native: &vectorcraft_affinity::Error) -> Result<(Document, String)> {
    let p = vectorcraft_affinity::preview(bytes).map_err(|e| err(format!("{native}; its embedded preview can't be read either ({e})")))?;
    // `raster_doc` normally probes PNG dimensions and keeps its encoded bytes. Verify this
    // untrusted preview's complete pixels first, so corrupt data cannot create a blank image.
    let mut reader = image::ImageReader::with_format(Cursor::new(p.png), image::ImageFormat::Png);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);
    limits.max_alloc = Some(128 << 20);
    reader.limits(limits);
    let _pixels = reader.decode().map_err(err)?;
    let d = raster_doc(name, p.png)?;
    Ok((
        d,
        format!(
            "The native Affinity document could not be read ({native}). Opened only its embedded {}×{} PNG preview, which may be smaller than the document: no layers, vectors, text or pages. Save a new copy; the Affinity source cannot be saved back. Export SVG or PDF from Affinity for editable vectors.",
            p.width, p.height
        ),
    ))
}

struct Builder {
    doc: Document,
    warnings: Vec<String>,
    /// Points per document pixel.
    k: f64,
    /// Layers made so far (their colours cycle).
    layer: u8,
}

fn rect_of(r: af::Rect) -> Rect {
    Rect::new(r.x0, r.y0, r.x1, r.y1)
}

fn affine(a: af::Affine) -> Affine {
    Affine::new(a.0)
}

impl Builder {
    fn warn(&mut self, w: &str) {
        if !self.warnings.iter().any(|x| x == w) {
            self.warnings.push(w.to_string());
        }
    }

    fn document(&mut self, d: &af::Document) -> Result<()> {
        // Spreads (Publisher pages) sit side by side, 36 pt apart; a single spread keeps its origin.
        let mut layers = Vec::new();
        let mut artboards = Vec::new();
        let mut x = 0.0;
        for (i, spread) in d.spreads.iter().enumerate() {
            let shift = if i == 0 { 0.0 } else { x - spread.bounds.x0 * self.k };
            let to_doc = Affine::translate((shift, 0.0)) * Affine::scale(self.k);
            let before = artboards.len();
            let mut loose: Vec<Arc<Node>> = Vec::new();
            for n in &spread.nodes {
                let top = match &n.kind {
                    Kind::Layer => self.node(n, to_doc, &mut artboards, true),
                    Kind::Artboard { .. } => self.node(n, to_doc, &mut artboards, true).map(|art| self.layer_of(&n.name, art, n.visible, n.locked)),
                    _ => {
                        if let Some(art) = self.node(n, to_doc, &mut artboards, false) {
                            loose.push(art);
                        }
                        continue;
                    }
                };
                if !loose.is_empty() {
                    layers.push(self.layer_with(None, std::mem::take(&mut loose)));
                }
                layers.extend(top);
            }
            if !loose.is_empty() {
                layers.push(self.layer_with(None, loose));
            }
            // Without artboards, each page (or the spread) becomes one.
            if artboards.len() == before {
                let rects = if spread.pages.is_empty() { vec![spread.bounds] } else { spread.pages.clone() };
                for r in rects {
                    let name = format!("{} {}", if d.spreads.len() > 1 || spread.pages.len() > 1 { "Page" } else { "Artboard" }, artboards.len() + 1);
                    artboards.push((name, to_doc.transform_rect_bbox(rect_of(r))));
                }
            }
            x = artboards.iter().map(|(_, r)| r.x1).fold(x, f64::max) + 36.0;
            if spread.transparent && i == 0 {
                self.warn("transparent page backgrounds show as white artboards");
            }
        }
        self.doc.artboards = artboards
            .into_iter()
            .enumerate()
            .filter(|(_, (_, r))| r.width() > 0.0 && r.height() > 0.0)
            .map(|(i, (name, rect))| Artboard { id: i as u32 + 1, name, rect, show_center_mark: false, show_cross_hairs: false, color: None })
            .collect();
        if self.doc.artboards.is_empty() {
            return Err(err("the Affinity document has no page with a size"));
        }
        // Bottom to top, as Affinity lists them.
        self.doc.layers = layers;
        if self.doc.layers.is_empty() {
            let empty = self.layer_with(None, Vec::new());
            self.doc.layers.push(empty);
        }
        Ok(())
    }

    fn layer_with(&mut self, name: Option<&str>, children: Vec<Arc<Node>>) -> Arc<Node> {
        let id = self.doc.alloc_id();
        self.layer = self.layer.wrapping_add(1);
        let label = name.map(str::to_string).unwrap_or_else(|| format!("Layer {}", self.layer));
        let mut n = Node::layer(id, &label, LayerColor::Preset(self.layer % 9));
        if let Some(c) = n.children_mut() {
            *c = children;
        }
        Arc::new(n)
    }

    /// An artboard at the top level becomes a layer of its name holding its clip group.
    fn layer_of(&mut self, name: &str, art: Arc<Node>, visible: bool, locked: bool) -> Arc<Node> {
        let mut l = (*self.layer_with(Some(if name.is_empty() { "Artboard" } else { name }), vec![art])).clone();
        l.visible = visible;
        l.locked = locked;
        Arc::new(l)
    }

    fn base(&mut self, n: &af::Node, kind: NodeKind) -> Node {
        let mut out = Node::new(self.doc.alloc_id(), kind);
        out.name = (!n.name.is_empty()).then(|| n.name.clone());
        out.visible = n.visible;
        out.locked = n.locked;
        out.opacity = n.opacity as f32;
        out.blend = self.blend(n.blend);
        out
    }

    fn blend(&mut self, b: Blend) -> BlendMode {
        match b {
            Blend::Normal | Blend::PassThrough => BlendMode::Normal,
            Blend::Darken => BlendMode::Darken,
            Blend::Multiply => BlendMode::Multiply,
            Blend::ColorBurn => BlendMode::ColorBurn,
            Blend::Lighten => BlendMode::Lighten,
            Blend::Screen => BlendMode::Screen,
            Blend::ColorDodge => BlendMode::ColorDodge,
            Blend::Overlay => BlendMode::Overlay,
            Blend::SoftLight => BlendMode::SoftLight,
            Blend::HardLight => BlendMode::HardLight,
            Blend::Difference => BlendMode::Difference,
            Blend::Exclusion => BlendMode::Exclusion,
            Blend::Hue => BlendMode::Hue,
            Blend::Saturation => BlendMode::Saturation,
            Blend::Color => BlendMode::Color,
            Blend::Luminosity => BlendMode::Luminosity,
            _ => {
                self.warn("blend modes VectorCraft doesn't have (Add, Linear Light, Vivid Light, Pin Light, Hard Mix, Subtract, Darker/Lighter Colour, Average, Negation, Reflect, Glow, Erase) import as Normal");
                BlendMode::Normal
            }
        }
    }

    /// One Affinity node (and its subtree) as VectorCraft art; `to_doc` maps document pixels to
    /// points. `layer_level`: the node is directly in a spread or a layer, so a layer stays a layer.
    fn node(&mut self, n: &af::Node, to_doc: Affine, boards: &mut Vec<(String, Rect)>, layer_level: bool) -> Option<Arc<Node>> {
        // Register before descending so nested artboards keep their document order.
        if let Kind::Artboard { rect, .. } = &n.kind {
            boards.push((
                if n.name.is_empty() { format!("Artboard {}", boards.len() + 1) } else { n.name.clone() },
                to_doc.transform_rect_bbox(rect_of(*rect)),
            ));
        }
        let child_level = matches!(n.kind, Kind::Layer) && layer_level;
        let children: Vec<Arc<Node>> = n.children.iter().filter_map(|c| self.node(c, to_doc, boards, child_level)).collect();
        let mut out = match &n.kind {
            Kind::Layer if layer_level => {
                let mut l = (*self.layer_with(Some(if n.name.is_empty() { "Layer" } else { &n.name }), children)).clone();
                l.visible = n.visible;
                l.locked = n.locked;
                l.opacity = n.opacity as f32;
                l.blend = self.blend(n.blend);
                l
            }
            Kind::Layer | Kind::Group => {
                let mut g = self.base(n, NodeKind::Group { children, clip: false });
                // Affinity groups isolate their blending unless they pass through.
                g.isolate = n.blend != Blend::PassThrough;
                g
            }
            Kind::Shape { path, fills, strokes, even_odd } => {
                let data = self.path(path, to_doc);
                let rule = if *even_odd { FillRule::EvenOdd } else { FillRule::NonZero };
                let appearance = self.appearance(fills, strokes, to_doc);
                if children.is_empty() {
                    let mut p = self.base(n, NodeKind::Path { path: data, rule, live: None, clipping: false, guide: false });
                    p.appearance = appearance.into();
                    p
                } else {
                    // A shape clips its children: VectorCraft's clipping path keeps its paint, its
                    // fill behind the clipped art and its stroke over it, as Affinity draws it.
                    let mut clip = Node::new(self.doc.alloc_id(), NodeKind::Path { path: data, rule, live: None, clipping: true, guide: false });
                    clip.appearance = appearance.into();
                    let mut kids = vec![Arc::new(clip)];
                    kids.extend(children);
                    self.base(n, NodeKind::Group { children: kids, clip: true })
                }
            }
            Kind::Artboard { path, even_odd, background, strokes, .. } => {
                let mut clip = Node::new(
                    self.doc.alloc_id(),
                    NodeKind::Path {
                        path: self.path(path, to_doc),
                        rule: if *even_odd { FillRule::EvenOdd } else { FillRule::NonZero },
                        live: None,
                        clipping: true,
                        guide: false,
                    },
                );
                clip.appearance = self.appearance(background, strokes, to_doc).into();
                let mut kids = vec![Arc::new(clip)];
                kids.extend(children);
                let mut g = self.base(n, NodeKind::Group { children: kids, clip: true });
                g.name = Some(if n.name.is_empty() { "Artboard".into() } else { n.name.clone() });
                g
            }
            Kind::Text(t) => {
                let text = self.text(t, to_doc);
                let mut node = self.base(n, NodeKind::Text(Box::new(text)));
                if !children.is_empty() {
                    let mut kids = vec![Arc::new(node)];
                    kids.extend(children);
                    node = Node::group(self.doc.alloc_id(), kids);
                }
                node
            }
            Kind::Image(img) => match self.image(img, to_doc) {
                Some(kind) => self.base(n, kind),
                None if children.is_empty() => return None,
                None => self.base(n, NodeKind::Group { children, clip: false }),
            },
            Kind::Unsupported if children.is_empty() => return None,
            Kind::Unsupported => self.base(n, NodeKind::Group { children, clip: false }),
        };
        if let Some(img) = &n.pixel_mask {
            // A pixel mask: its grey levels show (white) or hide (black) the node, and nothing
            // shows outside it, as an opacity mask that clips.
            if let Some(kind) = self.image(img, to_doc) {
                let art = Node::new(self.doc.alloc_id(), kind);
                out.extras_mut().mask = Some(Box::new(OpacityMask::new(art, true)));
            }
        }
        if let Some(mask) = &n.mask {
            // A vector mask: the node shows only inside it.
            let clip = Node::new(
                self.doc.alloc_id(),
                NodeKind::Path { path: self.path(mask, to_doc), rule: FillRule::NonZero, live: None, clipping: true, guide: false },
            );
            let (opacity, blend, visible, name) = (out.opacity, out.blend, out.visible, out.name.clone());
            out.opacity = 1.0;
            out.blend = BlendMode::Normal;
            let mut g = Node::new(self.doc.alloc_id(), NodeKind::Group { children: vec![Arc::new(clip), Arc::new(out)], clip: true });
            (g.opacity, g.blend, g.visible, g.name) = (opacity, blend, visible, name);
            out = g;
        }
        Some(Arc::new(out))
    }

    fn path(&self, p: &af::Path, to_doc: Affine) -> PathData {
        let mut bp = BezPath::new();
        for s in &p.subpaths {
            let pt = |q: af::Point| Point::new(q.x, q.y);
            bp.move_to(pt(s.start));
            for [c1, c2, e] in &s.segments {
                if c1 == c2 && c2 == e {
                    bp.line_to(pt(*e));
                } else {
                    bp.curve_to(pt(*c1), pt(*c2), pt(*e));
                }
            }
            if s.closed {
                bp.close_path();
            }
        }
        PathData::from_bezpath(&(to_doc * bp))
    }

    fn color(c: ap::Color) -> (Color, f32) {
        let f = |v: f64| v.clamp(0.0, 1.0) as f32;
        match c {
            ap::Color::Rgb { r, g, b, a } => (Color::Rgb { r: f(r), g: f(g), b: f(b) }, f(a)),
            ap::Color::Cmyk { c, m, y, k, a } => (Color::Cmyk { c: f(c), m: f(m), y: f(y), k: f(k) }, f(a)),
            ap::Color::Gray { v, a } => (Color::Gray { k: 1.0 - f(v) }, f(a)),
            ap::Color::Lab { l, a, b, alpha } => (Color::Lab { l: l as f32, a: a as f32, b: b as f32 }, f(alpha)),
        }
    }

    /// A paint and the opacity its colour's alpha becomes.
    fn paint(&mut self, p: &ap::Paint, to_doc: Affine) -> Option<(Paint, f32)> {
        match p {
            ap::Paint::None => None,
            ap::Paint::Solid(c) => {
                let (color, a) = Self::color(*c);
                Some((Paint::solid(color), a))
            }
            ap::Paint::Gradient(g) => {
                let stops = g
                    .stops
                    .iter()
                    .map(|s| {
                        let (color, opacity) = Self::color(s.color);
                        let mut stop = GradientStop::new(s.offset as f32, color);
                        stop.opacity = opacity;
                        stop.midpoint = s.half_point().clamp(0.13, 0.87) as f32;
                        stop
                    })
                    .collect();
                let m = to_doc * affine(g.transform);
                let (o, x, y) = (m * Point::ZERO, m * Point::new(1.0, 0.0), m * Point::new(0.0, 1.0));
                let kind = match g.kind {
                    AfKind::Linear => GradientKind::Linear,
                    AfKind::Radial => GradientKind::Radial,
                    AfKind::Conical => {
                        self.warn("conical gradients import as radial gradients");
                        GradientKind::Radial
                    }
                };
                let (u, v): (Vec2, Vec2) = (x - o, y - o);
                if kind == GradientKind::Radial && u.hypot() > 0.0 && (u.dot(v) / (u.hypot() * v.hypot().max(1e-12))).abs() > 0.01 {
                    self.warn("skewed elliptical gradients import as upright ellipses");
                }
                let aspect = if u.hypot() > 0.0 { (v.hypot() / u.hypot()).clamp(0.01, 100.0) } else { 1.0 };
                let geom = GradientGeom { start: o, end: x, aspect: if kind == GradientKind::Radial { aspect } else { 1.0 }, focal: None };
                let mut gp = GradientPaint::new(Gradient { kind, stops });
                gp.angle = (-(u.y)).atan2(u.x).to_degrees();
                gp.geom = Some(geom);
                Some((Paint::Gradient(Box::new(gp)), 1.0))
            }
        }
    }

    fn appearance(&mut self, fills: &[ap::Paint], strokes: &[ap::Stroke], to_doc: Affine) -> Appearance {
        let mut items = Vec::new();
        let mut above = Vec::new();
        for f in fills {
            if let Some((paint, opacity)) = self.paint(f, to_doc) {
                let mut l = FillLayer::new(paint);
                l.opacity = opacity;
                items.push(AppearanceItem::Fill(l));
            }
        }
        for s in strokes {
            let Some((paint, opacity)) = self.paint(&s.paint, to_doc) else { continue };
            let mut l = StrokeLayer::new(paint, s.width * self.k);
            l.opacity = opacity;
            l.cap = match s.cap {
                Cap::Butt => LineCap::Butt,
                Cap::Round => LineCap::Round,
                Cap::Square => LineCap::Square,
            };
            l.join = match s.join {
                Join::Miter => LineJoin::Miter,
                Join::Round => LineJoin::Round,
                Join::Bevel => LineJoin::Bevel,
            };
            l.miter_limit = s.miter_limit;
            l.align = match s.align {
                ap::Align::Center => StrokeAlign::Center,
                ap::Align::Inside => StrokeAlign::Inside,
                ap::Align::Outside => StrokeAlign::Outside,
            };
            l.dash = s.dash.as_ref().map(|(pattern, phase)| Dash {
                pattern: pattern.iter().map(|v| v * self.k).collect(),
                offset: phase * self.k,
                align_corners: false,
            });
            if s.behind {
                items.insert(0, AppearanceItem::Stroke(l));
            } else {
                above.push(AppearanceItem::Stroke(l));
            }
        }
        items.extend(above);
        Appearance { items, ..Appearance::default() }
    }

    fn text(&mut self, t: &af::Text, to_doc: Affine) -> TextObject {
        let origin = match t.frame {
            Some(f) => Point::new(f.x0, f.y0),
            None => Point::new(t.anchor.x, t.anchor.y),
        };
        // Text space: origin at the anchor (or frame corner), the transform's uniform scale moved
        // into the type size so sizes read in points.
        let m = to_doc * affine(t.transform) * Affine::translate(origin.to_vec2());
        let c = m.as_coeffs();
        let k = (c[0] * c[3] - c[1] * c[2]).abs().sqrt().max(1e-9);
        let xf = m * Affine::scale(1.0 / k);
        let mut obj = TextObject::point(Point::ZERO, "", CharStyle::default());
        obj.xf = xf;
        obj.runs = t
            .runs
            .iter()
            .map(|r| {
                let mut style = CharStyle::default();
                if !r.family.is_empty() {
                    style.font_family = r.family.clone();
                } else if !r.postscript.is_empty() {
                    style.font_family = r.postscript.clone();
                } else {
                    self.warn("text without a font name uses the default font");
                }
                // A family/weight pair cannot identify width variants such as Arial Narrow.
                // Resolve a known PostScript identity to the exact installed family and style.
                if let Some((family, face_style)) = vectorcraft_text::FontDb::global().by_postscript_name(&r.postscript) {
                    style.font_family = family;
                    style.font_style = face_style;
                } else {
                    style.font_style = font_style(r.weight, r.italic);
                }
                style.features = r.features.clone();
                if r.features.iter().any(|feature| !vectorcraft_text::OtFeatures::known_tag(feature)) {
                    self.warn("unsupported text OpenType features (retained but not rendered)");
                }
                style.all_caps = r.all_caps;
                style.size = r.size * k;
                style.leading = r.leading.map(|l| l * k);
                style.tracking = r.tracking * 1000.0;
                style.fill = match self.paint(&r.fill, to_doc) {
                    Some((p, a)) => {
                        if a < 0.999 {
                            self.warn("translucent text colours import opaque");
                        }
                        p
                    }
                    None => Paint::None,
                };
                // Affinity separates paragraphs with U+2029 and lines with U+2028.
                TextRun::new(r.text.replace(['\u{2029}', '\u{2028}'], "\n"), style)
            })
            .collect();
        obj.para.justify = match t.align {
            AfAlign::Left => Justify::Left,
            AfAlign::Center => Justify::Center,
            AfAlign::Right => Justify::Right,
            AfAlign::Justify => Justify::JustifyLeft,
        };
        if let Some(f) = t.frame {
            obj.kind = TextKind::Area { frame: shapes::rectangle(Rect::new(0.0, 0.0, (f.x1 - f.x0) * k, (f.y1 - f.y0) * k)) };
            // Affinity shows a line in a frame shorter than it (#1423); here none of it would draw.
            if vectorcraft_text::fit_first_line(vectorcraft_text::FontDb::global(), &mut obj) {
                self.warn("text frames too short for their first line (made taller so the line shows)");
            }
        }
        obj
    }

    fn image(&mut self, img: &af::Image, to_doc: Affine) -> Option<NodeKind> {
        let bytes = match &img.pixels {
            Pixels::Encoded(b) => b.clone(),
            Pixels::Rgba8(rgba) => {
                let buf = image::RgbaImage::from_raw(img.width, img.height, rgba.clone())?;
                let mut png = Vec::new();
                buf.write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png).ok()?;
                png
            }
        };
        let r = match super::load::raster_image(&bytes) {
            Ok(r) => r,
            Err(_) => {
                self.warn("an embedded image that could not be decoded was left out");
                return None;
            }
        };
        // The pixel grid maps onto the Affinity image's own size, whatever the stored file's.
        let sx = img.width as f64 / f64::from(r.width.max(1));
        let sy = img.height as f64 / f64::from(r.height.max(1));
        let xf = to_doc * affine(img.transform) * Affine::scale_non_uniform(sx, sy);
        self.doc.images.entry(r.key.clone()).or_insert(r.blob);
        Some(NodeKind::Image(Box::new(ImageObject { key: r.key, width: r.width, height: r.height, xf, link: None, placement: Default::default() })))
    }
}

fn font_style(weight: i64, italic: bool) -> String {
    let w = match weight {
        ..=150 => "Thin",
        151..=250 => "ExtraLight",
        251..=325 => "Light",
        326..=450 => "Regular",
        451..=550 => "Medium",
        551..=650 => "SemiBold",
        651..=750 => "Bold",
        751..=850 => "ExtraBold",
        _ => "Black",
    };
    match (w, italic) {
        ("Regular", true) => "Italic".into(),
        (w, true) => format!("{w} Italic"),
        (w, false) => w.into(),
    }
}
