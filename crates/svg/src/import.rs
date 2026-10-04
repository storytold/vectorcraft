//! usvg tree → document conversion.

use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;

use usvg::roxmltree;
use vectorcraft_color::{BlendMode, Color, Gradient, GradientGeom, GradientKind, GradientPaint, GradientStop, Paint};
use vectorcraft_doc::{
    Appearance, AppearanceItem, CharStyle, Dash, Document, FillLayer, ImageBlob, ImageObject, Justify, LayerColor, LineCap, LineJoin, Node, NodeKind,
    StrokeLayer, TextKind, TextObject, TextRun,
};
use vectorcraft_geom::{Affine, BezPath, FillRule, PathData, Point, Vec2};

use crate::SvgError;

pub(crate) fn import(svg: &str) -> Result<(Document, Vec<String>), SvgError> {
    let opt = usvg::Options::default();
    let tree = usvg::Tree::from_str(svg, &opt).map_err(|e| SvgError::Parse(e.to_string()))?;
    let size = tree.size();
    // Document points from CSS pixels: 1 px = 1 pt (as we export), except that a root size in
    // absolute units is that physical size (usvg resolves it to 96 px per inch, i.e. 4/3 px per pt).
    let (kx, ky) = physical_scale(svg);
    let doc = Document::new(size.width() as f64 * kx, size.height() as f64 * ky);
    let mut im = Importer { doc, warnings: Vec::new() };

    // usvg wraps everything in an id-less group carrying the viewBox transform when needed.
    let mut top = tree.root();
    let mut base = Affine::scale_non_uniform(kx, ky) * aff(top.transform());
    if let [usvg::Node::Group(g)] = top.children()
        && g.id().is_empty()
        && is_plain(g)
    {
        base *= aff(g.transform());
        top = g;
    }

    // Top-level `<g id>` elements become layers (Illustrator does the same); otherwise all art goes
    // into "Layer 1".
    let layer_mode =
        !top.children().is_empty() && top.children().iter().all(|c| matches!(c, usvg::Node::Group(g) if !g.id().is_empty() && is_plain(g)));
    if layer_mode {
        im.doc.layers.clear();
        for (i, c) in top.children().iter().enumerate() {
            let usvg::Node::Group(g) = c else { continue };
            let children = im.children(g, base * aff(g.transform()));
            let id = im.doc.alloc_id();
            let mut l = Node::layer(id, g.id(), LayerColor::Preset((i % vectorcraft_doc::LAYER_COLORS.len()) as u8));
            if let Some(ch) = l.children_mut() {
                *ch = children;
            }
            im.doc.layers.push(Arc::new(l));
        }
    } else {
        let children = im.children(top, base);
        if let Some(l) = im.doc.layers.first_mut()
            && let Some(ch) = Arc::make_mut(l).children_mut()
        {
            *ch = children;
        }
    }

    text_fallback(&mut im, svg, size.width() as f64, size.height() as f64, (kx, ky), layer_mode);
    Ok((im.doc, im.warnings))
}

/// Points per CSS pixel along x and y for the root `<svg>`'s `width` / `height`: 0.75 (72 / 96) for
/// absolute units (in, cm, mm, pt, pc), so that `width="210mm"` is 595.3 pt wide; 1 otherwise
/// (unitless, px, %, em, or absent).
fn physical_scale(svg: &str) -> (f64, f64) {
    let Ok(xml) = roxmltree::Document::parse_with_options(svg, roxmltree::ParsingOptions { allow_dtd: true, ..Default::default() }) else {
        return (1.0, 1.0);
    };
    let root = xml.root_element();
    let k = |name: &str| {
        use svgtypes::LengthUnit as U;
        match root.attribute(name).and_then(|v| svgtypes::Length::from_str(v.trim()).ok()).map(|l| l.unit) {
            Some(U::In | U::Cm | U::Mm | U::Pt | U::Pc) => 0.75,
            _ => 1.0,
        }
    };
    (k("width"), k("height"))
}

struct Importer {
    doc: Document,
    warnings: Vec<String>,
}

fn aff(t: usvg::Transform) -> Affine {
    Affine::new([t.sx as f64, t.ky as f64, t.kx as f64, t.sy as f64, t.tx as f64, t.ty as f64])
}

fn is_plain(g: &usvg::Group) -> bool {
    g.clip_path().is_none() && g.mask().is_none() && g.filters().is_empty() && g.opacity().get() >= 1.0 && g.blend_mode() == usvg::BlendMode::Normal
}

fn blend(b: usvg::BlendMode) -> BlendMode {
    use usvg::BlendMode as B;
    match b {
        B::Normal => BlendMode::Normal,
        B::Multiply => BlendMode::Multiply,
        B::Screen => BlendMode::Screen,
        B::Overlay => BlendMode::Overlay,
        B::Darken => BlendMode::Darken,
        B::Lighten => BlendMode::Lighten,
        B::ColorDodge => BlendMode::ColorDodge,
        B::ColorBurn => BlendMode::ColorBurn,
        B::HardLight => BlendMode::HardLight,
        B::SoftLight => BlendMode::SoftLight,
        B::Difference => BlendMode::Difference,
        B::Exclusion => BlendMode::Exclusion,
        B::Hue => BlendMode::Hue,
        B::Saturation => BlendMode::Saturation,
        B::Color => BlendMode::Color,
        B::Luminosity => BlendMode::Luminosity,
    }
}

fn bezpath(p: &usvg::tiny_skia_path::Path, m: Affine) -> BezPath {
    use usvg::tiny_skia_path::PathSegment as S;
    let pt = |p: usvg::tiny_skia_path::Point| m * Point::new(p.x as f64, p.y as f64);
    let mut bp = BezPath::new();
    for s in p.segments() {
        match s {
            S::MoveTo(p) => bp.move_to(pt(p)),
            S::LineTo(p) => bp.line_to(pt(p)),
            S::QuadTo(a, p) => bp.quad_to(pt(a), pt(p)),
            S::CubicTo(a, b, p) => bp.curve_to(pt(a), pt(b), pt(p)),
            S::Close => bp.close_path(),
        }
    }
    bp
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

fn rule(r: usvg::FillRule) -> FillRule {
    match r {
        usvg::FillRule::NonZero => FillRule::NonZero,
        usvg::FillRule::EvenOdd => FillRule::EvenOdd,
    }
}

impl Importer {
    fn warn(&mut self, s: String) {
        if !self.warnings.contains(&s) {
            self.warnings.push(s);
        }
    }

    fn children(&mut self, g: &usvg::Group, acc: Affine) -> Vec<Arc<Node>> {
        g.children().iter().filter_map(|c| self.node(c, acc)).map(Arc::new).collect()
    }

    fn node(&mut self, n: &usvg::Node, acc: Affine) -> Option<Node> {
        match n {
            usvg::Node::Group(g) => self.group(g, acc),
            usvg::Node::Path(p) => self.path(p, acc),
            usvg::Node::Image(i) => self.image(i, acc),
            // Only present when usvg had fonts; text is read from the XML instead.
            usvg::Node::Text(_) => None,
        }
    }

    fn named(&mut self, id: &str, kind: NodeKind) -> Node {
        let nid = self.doc.alloc_id();
        let mut n = Node::new(nid, kind);
        if !id.is_empty() {
            n.name = Some(id.to_string());
        }
        n
    }

    fn group(&mut self, g: &usvg::Group, acc: Affine) -> Option<Node> {
        let ts = acc * aff(g.transform());
        let label = if g.id().is_empty() { "a group".to_string() } else { format!("'{}'", g.id()) };
        let mask = g.mask().and_then(|m| self.opacity_mask(m, ts, &label));
        if !g.filters().is_empty() {
            self.warn(format!("filter on {label} ignored"));
        }
        let children = self.children(g, ts);
        let mut n = if let Some(cp) = g.clip_path() {
            if cp.clip_path().is_some() {
                self.warn(format!("nested clip path on {label} approximated by its outer clip"));
            }
            let clip = self.clip_node(cp, ts)?;
            let mut ch = vec![Arc::new(clip)];
            ch.extend(children);
            self.named(g.id(), NodeKind::Group { children: ch, clip: true })
        } else {
            if children.is_empty() {
                return None;
            }
            // An id-less wrapper around a single object (usvg adds these for opacity/transform on
            // shapes): fold its opacity and blend into the object.
            if g.id().is_empty()
                && mask.is_none()
                && children.len() == 1
                && (g.blend_mode() == usvg::BlendMode::Normal || children[0].blend == BlendMode::Normal)
            {
                // `children.len() == 1` above, so this always yields the one child.
                let Some(only) = children.into_iter().next() else {
                    return None;
                };
                let mut c = Arc::unwrap_or_clone(only);
                c.opacity *= g.opacity().get();
                if g.blend_mode() != usvg::BlendMode::Normal {
                    c.blend = blend(g.blend_mode());
                }
                c.isolate |= g.isolate();
                return Some(c);
            }
            self.named(g.id(), NodeKind::Group { children, clip: false })
        };
        n.opacity = g.opacity().get();
        n.blend = blend(g.blend_mode());
        n.isolate = g.isolate();
        n.mask = mask;
        Some(n)
    }

    /// `<mask>` → opacity mask (luminance; alpha masks are approximated by their luminance).
    fn opacity_mask(&mut self, m: &usvg::Mask, ts: Affine, label: &str) -> Option<Box<vectorcraft_doc::OpacityMask>> {
        if m.kind() == usvg::MaskType::Alpha {
            self.warn(format!("alpha mask on {label} imported as a luminance opacity mask"));
        }
        if m.mask().is_some() {
            self.warn(format!("nested mask on {label} ignored"));
        }
        let children = self.children(m.root(), ts);
        if children.is_empty() {
            return None;
        }
        let art = self.named("", NodeKind::Group { children, clip: false });
        Some(Box::new(vectorcraft_doc::OpacityMask::new(art, true)))
    }

    fn clip_node(&mut self, cp: &usvg::ClipPath, ts: Affine) -> Option<Node> {
        fn collect(g: &usvg::Group, m: Affine, out: &mut Vec<(BezPath, FillRule)>) {
            for c in g.children() {
                match c {
                    usvg::Node::Group(g) => collect(g, m * aff(g.transform()), out),
                    usvg::Node::Path(p) => out.push((bezpath(p.data(), m), p.fill().map(|f| rule(f.rule())).unwrap_or_default())),
                    _ => {}
                }
            }
        }
        let mut shapes = Vec::new();
        collect(cp.root(), ts * aff(cp.transform()), &mut shapes);
        if shapes.is_empty() {
            return None;
        }
        let r = shapes[0].1;
        let mut subs = Vec::new();
        for (bp, _) in &shapes {
            subs.extend(PathData::from_bezpath(bp).subpaths);
        }
        let id = cp.id().to_string();
        if subs.len() == 1 {
            let path = PathData::new(subs);
            return Some(self.named(&id, NodeKind::Path { path, rule: r, live: None, clipping: true, guide: false }));
        }
        let children = subs
            .into_iter()
            .map(|sp| {
                let nid = self.doc.alloc_id();
                Arc::new(Node::new(nid, NodeKind::Path { path: PathData::single(sp), rule: r, live: None, clipping: false, guide: false }))
            })
            .collect();
        Some(self.named(&id, NodeKind::Compound { children, rule: r }))
    }

    fn paint(&mut self, p: &usvg::Paint, m: Affine) -> Paint {
        let stops = |g: &usvg::BaseGradient| {
            g.stops()
                .iter()
                .map(|s| {
                    let c = s.color();
                    GradientStop { offset: s.offset().get(), color: Color::rgb8(c.red, c.green, c.blue), opacity: s.opacity().get(), midpoint: 0.5 }
                })
                .collect::<Vec<_>>()
        };
        let gp = |kind, stops, geom: GradientGeom| {
            Paint::Gradient(Box::new(GradientPaint { gradient: Gradient { kind, stops }, geom: Some(geom), angle: geom.angle_deg(), swatch: None }))
        };
        match p {
            usvg::Paint::Color(c) => Paint::solid(Color::rgb8(c.red, c.green, c.blue)),
            usvg::Paint::LinearGradient(lg) => {
                if lg.spread_method() != usvg::SpreadMethod::Pad {
                    self.warn(format!("gradient '{}': spreadMethod approximated as pad", lg.id()));
                }
                let mut geom =
                    GradientGeom { start: Point::new(lg.x1() as f64, lg.y1() as f64), end: Point::new(lg.x2() as f64, lg.y2() as f64), aspect: 1.0 };
                geom.transform(m * aff(lg.transform()), GradientKind::Linear);
                gp(GradientKind::Linear, stops(lg), geom)
            }
            usvg::Paint::RadialGradient(rg) => {
                if rg.spread_method() != usvg::SpreadMethod::Pad {
                    self.warn(format!("gradient '{}': spreadMethod approximated as pad", rg.id()));
                }
                if (rg.fx() - rg.cx()).abs() > 1e-4 || (rg.fy() - rg.cy()).abs() > 1e-4 {
                    self.warn(format!("gradient '{}': focal point ignored", rg.id()));
                }
                let c = Point::new(rg.cx() as f64, rg.cy() as f64);
                let mut geom = GradientGeom { start: c, end: c + Vec2::new(rg.r().get() as f64, 0.0), aspect: 1.0 };
                geom.transform(m * aff(rg.transform()), GradientKind::Radial);
                gp(GradientKind::Radial, stops(rg), geom)
            }
            usvg::Paint::Pattern(pt) => {
                self.warn(format!("pattern '{}' not supported; painted as none", pt.id()));
                Paint::None
            }
        }
    }

    fn appearance(&mut self, p: &usvg::Path, m: Affine) -> (Appearance, FillRule) {
        let mut fill = None;
        let mut r = FillRule::NonZero;
        if let Some(f) = p.fill() {
            let paint = self.paint(f.paint(), m);
            let mut fl = FillLayer::new(paint);
            fl.opacity = f.opacity().get();
            r = rule(f.rule());
            fill = Some(AppearanceItem::Fill(fl));
        }
        let mut stroke = None;
        if let Some(s) = p.stroke() {
            let scale = m.determinant().abs().sqrt();
            let paint = self.paint(s.paint(), m);
            let mut sl = StrokeLayer::new(paint, s.width().get() as f64 * scale);
            sl.opacity = s.opacity().get();
            sl.cap = match s.linecap() {
                usvg::LineCap::Butt => LineCap::Butt,
                usvg::LineCap::Round => LineCap::Round,
                usvg::LineCap::Square => LineCap::Square,
            };
            sl.join = match s.linejoin() {
                usvg::LineJoin::Miter | usvg::LineJoin::MiterClip => LineJoin::Miter,
                usvg::LineJoin::Round => LineJoin::Round,
                usvg::LineJoin::Bevel => LineJoin::Bevel,
            };
            sl.miter_limit = s.miterlimit().get() as f64;
            if let Some(d) = s.dasharray() {
                sl.dash = Some(Dash {
                    pattern: d.iter().map(|v| *v as f64 * scale).collect(),
                    offset: s.dashoffset() as f64 * scale,
                    align_corners: false,
                });
            }
            stroke = Some(AppearanceItem::Stroke(sl));
        }
        let items = match p.paint_order() {
            usvg::PaintOrder::FillAndStroke => [fill, stroke],
            usvg::PaintOrder::StrokeAndFill => [stroke, fill],
        };
        (Appearance { items: items.into_iter().flatten().collect(), effects: vec![] }, r)
    }

    fn path(&mut self, p: &usvg::Path, acc: Affine) -> Option<Node> {
        if !p.is_visible() {
            return None;
        }
        let path = PathData::from_bezpath(&bezpath(p.data(), acc));
        if path.is_empty() {
            return None;
        }
        let (appearance, r) = self.appearance(p, acc);
        let mut n = if path.subpaths.len() > 1 {
            // Multi-subpath SVG paths are compound paths (as in Illustrator).
            let children = path
                .subpaths
                .into_iter()
                .map(|sp| {
                    let id = self.doc.alloc_id();
                    let mut c = Node::new(id, NodeKind::Path { path: PathData::single(sp), rule: r, live: None, clipping: false, guide: false });
                    c.appearance = appearance.clone();
                    Arc::new(c)
                })
                .collect();
            self.named(p.id(), NodeKind::Compound { children, rule: r })
        } else {
            self.named(p.id(), NodeKind::Path { path, rule: r, live: None, clipping: false, guide: false })
        };
        n.appearance = appearance;
        Some(n)
    }

    fn image(&mut self, i: &usvg::Image, acc: Affine) -> Option<Node> {
        if !i.is_visible() {
            return None;
        }
        let (bytes, mime) = match i.kind() {
            usvg::ImageKind::JPEG(d) => (d, "image/jpeg"),
            usvg::ImageKind::PNG(d) => (d, "image/png"),
            usvg::ImageKind::GIF(d) => (d, "image/gif"),
            usvg::ImageKind::WEBP(d) => (d, "image/webp"),
            usvg::ImageKind::SVG(_) => {
                self.warn("embedded SVG image skipped".into());
                return None;
            }
        };
        let size = i.size();
        let (pw, ph) = image::ImageReader::new(std::io::Cursor::new(&bytes[..]))
            .with_guessed_format()
            .ok()
            .and_then(|r| r.into_dimensions().ok())
            .unwrap_or((size.width().round().max(1.0) as u32, size.height().round().max(1.0) as u32));
        let key = format!("img-{:016x}", fnv1a(bytes));
        self.doc.images.entry(key.clone()).or_insert_with(|| ImageBlob { mime: mime.into(), bytes: Arc::new(bytes.to_vec()) });
        let xf = acc * Affine::scale_non_uniform(size.width() as f64 / pw as f64, size.height() as f64 / ph as f64);
        Some(self.named(i.id(), NodeKind::Image(ImageObject { key, width: pw, height: ph, xf, link: None })))
    }
}

// ---------------------------------------------------------------------------------------------
// <text> fallback: read text elements straight from the XML as live point type.

type XNode<'a, 'i> = roxmltree::Node<'a, 'i>;

struct TextCtx {
    /// `.class` → declarations from `<style>` elements.
    classes: HashMap<String, Vec<(String, String)>>,
}

fn parse_decls(s: &str) -> Vec<(String, String)> {
    s.split(';')
        .filter_map(|d| {
            let (k, v) = d.split_once(':')?;
            Some((k.trim().to_string(), v.trim().trim_end_matches("!important").trim().to_string()))
        })
        .collect()
}

impl TextCtx {
    fn new(doc: &roxmltree::Document) -> Self {
        let mut classes: HashMap<String, Vec<(String, String)>> = HashMap::new();
        for st in doc.descendants().filter(|n| n.has_tag_name("style") || n.tag_name().name() == "style") {
            let css: String = st.children().filter_map(|c| c.text()).collect();
            for rule in css.split('}') {
                let Some((sel, body)) = rule.split_once('{') else { continue };
                let decls = parse_decls(body);
                for s in sel.split(',') {
                    if let Some(c) = s.trim().strip_prefix('.')
                        && c.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_')
                    {
                        classes.entry(c.to_string()).or_default().extend(decls.clone());
                    }
                }
            }
        }
        Self { classes }
    }

    /// A property on this element only: style attribute > class rule > presentation attribute.
    fn own(&self, n: XNode, name: &str) -> Option<String> {
        if let Some(st) = n.attribute("style")
            && let Some((_, v)) = parse_decls(st).into_iter().rev().find(|(k, _)| k == name)
        {
            return Some(v);
        }
        if let Some(cls) = n.attribute("class") {
            for c in cls.split_whitespace().rev() {
                if let Some((_, v)) = self.classes.get(c).and_then(|d| d.iter().rev().find(|(k, _)| k == name)) {
                    return Some(v.clone());
                }
            }
        }
        n.attribute(name).map(str::to_string)
    }

    /// An inherited property.
    fn prop(&self, n: XNode, name: &str) -> Option<String> {
        n.ancestors().filter(|a| a.is_element()).find_map(|a| self.own(a, name).filter(|v| v != "inherit"))
    }
}

fn first_number(s: Option<&str>) -> f64 {
    s.and_then(|s| s.split(|c: char| c.is_whitespace() || c == ',').find(|t| !t.is_empty()).map(str::to_string))
        .and_then(|t| svgtypes::Length::from_str(&t).ok())
        .map(|l| l.number)
        .unwrap_or(0.0)
}

fn parse_transform(s: Option<&str>) -> Affine {
    s.and_then(|s| svgtypes::Transform::from_str(s).ok()).map(|t| Affine::new([t.a, t.b, t.c, t.d, t.e, t.f])).unwrap_or(Affine::IDENTITY)
}

fn parse_color_paint(v: &str) -> Paint {
    let v = v.trim();
    if v == "none" {
        return Paint::None;
    }
    if let Ok(c) = svgtypes::Color::from_str(v) {
        return Paint::solid(Color::rgb8(c.red, c.green, c.blue));
    }
    // url(#…) and other paints: fall back to black (or the fallback colour after the url).
    if let Some(rest) = v.strip_prefix("url(").and_then(|r| r.split_once(')')).map(|(_, r)| r.trim())
        && let Ok(c) = svgtypes::Color::from_str(rest)
    {
        return Paint::solid(Color::rgb8(c.red, c.green, c.blue));
    }
    Paint::solid(Color::BLACK)
}

fn char_style(ctx: &TextCtx, n: XNode) -> CharStyle {
    let mut st = CharStyle::default();
    if let Some(f) = ctx.prop(n, "font-family") {
        let fam = f.split(',').next().unwrap_or("").trim().trim_matches(|c| c == '\'' || c == '"').to_string();
        if !fam.is_empty() {
            st.font_family = fam;
        }
    }
    if let Some(s) = ctx.prop(n, "font-size")
        && let Ok(l) = svgtypes::Length::from_str(s.trim())
        && l.number > 0.0
    {
        st.size = match l.unit {
            svgtypes::LengthUnit::Em => l.number * 12.0,
            svgtypes::LengthUnit::Percent => l.number / 100.0 * 12.0,
            _ => l.number,
        };
    }
    let bold = ctx.prop(n, "font-weight").is_some_and(|w| w == "bold" || w == "bolder" || w.parse::<u32>().is_ok_and(|v| v >= 600));
    let italic = ctx.prop(n, "font-style").is_some_and(|s| s == "italic" || s == "oblique");
    st.font_style = match (bold, italic) {
        (true, true) => "Bold Italic",
        (true, false) => "Bold",
        (false, true) => "Italic",
        _ => "Regular",
    }
    .into();
    if let Some(f) = ctx.prop(n, "fill") {
        st.fill = parse_color_paint(&f);
    }
    if let Some(s) = ctx.prop(n, "stroke") {
        st.stroke = parse_color_paint(&s);
        st.stroke_width = ctx.prop(n, "stroke-width").map(|w| first_number(Some(&w))).unwrap_or(1.0);
    }
    if let Some(ls) = ctx.prop(n, "letter-spacing") {
        let v = first_number(Some(&ls));
        if v != 0.0 {
            st.tracking = v / st.size * 1000.0;
        }
    }
    if let Some(d) = ctx.prop(n, "text-decoration") {
        st.underline = d.contains("underline");
        st.strikethrough = d.contains("line-through");
    }
    st
}

fn view_box_transform(root: XNode, w: f64, h: f64) -> Affine {
    let Some(vb) = root.attribute("viewBox").and_then(|v| svgtypes::ViewBox::from_str(v).ok()) else { return Affine::IDENTITY };
    if vb.w <= 0.0 || vb.h <= 0.0 {
        return Affine::IDENTITY;
    }
    let ar = root.attribute("preserveAspectRatio").and_then(|v| svgtypes::AspectRatio::from_str(v).ok()).unwrap_or_default();
    let (sx, sy) = (w / vb.w, h / vb.h);
    use svgtypes::Align as A;
    if ar.align == A::None {
        return Affine::scale_non_uniform(sx, sy) * Affine::translate((-vb.x, -vb.y));
    }
    let s = if ar.slice { sx.max(sy) } else { sx.min(sy) };
    let (fx, fy) = match ar.align {
        A::XMinYMin => (0.0, 0.0),
        A::XMidYMin => (0.5, 0.0),
        A::XMaxYMin => (1.0, 0.0),
        A::XMinYMid => (0.0, 0.5),
        A::XMinYMax => (0.0, 1.0),
        A::XMidYMax => (0.5, 1.0),
        A::XMaxYMid => (1.0, 0.5),
        A::XMaxYMax => (1.0, 1.0),
        A::XMidYMid | A::None => (0.5, 0.5),
    };
    Affine::translate(((w - vb.w * s) * fx, (h - vb.h * s) * fy)) * Affine::scale(s) * Affine::translate((-vb.x, -vb.y))
}

/// Most line breaks one absolutely positioned `<tspan>` adds before itself.
const MAX_TSPAN_LINE_GAP: f64 = 10_000.0;

struct RunBuilder {
    runs: Vec<TextRun>,
    last_y: f64,
    preserve: bool,
}

impl RunBuilder {
    fn push(&mut self, text: &str, style: CharStyle) {
        if text.is_empty() {
            return;
        }
        if let Some(l) = self.runs.last_mut()
            && l.style == style
        {
            l.text.push_str(text);
            return;
        }
        self.runs.push(TextRun { text: text.into(), style });
    }
    fn newline(&mut self, n: usize) {
        if let Some(l) = self.runs.last_mut() {
            for _ in 0..n {
                l.text.push('\n');
            }
        }
    }
}

fn collect_runs(ctx: &TextCtx, el: XNode, rb: &mut RunBuilder) {
    for c in el.children() {
        if c.is_text() {
            let raw = c.text().unwrap_or("");
            let t = if rb.preserve {
                raw.replace(['\n', '\r', '\t'], " ")
            } else {
                let s: String = raw.chars().filter(|c| *c != '\n' && *c != '\r').map(|c| if c == '\t' { ' ' } else { c }).collect();
                let mut out = String::new();
                for ch in s.chars() {
                    if ch == ' '
                        && (out.ends_with(' ') || (out.is_empty() && rb.runs.last().is_none_or(|r| r.text.ends_with(' ') || r.text.ends_with('\n'))))
                    {
                        continue;
                    }
                    out.push(ch);
                }
                out
            };
            rb.push(&t, char_style(ctx, el));
        } else if c.is_element() && matches!(c.tag_name().name(), "tspan" | "textPath" | "a") {
            if ctx.own(c, "display").as_deref() == Some("none") {
                continue;
            }
            if !rb.runs.is_empty() {
                let lead = char_style(ctx, c).effective_leading().max(1e-6);
                if let Some(y) = c.attribute("y") {
                    let y = first_number(Some(y));
                    // One line break per line of baseline gap: none for a tspan on the same baseline
                    // (a styled or kerned run of the same line), at least one for a tspan placed
                    // higher up, and capped so that a far-off (or non-finite) y can't ask for billions.
                    let gap = ((y - rb.last_y) / lead).round();
                    let n = if !gap.is_finite() {
                        1
                    } else if (y - rb.last_y).abs() < lead * 0.5 {
                        0
                    } else {
                        gap.clamp(1.0, MAX_TSPAN_LINE_GAP) as usize
                    };
                    rb.newline(n);
                    rb.last_y = y;
                } else if c.attribute("dy").is_some_and(|dy| first_number(Some(dy)) > 1e-9) {
                    rb.last_y += first_number(c.attribute("dy"));
                    rb.newline(1);
                }
            }
            collect_runs(ctx, c, rb);
        }
    }
}

fn text_fallback(im: &mut Importer, svg: &str, w: f64, h: f64, (kx, ky): (f64, f64), layer_mode: bool) {
    let Ok(xml) = roxmltree::Document::parse_with_options(svg, roxmltree::ParsingOptions { allow_dtd: true, ..Default::default() }) else { return };
    let ctx = TextCtx::new(&xml);
    let root = xml.root_element();
    let vb = Affine::scale_non_uniform(kx, ky) * view_box_transform(root, w, h);
    const SKIP: [&str; 8] = ["defs", "clipPath", "mask", "pattern", "symbol", "marker", "title", "desc"];
    for t in root.descendants().filter(|n| n.is_element() && n.tag_name().name() == "text") {
        if t.ancestors().skip(1).any(|a| a.is_element() && SKIP.contains(&a.tag_name().name())) {
            continue;
        }
        if t.ancestors().filter(|a| a.is_element()).any(|a| ctx.own(a, "display").as_deref() == Some("none")) {
            continue;
        }
        if ctx.prop(t, "visibility").is_some_and(|v| v == "hidden" || v == "collapse") {
            continue;
        }
        // Transform: viewBox, then each ancestor's transform from the root down.
        let mut acc = vb;
        let chain: Vec<XNode> = t.ancestors().filter(|a| a.is_element()).collect();
        for a in chain.iter().rev() {
            if *a != root {
                acc *= parse_transform(a.attribute("transform"));
            }
        }
        let y0 = first_number(t.attribute("y"));
        let xf = acc * Affine::translate((first_number(t.attribute("x")), y0));
        let preserve = t.ancestors().filter(|a| a.is_element()).find_map(|a| a.attribute((roxmltree::NS_XML_URI, "space"))) == Some("preserve");
        // tspan y values are absolute in the text's user space, like the text's own y.
        let mut rb = RunBuilder { runs: Vec::new(), last_y: y0, preserve };
        collect_runs(&ctx, t, &mut rb);
        if !preserve {
            if let Some(f) = rb.runs.first_mut() {
                f.text = f.text.trim_start().to_string();
            }
            if let Some(l) = rb.runs.last_mut() {
                l.text = l.text.trim_end().to_string();
            }
            rb.runs.retain(|r| !r.text.is_empty());
        }
        if rb.runs.is_empty() {
            continue;
        }
        // Type on a path: a <textPath> naming a <path> (in the text's user space) by href.
        let on_path = t.children().find(|c| c.is_element() && c.tag_name().name() == "textPath").and_then(|tp| {
            let href = tp.attribute("href").or_else(|| tp.attribute(("http://www.w3.org/1999/xlink", "href")))?;
            let id = href.strip_prefix('#')?;
            let el = xml.descendants().find(|n| n.is_element() && n.tag_name().name() == "path" && n.attribute("id") == Some(id))?;
            let mut bp = BezPath::from_svg(el.attribute("d")?).ok()?;
            bp.apply_affine(acc * parse_transform(el.attribute("transform")));
            let start = match tp.attribute("startOffset").map(str::trim) {
                Some(o) if o.ends_with('%') => first_number(o.strip_suffix('%')) / 100.0,
                Some(o) => {
                    let len: f64 = bp.segments().map(|s| kurbo::ParamCurveArclen::arclen(&s, 1e-3)).sum();
                    if len > 0.0 { first_number(Some(o)) / len } else { 0.0 }
                }
                None => 0.0,
            };
            Some((PathData::from_bezpath(&bp), start.clamp(0.0, 1.0)))
        });
        let justify = match ctx.prop(t, "text-anchor").as_deref() {
            Some("middle") => Justify::Center,
            Some("end") => Justify::Right,
            _ => Justify::Left,
        };
        let (kind, xf) = match on_path {
            Some((path, start)) => (TextKind::OnPath { path, start }, Affine::IDENTITY),
            None => (TextKind::Point, xf),
        };
        let mut obj = TextObject {
            kind,
            xf,
            runs: rb.runs,
            para: Default::default(),
            area: Default::default(),
            path_effect: Default::default(),
            wrap: Vec::new(),
            cached_bounds: None,
        };
        obj.para.justify = justify;
        let mut node = im.named(t.attribute("id").unwrap_or(""), NodeKind::Text(Box::new(obj)));
        if let Some(o) = ctx.own(t, "opacity").and_then(|o| o.trim().parse::<f32>().ok()) {
            node.opacity = o.clamp(0.0, 1.0);
        }
        // Place on the layer made from the top-level <g> containing the text, if any.
        let top_id = if layer_mode { chain.iter().rev().nth(1).filter(|g| g.tag_name().name() == "g").and_then(|g| g.attribute("id")) } else { None };
        let li = top_id.and_then(|id| im.doc.layers.iter().position(|l| l.name.as_deref() == Some(id)));
        let li = match (li, top_id) {
            (Some(i), _) => i,
            (None, Some(id)) => {
                im.doc.add_layer(Some(id));
                im.doc.layers.len() - 1
            }
            (None, None) => im.doc.layers.len().saturating_sub(1),
        };
        if im.doc.layers.is_empty() {
            im.doc.add_layer(Some("Layer 1"));
        }
        if let Some(ch) = Arc::make_mut(&mut im.doc.layers[li]).children_mut() {
            ch.push(Arc::new(node));
        }
    }
}
