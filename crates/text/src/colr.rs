//! COLR colour glyphs (v0 layers, v1 paint graphs): read into a tree of document nodes in font
//! units (y down, as outlines are), which the renderer, the exporters and Create Outlines draw and
//! keep as they do any art. A glyph whose paints VectorCraft has no form for is drawn into an image
//! instead, as bitmap glyphs are ([`GlyphImage`]); [`Tree::unsupported`] names what made it one.

use std::sync::Arc;

use kurbo::{Affine, BezPath, Point, Rect, Shape, Vec2};
use skrifa::color::{Brush, ColorGlyph, ColorGlyphCollection, ColorPainter, ColorPalettes, ColorStop, CompositeMode, Extend, Transform};
use skrifa::instance::{LocationRef, Size};
use skrifa::outline::DrawSettings;
use skrifa::raw::TableProvider;
use skrifa::raw::types::BoundingBox;
use skrifa::{GlyphId, MetadataProvider};
use vectorcraft_doc::color::{BlendMode, Color, Gradient, GradientGeom, GradientKind, GradientPaint, GradientStop, Paint};
use vectorcraft_doc::{Appearance, FillLayer, ImageBlob, Node, NodeId, NodeKind};
use vectorcraft_geom::PathData;
use vello_cpu::peniko;

use crate::GlyphImage;
use crate::fontdb::FlipPen;

/// The most nodes a colour glyph's tree may have: past it, a font's glyph draws its outline.
const MAX_COLR_NODES: usize = 4096;

/// The most path elements the clips and fills of a colour glyph's tree may hold in all: layers
/// that repeat one large outline each hold a copy of it.
const MAX_COLR_POINTS: usize = 1 << 20;
/// The deepest the groups of a colour glyph may nest.
const MAX_COLR_DEPTH: usize = 64;
/// The pixels per em an image of a colour glyph is drawn at.
const FALLBACK_PPEM: f64 = 256.0;
/// The largest side of such an image, in pixels.
const MAX_FALLBACK_SIDE: f64 = 1024.0;
/// The id of the layers' nodes: none, as the art exporters make on the fly has (Create Outlines
/// gives them ids of the document's).
const NO_ID: NodeId = NodeId(u64::MAX);
/// Font units y up → y down.
const FLIP: Affine = Affine::new([1.0, 0.0, 0.0, -1.0, 0.0, 0.0]);

/// A colour glyph as VectorCraft draws it.
#[derive(Clone)]
pub(crate) enum ColourContent {
    /// Its layers as document nodes in font units (y down): a group.
    Art(Arc<Node>),
    /// Its picture, for paints VectorCraft has no form for.
    Image(Arc<GlyphImage>),
}

/// Does `font` have COLR colour glyphs?
pub(crate) fn has_colour_layers(font: &skrifa::FontRef<'_>) -> bool {
    font.colr().is_ok()
}

/// Does glyph `gid` of `font` have COLR layers?
pub(crate) fn has_colour_glyph(font: &skrifa::FontRef<'_>, gid: u32) -> bool {
    ColorGlyphCollection::new(font).get(GlyphId::new(gid)).is_some()
}

/// Glyph `gid`'s colour content in `font` at `location`, palette 0's colours and `foreground` for
/// palette index 0xFFFF, and whether it used `foreground`; `None` when it isn't a colour glyph, or
/// its graph doesn't read (a cycle, too deep, too many layers).
pub(crate) fn colour_glyph(
    font: &skrifa::FontRef<'_>,
    location: LocationRef<'_>,
    upem: f64,
    gid: u32,
    foreground: Color,
) -> (Option<ColourContent>, bool) {
    let Some(glyph) = ColorGlyphCollection::new(font).get(GlyphId::new(gid)) else { return (None, false) };
    let mut tree = Tree::new(font, location, foreground);
    if glyph.paint(location, &mut tree).is_err() {
        return (None, false);
    }
    let used = tree.foreground_used;
    if tree.unsupported.is_some() {
        // What it paints, y up, for a glyph without a clip box.
        let painted = tree.painted.map(|b| FLIP.transform_rect_bbox(b));
        return (raster(&glyph, font, location, upem, foreground, painted).map(|i| ColourContent::Image(Arc::new(i))), used);
    }
    (tree.finish().map(ColourContent::Art), used)
}

/// Palette 0's colours with their alpha.
fn palette(font: &skrifa::FontRef<'_>) -> Vec<(Color, f32)> {
    let palettes = ColorPalettes::new(font);
    let Some(palette) = palettes.get(0) else { return vec![] };
    palette.colors().iter().map(|c| (Color::rgb8(c.red, c.green, c.blue), f32::from(c.alpha) / 255.0)).collect()
}

/// The colour and alpha of palette entry `index` (0xFFFF: `foreground`), times `alpha`; `None`
/// for an entry the palette doesn't have.
fn colour(palette: &[(Color, f32)], foreground: Color, index: u16, alpha: f32) -> Option<(Color, f32)> {
    let (c, a) = if index == 0xFFFF { (foreground, 1.0) } else { *palette.get(usize::from(index))? };
    Some((c, (a * alpha).clamp(0.0, 1.0)))
}

fn affine(t: &Transform) -> Affine {
    Affine::new([t.xx, t.yx, t.xy, t.yy, t.dx, t.dy].map(f64::from))
}

fn point(p: skrifa::raw::types::Point<f32>) -> Point {
    Point::new(f64::from(p.x), f64::from(p.y))
}

fn box_path(b: &BoundingBox<f32>) -> BezPath {
    Rect::new(f64::from(b.x_min), f64::from(b.y_min), f64::from(b.x_max), f64::from(b.y_max)).to_path(0.1)
}

/// Glyph `gid`'s outline in font units, y up (the font database's pen draws y down, as text
/// space has it).
fn outline(font: &skrifa::FontRef<'_>, location: LocationRef<'_>, gid: GlyphId) -> BezPath {
    let mut pen = FlipPen(BezPath::new());
    if let Some(g) = font.outline_glyphs().get(gid) {
        let _ = g.draw(DrawSettings::unhinted(Size::unscaled(), location), &mut pen);
    }
    let mut path = pen.0;
    path.apply_affine(FLIP);
    path
}

/// A group being built: its clipping path, or the blend mode of a composite's layer.
struct Open {
    clip: Option<BezPath>,
    blend: Option<BlendMode>,
    children: Vec<Arc<Node>>,
    /// A fill filled the clip's whole area: alone, it is the clip filled.
    flooded: bool,
}

/// A [`ColorPainter`] building a colour glyph's tree.
struct Tree<'a> {
    font: &'a skrifa::FontRef<'a>,
    location: LocationRef<'a>,
    palette: Vec<(Color, f32)>,
    foreground: Color,
    /// Font units y up → the tree's space (y down), with the paint graph's transforms.
    transforms: Vec<Affine>,
    groups: Vec<Open>,
    nodes: usize,
    /// The path elements of the clips and fills so far.
    points: usize,
    /// What VectorCraft has no form for, the first one met: the glyph becomes an image.
    unsupported: Option<&'static str>,
    /// The graph is past [`MAX_COLR_NODES`] or [`MAX_COLR_DEPTH`].
    too_large: bool,
    /// The box of what it paints, in the tree's space.
    painted: Option<Rect>,
    /// It paints with the foreground colour (palette index 0xFFFF).
    foreground_used: bool,
}

impl<'a> Tree<'a> {
    fn new(font: &'a skrifa::FontRef<'a>, location: LocationRef<'a>, foreground: Color) -> Self {
        let root = Open { clip: None, blend: Some(BlendMode::Normal), children: vec![], flooded: false };
        Tree {
            font,
            location,
            palette: palette(font),
            foreground,
            transforms: vec![FLIP],
            groups: vec![root],
            nodes: 0,
            points: 0,
            unsupported: None,
            too_large: false,
            painted: None,
            foreground_used: false,
        }
    }

    fn transform(&self) -> Affine {
        self.transforms.last().copied().unwrap_or(FLIP)
    }

    fn add(&mut self, node: Node) {
        self.nodes += 1;
        self.too_large |= self.nodes > MAX_COLR_NODES;
        if let Some(g) = self.groups.last_mut().filter(|_| !self.too_large) {
            g.children.push(Arc::new(node));
        }
    }

    /// Count `path` against [`MAX_COLR_POINTS`]: whether it may be kept.
    fn charge(&mut self, path: &BezPath) -> bool {
        self.points = self.points.saturating_add(path.elements().len());
        self.too_large |= self.points > MAX_COLR_POINTS;
        !self.too_large
    }

    fn open(&mut self, group: Open) {
        self.too_large |= self.groups.len() >= MAX_COLR_DEPTH;
        self.groups.push(group);
    }

    /// Closes the innermost group into its parent: a clip with only the fill it clips is that
    /// filled path; an empty group goes.
    fn close(&mut self) {
        // The root stays open until `finish`.
        if self.groups.len() < 2 {
            return;
        }
        let Some(g) = self.groups.pop() else { return };
        if g.children.is_empty() {
            return;
        }
        let mut children = g.children;
        if let Some(clip) = g.clip {
            if let ([only], true) = (children.as_slice(), g.flooded) {
                let only = only.clone();
                return self.add_arc(only);
            }
            let mut path = Node::path(NO_ID, PathData::from_bezpath(&clip), Appearance::default());
            if let NodeKind::Path { clipping, .. } = &mut path.kind {
                *clipping = true;
            }
            children.insert(0, Arc::new(path));
            let mut group = Node::group(NO_ID, children);
            if let NodeKind::Group { clip, .. } = &mut group.kind {
                *clip = true;
            }
            return self.add(group);
        }
        let mut group = Node::group(NO_ID, children);
        group.isolate = true;
        group.blend = g.blend.unwrap_or(BlendMode::Normal);
        self.add(group);
    }

    fn add_arc(&mut self, node: Arc<Node>) {
        if let Some(g) = self.groups.last_mut() {
            g.children.push(node);
        }
    }

    /// The tree: the glyph as a group. It needs no isolation of its own: blend modes only come
    /// with composites, whose groups are isolated, so they never reach the art behind the type.
    fn finish(mut self) -> Option<Arc<Node>> {
        if self.too_large {
            return None;
        }
        while self.groups.len() > 1 {
            self.close();
        }
        let root = self.groups.pop()?;
        if root.children.is_empty() {
            return None;
        }
        Some(Arc::new(Node::group(NO_ID, root.children)))
    }

    /// `brush` as a VectorCraft paint and opacity, in the tree's space; `None` with
    /// [`Self::unsupported`] set when it has no form there.
    fn paint(&mut self, brush: &Brush<'_>) -> Option<(Paint, f32)> {
        self.foreground_used |= match brush {
            Brush::Solid { palette_index, .. } => *palette_index == 0xFFFF,
            Brush::LinearGradient { color_stops, .. } | Brush::RadialGradient { color_stops, .. } | Brush::SweepGradient { color_stops, .. } => {
                color_stops.iter().any(|s| s.palette_index == 0xFFFF)
            }
        };
        let xf = self.transform();
        let stops = |stops: &[ColorStop], tree: &Self| -> Vec<GradientStop> {
            stops
                .iter()
                .filter_map(|s| {
                    let (color, opacity) = colour(&tree.palette, tree.foreground, s.palette_index, s.alpha)?;
                    Some(GradientStop { opacity, ..GradientStop::new(s.offset, color) })
                })
                .collect()
        };
        let gradient = |kind, stops: Vec<GradientStop>, geom| {
            let mut paint = GradientPaint::new(Gradient { kind, stops, ..Gradient::default() });
            paint.geom = Some(geom);
            Paint::Gradient(Box::new(paint))
        };
        match brush {
            Brush::Solid { palette_index, alpha } => {
                let (c, a) = colour(&self.palette, self.foreground, *palette_index, *alpha)?;
                Some((Paint::solid(c), a))
            }
            Brush::LinearGradient { p0, p1, color_stops, extend } => {
                if *extend != Extend::Pad {
                    self.unsupported.get_or_insert("a repeating or reflecting gradient");
                    return None;
                }
                let (p0, p1) = (point(*p0), point(*p1));
                let (stops, first, last) = normalized(stops(color_stops, self))?;
                // Stops past 0 and 1 move the ends instead.
                let mut geom = GradientGeom { start: p0.lerp(p1, f64::from(first)), end: p0.lerp(p1, f64::from(last)), aspect: 1.0, focal: None };
                geom.transform(xf, GradientKind::Linear);
                Some((gradient(GradientKind::Linear, stops, geom), 1.0))
            }
            Brush::RadialGradient { c0, r0, c1, r1, color_stops, extend } => {
                if *extend != Extend::Pad {
                    self.unsupported.get_or_insert("a repeating or reflecting gradient");
                    return None;
                }
                let (c0, c1, r0, r1) = (point(*c0), point(*c1), f64::from(*r0), f64::from(*r1));
                let concentric = (c0 - c1).hypot() < 1e-6;
                if (r0 > 1e-6 && !concentric) || r1 <= r0 || (c0 - c1).hypot() >= r1 {
                    self.unsupported.get_or_insert("a radial gradient from a circle");
                    return None;
                }
                let mut stops = stops(color_stops, self);
                if stops.iter().any(|s| !(0.0..=1.0).contains(&s.offset)) {
                    self.unsupported.get_or_insert("a radial gradient with stops past its circles");
                    return None;
                }
                // A start circle around the centre starts the stops further out.
                for s in &mut stops {
                    s.offset = ((r0 + f64::from(s.offset) * (r1 - r0)) / r1) as f32;
                }
                let mut geom = GradientGeom { start: c1, end: c1 + Vec2::new(r1, 0.0), aspect: 1.0, focal: (!concentric).then_some(c0) };
                geom.transform(xf, GradientKind::Radial);
                Some((gradient(GradientKind::Radial, stops, geom), 1.0))
            }
            Brush::SweepGradient { .. } => {
                self.unsupported.get_or_insert("a sweep gradient");
                None
            }
        }
    }

    /// `path` (in the tree's space) filled with `brush` (in the current transform's).
    fn fill_path(&mut self, path: &BezPath, brush: &Brush<'_>) {
        if !self.charge(path) {
            return;
        }
        let b = path.bounding_box();
        self.painted = Some(self.painted.map_or(b, |p| p.union(b)));
        let Some((paint, opacity)) = self.paint(brush) else { return };
        let fill = vectorcraft_doc::AppearanceItem::Fill(FillLayer { opacity, ..FillLayer::new(paint) });
        self.add(Node::path(NO_ID, PathData::from_bezpath(path), Appearance { items: vec![fill], ..Appearance::default() }));
    }
}

/// `stops` sorted, from 0 to 1 (as the gradient's ends then move), and where the first and last
/// were; `None` without stops.
fn normalized(mut stops: Vec<GradientStop>) -> Option<(Vec<GradientStop>, f32, f32)> {
    stops.sort_by(|a, b| a.offset.total_cmp(&b.offset));
    let (first, last) = (stops.first()?.offset, stops.last()?.offset);
    let span = last - first;
    for s in &mut stops {
        s.offset = if span > 0.0 { (s.offset - first) / span } else { 0.0 };
    }
    Some((stops, first, if span > 0.0 { last } else { first + 1.0 }))
}

impl ColorPainter for Tree<'_> {
    fn push_transform(&mut self, transform: Transform) {
        let t = self.transform() * affine(&transform);
        self.transforms.push(t);
    }

    fn pop_transform(&mut self) {
        if self.transforms.len() > 1 {
            self.transforms.pop();
        }
    }

    fn push_clip_glyph(&mut self, glyph_id: GlyphId) {
        let mut clip = outline(self.font, self.location, glyph_id);
        clip.apply_affine(self.transform());
        self.charge(&clip);
        self.open(Open { clip: Some(clip), blend: None, children: vec![], flooded: false });
    }

    fn push_clip_box(&mut self, clip_box: BoundingBox<f32>) {
        let mut clip = box_path(&clip_box);
        clip.apply_affine(self.transform());
        self.open(Open { clip: Some(clip), blend: None, children: vec![], flooded: false });
    }

    fn pop_clip(&mut self) {
        self.close();
    }

    /// A fill fills the innermost clip's shape (a layer between them stays around it).
    fn fill(&mut self, brush: Brush<'_>) {
        let Some(at) = self.groups.iter().rposition(|g| g.clip.is_some()) else {
            self.unsupported.get_or_insert("a fill outside any glyph");
            return;
        };
        let Some(clip) = self.groups.get(at).and_then(|g| g.clip.clone()) else { return };
        if at + 1 == self.groups.len()
            && let Some(g) = self.groups.last_mut()
        {
            g.flooded = g.children.is_empty();
        }
        self.fill_path(&clip, &brush);
    }

    /// The glyph's outline filled, the brush moved by `brush_transform` (the glyph stays).
    fn fill_glyph(&mut self, glyph_id: GlyphId, brush_transform: Option<Transform>, brush: Brush<'_>) {
        let mut path = outline(self.font, self.location, glyph_id);
        path.apply_affine(self.transform());
        let brush_xf = self.transform() * brush_transform.as_ref().map_or(Affine::IDENTITY, affine);
        self.transforms.push(brush_xf);
        self.fill_path(&path, &brush);
        self.transforms.pop();
    }

    fn push_layer(&mut self, mode: CompositeMode) {
        let blend = blend_mode(mode);
        if blend.is_none() {
            self.unsupported.get_or_insert("a compositing mode other than blending");
        }
        self.open(Open { clip: None, blend, children: vec![], flooded: false });
    }

    fn pop_layer(&mut self) {
        self.close();
    }
}

/// COLRv1's blend composite modes as VectorCraft's (`SrcOver` is Normal); `None` for the
/// Porter-Duff modes, which it has no form for.
fn blend_mode(mode: CompositeMode) -> Option<BlendMode> {
    Some(match mode {
        CompositeMode::SrcOver => BlendMode::Normal,
        CompositeMode::Screen => BlendMode::Screen,
        CompositeMode::Overlay => BlendMode::Overlay,
        CompositeMode::Darken => BlendMode::Darken,
        CompositeMode::Lighten => BlendMode::Lighten,
        CompositeMode::ColorDodge => BlendMode::ColorDodge,
        CompositeMode::ColorBurn => BlendMode::ColorBurn,
        CompositeMode::HardLight => BlendMode::HardLight,
        CompositeMode::SoftLight => BlendMode::SoftLight,
        CompositeMode::Difference => BlendMode::Difference,
        CompositeMode::Exclusion => BlendMode::Exclusion,
        CompositeMode::Multiply => BlendMode::Multiply,
        CompositeMode::HslHue => BlendMode::Hue,
        CompositeMode::HslSaturation => BlendMode::Saturation,
        CompositeMode::HslColor => BlendMode::Color,
        CompositeMode::HslLuminosity => BlendMode::Luminosity,
        _ => return None,
    })
}

/// Colour glyph `glyph` drawn into an image at [`FALLBACK_PPEM`], placed in font units (y down)
/// as bitmap glyphs are, in its clip box or else the box of what it `painted` (y up); `None` when
/// it paints nothing or doesn't draw.
fn raster(
    glyph: &ColorGlyph<'_>,
    font: &skrifa::FontRef<'_>,
    location: LocationRef<'_>,
    upem: f64,
    foreground: Color,
    painted: Option<Rect>,
) -> Option<GlyphImage> {
    let b = glyph.bounding_box(location, Size::unscaled()).map(|b| Rect::new(b.x_min.into(), b.y_min.into(), b.x_max.into(), b.y_max.into()));
    // Without a clip box, the box of what it paints (`painted`, y up).
    let b = b.or(painted).filter(|b| b.area() > 0.0)?;
    let s = (FALLBACK_PPEM / upem.max(1.0)).min(MAX_FALLBACK_SIDE / b.width().max(b.height()));
    if !(s.is_finite() && s > 0.0) {
        return None;
    }
    let (w, h) = ((b.width() * s).ceil() as u16, (b.height() * s).ceil() as u16);
    if w == 0 || h == 0 {
        return None;
    }
    // Font units y up → pixels.
    let to_pixels = Affine::new([s, 0.0, 0.0, -s, -b.x0 * s, b.y1 * s]);
    let mut ctx = vello_cpu::RenderContext::new(w, h);
    let mut painter = Pixels { ctx: &mut ctx, font, location, palette: palette(font), foreground, transforms: vec![to_pixels], width: w, height: h };
    glyph.paint(location, &mut painter).ok()?;
    ctx.flush();
    let mut pm = vello_cpu::Pixmap::new(w, h);
    ctx.render(&mut pm, &mut vello_cpu::Resources::new());
    let png = pm.into_png().ok()?;
    let xf = Affine::new([1.0 / s, 0.0, 0.0, 1.0 / s, b.x0, -b.y1]);
    Some(GlyphImage { blob: ImageBlob::png(png), width: u32::from(w), height: u32::from(h), xf })
}

/// A [`ColorPainter`] drawing a colour glyph with vello_cpu.
struct Pixels<'a, 'c> {
    ctx: &'c mut vello_cpu::RenderContext,
    font: &'a skrifa::FontRef<'a>,
    location: LocationRef<'a>,
    palette: Vec<(Color, f32)>,
    foreground: Color,
    transforms: Vec<Affine>,
    width: u16,
    height: u16,
}

impl Pixels<'_, '_> {
    fn transform(&self) -> Affine {
        self.transforms.last().copied().unwrap_or_default()
    }

    fn peniko_colour(&self, index: u16, alpha: f32) -> peniko::Color {
        let (c, a) = colour(&self.palette, self.foreground, index, alpha).unwrap_or((Color::BLACK, 0.0));
        let [r, g, b, a] = c.to_rgba8(a);
        peniko::Color::from_rgba8(r, g, b, a)
    }

    fn clip(&mut self, path: &BezPath) {
        self.ctx.set_transform(self.transform());
        self.ctx.push_clip_layer(path);
    }
}

impl ColorPainter for Pixels<'_, '_> {
    fn push_transform(&mut self, transform: Transform) {
        let t = self.transform() * affine(&transform);
        self.transforms.push(t);
    }

    fn pop_transform(&mut self) {
        if self.transforms.len() > 1 {
            self.transforms.pop();
        }
    }

    fn push_clip_glyph(&mut self, glyph_id: GlyphId) {
        let path = outline(self.font, self.location, glyph_id);
        self.clip(&path);
    }

    fn push_clip_box(&mut self, clip_box: BoundingBox<f32>) {
        self.clip(&box_path(&clip_box));
    }

    fn pop_clip(&mut self) {
        self.ctx.pop_layer();
    }

    fn fill(&mut self, brush: Brush<'_>) {
        let stops = |stops: &[ColorStop]| -> Vec<peniko::ColorStop> {
            stops.iter().map(|s| peniko::ColorStop::from((s.offset, self.peniko_colour(s.palette_index, s.alpha)))).collect()
        };
        let extend = |e: Extend| match e {
            Extend::Repeat => peniko::Extend::Repeat,
            Extend::Reflect => peniko::Extend::Reflect,
            _ => peniko::Extend::Pad,
        };
        let paint: peniko::Brush = match brush {
            Brush::Solid { palette_index, alpha } => peniko::Brush::Solid(self.peniko_colour(palette_index, alpha)),
            Brush::LinearGradient { p0, p1, color_stops, extend: e } => {
                peniko::Gradient::new_linear(point(p0), point(p1)).with_stops(stops(color_stops).as_slice()).with_extend(extend(e)).into()
            }
            Brush::RadialGradient { c0, r0, c1, r1, color_stops, extend: e } => peniko::Gradient::new_two_point_radial(point(c0), r0, point(c1), r1)
                .with_stops(stops(color_stops).as_slice())
                .with_extend(extend(e))
                .into(),
            Brush::SweepGradient { c0, start_angle, end_angle, color_stops, extend: e } => {
                peniko::Gradient::new_sweep(point(c0), start_angle.to_radians(), end_angle.to_radians())
                    .with_stops(stops(color_stops).as_slice())
                    .with_extend(extend(e))
                    .into()
            }
        };
        // Everything the image shows, in the paint's space.
        let xf = self.transform();
        let all = xf.inverse().transform_rect_bbox(Rect::new(0.0, 0.0, f64::from(self.width), f64::from(self.height)));
        self.ctx.set_transform(xf);
        match paint {
            peniko::Brush::Solid(c) => self.ctx.set_paint(c),
            peniko::Brush::Gradient(g) => self.ctx.set_paint(g),
            peniko::Brush::Image(_) => return,
        }
        self.ctx.fill_rect(&all);
    }

    fn push_layer(&mut self, mode: CompositeMode) {
        use peniko::{BlendMode as Mode, Compose, Mix};
        let (mix, compose) = match mode {
            CompositeMode::Clear => (Mix::Normal, Compose::Clear),
            CompositeMode::Src => (Mix::Normal, Compose::Copy),
            CompositeMode::Dest => (Mix::Normal, Compose::Dest),
            CompositeMode::DestOver => (Mix::Normal, Compose::DestOver),
            CompositeMode::SrcIn => (Mix::Normal, Compose::SrcIn),
            CompositeMode::DestIn => (Mix::Normal, Compose::DestIn),
            CompositeMode::SrcOut => (Mix::Normal, Compose::SrcOut),
            CompositeMode::DestOut => (Mix::Normal, Compose::DestOut),
            CompositeMode::SrcAtop => (Mix::Normal, Compose::SrcAtop),
            CompositeMode::DestAtop => (Mix::Normal, Compose::DestAtop),
            CompositeMode::Xor => (Mix::Normal, Compose::Xor),
            CompositeMode::Plus => (Mix::Normal, Compose::Plus),
            CompositeMode::Screen => (Mix::Screen, Compose::SrcOver),
            CompositeMode::Overlay => (Mix::Overlay, Compose::SrcOver),
            CompositeMode::Darken => (Mix::Darken, Compose::SrcOver),
            CompositeMode::Lighten => (Mix::Lighten, Compose::SrcOver),
            CompositeMode::ColorDodge => (Mix::ColorDodge, Compose::SrcOver),
            CompositeMode::ColorBurn => (Mix::ColorBurn, Compose::SrcOver),
            CompositeMode::HardLight => (Mix::HardLight, Compose::SrcOver),
            CompositeMode::SoftLight => (Mix::SoftLight, Compose::SrcOver),
            CompositeMode::Difference => (Mix::Difference, Compose::SrcOver),
            CompositeMode::Exclusion => (Mix::Exclusion, Compose::SrcOver),
            CompositeMode::Multiply => (Mix::Multiply, Compose::SrcOver),
            CompositeMode::HslHue => (Mix::Hue, Compose::SrcOver),
            CompositeMode::HslSaturation => (Mix::Saturation, Compose::SrcOver),
            CompositeMode::HslColor => (Mix::Color, Compose::SrcOver),
            CompositeMode::HslLuminosity => (Mix::Luminosity, Compose::SrcOver),
            _ => (Mix::Normal, Compose::SrcOver),
        };
        self.ctx.push_layer(None, Some(Mode::new(mix, compose)), None, None, None);
    }

    fn pop_layer(&mut self) {
        self.ctx.pop_layer();
    }
}
