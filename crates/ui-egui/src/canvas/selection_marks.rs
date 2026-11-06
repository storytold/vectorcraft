//! The selected objects' outlines, anchors and handles, kept between frames as triangles.
//!
//! Each selected object has an [`ObjectMarks`], valid for one node (by [`Arc`] identity), layer
//! color and set of selected anchors. The marks of all objects are joined into one mesh, in the
//! order they are drawn. A frame with nothing changed draws that mesh again. A pan moves the
//! triangles of outlines and handles, and places the anchor squares again, because a square's edges
//! are rounded to whole pixels where it lands. A frame with a new zoom, rotation or display scale
//! draws plain shapes and keeps nothing, and the next frame with the same view builds the marks.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Arc, Weak};

use egui::emath::{GuiRounding as _, TSTransform};
use egui::epaint::{ClippedShape, Primitive, RectShape, TessellationOptions, Tessellator, Vertex};
use egui::{Mesh, Vec2};
use vectorcraft_doc::{AnchorRef, NodeId, SegmentRef};

use super::outline::{KEY_OUTLINE, c32, node_outline, to_screen_path, walk_drawn};
use super::overlays::{HandleLook, anchor_square, selected_anchor};
use super::{BezPath, Color32, Node, NodeKind, Point, Pos2, Shape, Stroke, StrokeKind, SymbolBoxes, Xf, pos2, vec2};

/// How far a pan moves kept triangles before their object is built again. Screen coordinates are
/// `f32`, which holds a coordinate under 2^16 to within 1/256 px, and a point farther out than this
/// plus the width of a window stays off screen.
const MAX_PAN: f64 = 16_384.0;

/// The most points and vertices kept. Past this the marks are dropped after the frame that built
/// them.
const MAX_KEPT: usize = 8_000_000;

/// The most anchor square meshes kept, one per color, style and size in pixels.
const MAX_TEMPLATES: usize = 256;

/// The farthest a handle's dot and its stroke reach from the end of its line, at the default size.
const HANDLE_REACH: f32 = 6.0;

/// Everything that decides how marks look, apart from the object and the view's position.
#[derive(Clone, PartialEq)]
struct MarkLook {
    zoom: f64,
    rot: f64,
    pixels_per_point: f32,
    tessellation: TessellationOptions,
    /// Disc texture coordinates scale with the size of the font texture.
    font_image_size: [usize; 2],
    direct: bool,
    curvature: bool,
    curve_points_from_tool: bool,
    grow: f32,
    handle_style: String,
    segments: BTreeMap<NodeId, BTreeSet<SegmentRef>>,
}

/// The options [`SelectionMarks::paint`] draws with.
#[derive(Clone, Copy)]
pub(super) struct MarkOptions<'a> {
    /// Whether the tool edits anchors.
    pub direct: bool,
    /// Whether the tool is the Curvature tool.
    pub curvature: bool,
    /// Whether the Curvature tool draws its own points.
    pub curve_points_from_tool: bool,
    /// Selection & Anchor Display › Size and Handles.
    pub look: HandleLook<'a>,
    /// Selection & Anchor Display › Show handles when multiple anchors are selected.
    pub all_handles: bool,
}

/// [`Xf`] with its rotation worked out, split into a linear part and an offset.
struct ViewMap {
    sin: f64,
    cos: f64,
    zoom: f64,
    origin: Pos2,
    center: [f64; 2],
    offset: [f64; 2],
}

impl ViewMap {
    fn new(xf: &Xf) -> Self {
        let (sin, cos) = xf.rot.sin_cos();
        let origin = xf.rect.center();
        let mut map = Self { sin, cos, zoom: xf.zoom, origin, center: [0.0; 2], offset: [0.0; 2] };
        map.center = map.view(xf.center);
        map.offset = [f64::from(origin.x) - map.center[0], f64::from(origin.y) - map.center[1]];
        map
    }

    /// `p` scaled and rotated by the view, without its position.
    fn view(&self, p: Point) -> [f64; 2] {
        let (x, y) = (p.x * self.zoom, p.y * self.zoom);
        [x * self.cos - y * self.sin, x * self.sin + y * self.cos]
    }

    fn screen(&self, v: [f64; 2]) -> Pos2 {
        pos2(self.origin.x + (v[0] - self.center[0]) as f32, self.origin.y + (v[1] - self.center[1]) as f32)
    }

    fn to_screen(&self, p: Point) -> Pos2 {
        self.screen(self.view(p))
    }
}

/// One run of an object's marks.
enum Mark {
    /// Marks whose triangles keep their form when moved: polylines, discs and slanted lines.
    /// `shapes` are in screen coordinates at the object's `base` and become `mesh` the first time
    /// the run is on screen. Discs need the frame's own tessellator (`discs`).
    Rigid { shapes: Vec<Shape>, mesh: Mesh, bounds: egui::Rect, points: usize, discs: bool },
    /// Direction handles of one color, from [`ViewMap::view`]: the line from each anchor and the dot
    /// at its end. They become `mesh` for the offset `at` the first time the run is on screen. With a
    /// line that is, or is nearly, level or upright (`rounded`), `mesh` moves only by whole pixels,
    /// and any other move draws the run again.
    Handles { color: Color32, lines: Vec<[[f64; 2]; 2]>, rounded: bool, mesh: Mesh, at: [f64; 2], bounds: ([f64; 2], [f64; 2]) },
    /// Anchor squares in one style, at positions from [`ViewMap::view`].
    Squares { color: Color32, filled: bool, size: f32, at: Vec<[f64; 2]> },
    /// A line that is, or is nearly, level or upright: the tessellator rounds those to whole pixels.
    Segment { a: [f64; 2], b: [f64; 2], stroke: Stroke },
}

impl Mark {
    fn kept(&self) -> usize {
        match self {
            Self::Rigid { mesh, points, .. } => mesh.vertices.len().max(*points),
            Self::Handles { lines, mesh, .. } => mesh.vertices.len() + lines.len(),
            Self::Squares { at, .. } => at.len(),
            Self::Segment { .. } => 1,
        }
    }
}

/// Builds a list of [`Mark`]s, joining neighbors that can be drawn as one run.
struct MarkList<'a> {
    map: &'a ViewMap,
    marks: Vec<Mark>,
}

impl<'a> MarkList<'a> {
    fn new(map: &'a ViewMap) -> Self {
        Self { map, marks: vec![] }
    }

    fn rigid(&mut self, shape: Shape) {
        let rect = shape.visual_bounding_rect();
        let count = match &shape {
            Shape::Path(path) => path.points.len(),
            _ => 1,
        };
        let disc = matches!(shape, Shape::Circle(_));
        if let Some(Mark::Rigid { shapes, bounds, points, discs, .. }) = self.marks.last_mut() {
            *bounds = bounds.union(rect);
            *points += count;
            *discs |= disc;
            shapes.push(shape);
        } else {
            self.marks.push(Mark::Rigid { shapes: vec![shape], mesh: Mesh::default(), bounds: rect, points: count, discs: disc });
        }
    }

    /// A direction handle: its line from `from` and the dot at `to`.
    fn handle(&mut self, from: Point, to: Point, color: Color32) {
        let (a, b) = (self.map.view(from), self.map.view(to));
        let is_rounded = is_rounded_line(self.map.screen(a), self.map.screen(b));
        let least = [a[0].min(b[0]), a[1].min(b[1])];
        let most = [a[0].max(b[0]), a[1].max(b[1])];
        if let Some(Mark::Handles { color: c, lines, rounded, bounds, .. }) = self.marks.last_mut()
            && *c == color
        {
            *bounds = ([bounds.0[0].min(least[0]), bounds.0[1].min(least[1])], [bounds.1[0].max(most[0]), bounds.1[1].max(most[1])]);
            *rounded |= is_rounded;
            lines.push([a, b]);
        } else {
            self.marks.push(Mark::Handles {
                color,
                lines: vec![[a, b]],
                rounded: is_rounded,
                mesh: Mesh::default(),
                at: [0.0; 2],
                bounds: (least, most),
            });
        }
    }

    fn outline(&mut self, path: &BezPath, xf: &Xf, stroke: Stroke) {
        for line in to_screen_path(path, xf) {
            self.rigid(Shape::line(line, stroke));
        }
    }

    fn square(&mut self, p: Point, color: Color32, filled: bool, size: f32) {
        let v = self.map.view(p);
        if let Some(Mark::Squares { color: c, filled: f, size: s, at }) = self.marks.last_mut()
            && (*c, *f, *s) == (color, filled, size)
        {
            at.push(v);
        } else {
            self.marks.push(Mark::Squares { color, filled, size, at: vec![v] });
        }
    }

    fn line(&mut self, from: Point, to: Point, stroke: Stroke) {
        let (a, b) = (self.map.view(from), self.map.view(to));
        let (sa, sb) = (self.map.screen(a), self.map.screen(b));
        if is_rounded_line(sa, sb) {
            self.marks.push(Mark::Segment { a, b, stroke });
        } else {
            self.rigid(Shape::line_segment([sa, sb], stroke));
        }
    }
}

/// Whether a line from `a` to `b` is, or after a move could be, exactly level or upright.
fn is_rounded_line(a: Pos2, b: Pos2) -> bool {
    (a.x - b.x).abs() < 0.002 || (a.y - b.y).abs() < 0.002
}

/// The marks of one object (or the key object's outline), and what they were built from.
struct ObjectMarks {
    /// Holds the allocation, so that no later node can have this address. While it exists,
    /// `Arc::make_mut` moves an edited node to a new address.
    node: Weak<Node>,
    color: Color32,
    partial: Option<BTreeSet<AnchorRef>>,
    /// The document and revision, for an object that measures symbols' art.
    symbols: Option<(u64, u64)>,
    /// [`ViewMap::offset`] when the marks were built.
    base: [f64; 2],
    marks: Vec<Mark>,
    handles: Vec<Mark>,
    /// The anchors whose handles `handles` shows.
    with_handles: usize,
    seen: u64,
}

impl ObjectMarks {
    fn current(&self, node: &Arc<Node>, color: Color32, partial: Option<&BTreeSet<AnchorRef>>, source: (u64, u64), map: &ViewMap) -> bool {
        Weak::as_ptr(&self.node) == Arc::as_ptr(node)
            && self.color == color
            && self.partial.as_ref() == partial
            && self.symbols.is_none_or(|s| s == source)
            && (map.offset[0] - self.base[0]).abs() <= MAX_PAN
            && (map.offset[1] - self.base[1]).abs() <= MAX_PAN
    }
}

/// The node `id` as held in the document, for its identity.
fn node_arc(doc: &vectorcraft_doc::Document, id: NodeId) -> Option<&Arc<Node>> {
    let path = doc.index_path(id)?;
    let (first, rest) = path.split_first()?;
    let mut node = doc.layers.get(*first)?;
    for i in rest {
        node = node.children()?.get(*i)?;
    }
    Some(node)
}

/// A square's triangles, relative to the corner it was rounded to.
struct Template {
    color: Color32,
    filled: bool,
    size: f32,
    pixels: (i32, i32),
    mesh: Mesh,
}

/// A handle's dot in one color, around the origin.
struct Dot {
    color: Color32,
    mesh: Mesh,
}

/// What one anchor square draws: the fill, then the stroke of a hollow one.
fn square_shapes(rect: egui::Rect, color: Color32, filled: bool) -> [Option<RectShape>; 2] {
    if filled {
        [Some(RectShape::filled(rect, 0.0, color)), None]
    } else {
        [Some(RectShape::filled(rect, 0.0, Color32::WHITE)), Some(RectShape::stroke(rect, 0.0, Stroke::new(1.0, color), StrokeKind::Inside))]
    }
}

/// `shapes` as the frame's tessellator draws them, with nothing culled.
fn tessellated(ctx: &egui::Context, shapes: Vec<Shape>, pixels_per_point: f32) -> Mesh {
    let clipped = shapes.into_iter().map(|shape| ClippedShape {
        clip_rect: egui::Rect::EVERYTHING,
        shape,
        transform_after_tessellation: TSTransform::IDENTITY,
    });
    let primitives = ctx.tessellate(clipped.collect(), pixels_per_point);
    primitives
        .into_iter()
        .find_map(|c| match c.primitive {
            Primitive::Mesh(mesh) => Some(mesh),
            Primitive::Callback(_) => None,
        })
        .unwrap_or_default()
}

/// Joins marks into the frame's mesh.
struct Composer<'a> {
    ctx: &'a egui::Context,
    map: &'a ViewMap,
    clip: egui::Rect,
    pixels_per_point: f32,
    /// Squares this size or smaller are drawn as lines, which round differently: see
    /// [`Tessellator::tessellate_rect`].
    thin: f32,
    /// Draws lines and squares that rounding keeps from being reused, culled by `clip`.
    tessellator: Tessellator,
    /// Draws what is kept for later frames, with nothing culled. Like `tessellator`, it has no discs,
    /// so neither draws circles.
    keeper: Tessellator,
    /// Draws filled squares without the soft edge, when that edge falls between pixel centers: on a
    /// square rounded to whole pixels, each pixel center is then fully in or fully out.
    crisp: Option<Tessellator>,
    /// Whether the tessellator rounds rectangles to whole pixels, as [`Self::squares`] assumes.
    rounds_rects: bool,
    look: HandleLook<'a>,
    templates: &'a mut Vec<Template>,
    dots: &'a mut Vec<Dot>,
    out: Mesh,
}

/// Append `mesh` to `out`, moved by `delta`.
fn stamp(out: &mut Mesh, mesh: &Mesh, delta: Vec2) {
    let first = out.vertices.len() as u32;
    out.vertices.extend(mesh.vertices.iter().map(|v| Vertex { pos: v.pos + delta, ..*v }));
    out.indices.extend(mesh.indices.iter().map(|i| i + first));
}

impl Composer<'_> {
    fn marks(&mut self, marks: &mut [Mark], base: [f64; 2]) {
        let delta = vec2((self.map.offset[0] - base[0]) as f32, (self.map.offset[1] - base[1]) as f32);
        let margin = 2.0 / self.pixels_per_point;
        for mark in marks {
            match mark {
                Mark::Rigid { shapes, mesh, bounds, discs, .. } => {
                    if !bounds.translate(delta).expand(margin).intersects(self.clip) {
                        continue;
                    }
                    if !shapes.is_empty() {
                        let shapes = std::mem::take(shapes);
                        if *discs {
                            *mesh = tessellated(self.ctx, shapes, self.pixels_per_point);
                        } else {
                            for shape in shapes {
                                self.keeper.tessellate_shape(shape, mesh);
                            }
                        }
                    }
                    stamp(&mut self.out, mesh, delta);
                }
                Mark::Handles { color, lines, rounded, mesh, at, bounds } => {
                    let onscreen = egui::Rect::from_two_pos(self.map.screen(bounds.0), self.map.screen(bounds.1))
                        .expand(HANDLE_REACH + self.look.grow.max(0.0) / 2.0);
                    if !onscreen.intersects(self.clip) {
                        continue;
                    }
                    let moved = vec2((self.map.offset[0] - at[0]) as f32, (self.map.offset[1] - at[1]) as f32);
                    let whole_pixels = (moved * self.pixels_per_point).round() == moved * self.pixels_per_point;
                    if *rounded && !mesh.is_empty() && !whole_pixels {
                        let mut out = std::mem::take(&mut self.out);
                        self.handles(*color, lines, &mut out);
                        self.out = out;
                        continue;
                    }
                    if mesh.is_empty() {
                        self.handles(*color, lines, mesh);
                        *at = self.map.offset;
                        stamp(&mut self.out, mesh, Vec2::ZERO);
                    } else {
                        stamp(&mut self.out, mesh, moved);
                    }
                }
                Mark::Squares { color, filled, size, at } => self.squares(*color, *filled, *size, at),
                Mark::Segment { a, b, stroke } => {
                    let points = [self.map.screen(*a), self.map.screen(*b)];
                    self.tessellator.tessellate_line_segment(points, *stroke, &mut self.out);
                }
            }
        }
    }

    /// Draw direction handles into `mesh`.
    fn handles(&mut self, color: Color32, lines: &[[[f64; 2]; 2]], mesh: &mut Mesh) {
        let index = match self.dots.iter().position(|d| d.color == color) {
            Some(index) => index,
            None => {
                let shapes = self.look.shapes(Pos2::ZERO, color).into_iter().flatten().collect();
                let mesh = tessellated(self.ctx, shapes, self.pixels_per_point);
                if self.dots.len() >= MAX_TEMPLATES {
                    self.dots.clear();
                }
                self.dots.push(Dot { color, mesh });
                self.dots.len() - 1
            }
        };
        let Some(dot) = self.dots.get(index) else { return };
        let stroke = Stroke::new(1.0, color);
        for [from, to] in lines {
            let (from, to) = (self.map.screen(*from), self.map.screen(*to));
            self.keeper.tessellate_line_segment([from, to], stroke, mesh);
            stamp(mesh, &dot.mesh, to.to_vec2());
        }
    }

    fn squares(&mut self, color: Color32, filled: bool, size: f32, at: &[[f64; 2]]) {
        let stamps = self.rounds_rects && size > self.thin;
        let mut template = None;
        let mut last = None;
        for v in at {
            let rect = egui::Rect::from_center_size(self.map.screen(*v), vec2(size, size));
            if !rect.intersects(self.clip) {
                continue;
            }
            if !stamps {
                for shape in square_shapes(rect, color, filled).into_iter().flatten() {
                    self.tessellator.tessellate_rect(&shape, &mut self.out);
                }
                continue;
            }
            let rounded = rect.round_to_pixels(self.pixels_per_point);
            // A filled square on the same pixels as the last one, in the same run, adds nothing.
            if filled && last == Some(rounded) {
                continue;
            }
            last = Some(rounded);
            let pixels = ((rounded.width() * self.pixels_per_point).round() as i32, (rounded.height() * self.pixels_per_point).round() as i32);
            let index = match template {
                Some((p, index)) if p == pixels => index,
                _ => self.template(color, filled, size, pixels, rect),
            };
            template = Some((pixels, index));
            let Some(found) = self.templates.get(index) else { continue };
            stamp(&mut self.out, &found.mesh, rounded.min.to_vec2());
        }
    }

    /// The index of the template for a square in this style, made from `rect` when there is none.
    fn template(&mut self, color: Color32, filled: bool, size: f32, pixels: (i32, i32), rect: egui::Rect) -> usize {
        let same = |t: &Template| t.color == color && t.filled == filled && t.size == size && t.pixels == pixels;
        if let Some(index) = self.templates.iter().position(same) {
            return index;
        }
        let mut mesh = Mesh::default();
        let tessellator = match &mut self.crisp {
            Some(crisp) if filled => crisp,
            _ => &mut self.keeper,
        };
        for shape in square_shapes(rect, color, filled).into_iter().flatten() {
            tessellator.tessellate_rect(&shape, &mut mesh);
        }
        let corner = rect.round_to_pixels(self.pixels_per_point).min.to_vec2();
        for v in &mut mesh.vertices {
            v.pos -= corner;
        }
        if self.templates.len() >= MAX_TEMPLATES {
            self.templates.clear();
        }
        self.templates.push(Template { color, filled, size, pixels, mesh });
        self.templates.len() - 1
    }
}

/// What the frame's mesh was joined for.
#[derive(PartialEq)]
struct Shown {
    offset: [f64; 2],
    clip: egui::Rect,
    all_handles: bool,
}

/// The kept marks of the selection ([`CanvasCache::selection_marks`](crate::caches::CanvasCache)).
#[derive(Default)]
pub struct SelectionMarks {
    look: Option<MarkLook>,
    objects: HashMap<NodeId, ObjectMarks>,
    key: Option<(NodeId, ObjectMarks)>,
    templates: Vec<Template>,
    dots: Vec<Dot>,
    frame: Arc<Mesh>,
    shown: Option<Shown>,
    order: Vec<NodeId>,
    pass: u64,
    kept: usize,
    #[cfg(test)]
    builds: usize,
    #[cfg(test)]
    composes: usize,
}

impl SelectionMarks {
    /// Draw the key object `key`'s outline and the marks of `objects`, in that order, reusing what
    /// is still valid. Entries of any other object are dropped.
    pub(super) fn paint(
        &mut self,
        st: &vectorcraft_engine::DocState,
        p: &egui::Painter,
        xf: &Xf,
        options: &MarkOptions,
        key: Option<NodeId>,
        objects: &[NodeId],
    ) {
        let ctx = p.ctx();
        let look = MarkLook {
            zoom: xf.zoom,
            rot: xf.rot,
            pixels_per_point: ctx.pixels_per_point(),
            tessellation: ctx.tessellation_options(|o| *o),
            font_image_size: ctx.fonts(|f| f.font_image_size()),
            direct: options.direct,
            curvature: options.curvature,
            curve_points_from_tool: options.curve_points_from_tool,
            grow: options.look.grow,
            handle_style: options.look.style.to_owned(),
            segments: st.selection.segments.clone(),
        };
        let pixels_per_point = look.pixels_per_point;
        let tessellation = look.tessellation;
        if self.look.as_ref() != Some(&look) {
            self.clear();
            self.look = Some(look);
            draw_once(st, p, xf, options, key, objects);
            return;
        }
        let map = ViewMap::new(xf);
        let symbols = st.doc.symbol_boxes();
        let source = (st.uid, st.revision);
        let mut changed = self.update_key(st, key, xf, &map, &symbols, source);
        self.pass = self.pass.wrapping_add(1);
        let pass = self.pass;
        for id in objects {
            let Some(node) = node_arc(&st.doc, *id) else { continue };
            let color = c32(st.doc.layer_color(*id));
            match self.objects.get_mut(id) {
                Some(entry) if entry.current(node, color, st.selection.partial(*id), source, &map) => entry.seen = pass,
                _ => {
                    let mut entry = object_marks(st, *id, node, color, source, xf, &map, options, &symbols);
                    entry.seen = pass;
                    self.objects.insert(*id, entry);
                    changed = true;
                    #[cfg(test)]
                    {
                        self.builds += 1;
                    }
                }
            }
        }
        self.objects.retain(|_, entry| entry.seen == pass);
        let shown = Shown { offset: map.offset, clip: p.clip_rect(), all_handles: options.all_handles };
        if changed || self.shown.as_ref() != Some(&shown) || self.order != objects {
            self.order.clear();
            self.order.extend_from_slice(objects);
            self.compose(ctx, &map, shown.clip, (pixels_per_point, tessellation), options);
            self.shown = Some(shown);
        }
        if !self.frame.is_empty() {
            p.add(Shape::Mesh(self.frame.clone()));
        }
        self.trim(MAX_KEPT);
    }

    /// Drop everything when more than `limit` points and vertices are kept.
    fn trim(&mut self, limit: usize) {
        if self.kept > limit {
            self.clear();
        }
    }

    /// Bring the key object's outline up to date. True when it changed.
    fn update_key(
        &mut self,
        st: &vectorcraft_engine::DocState,
        key: Option<NodeId>,
        xf: &Xf,
        map: &ViewMap,
        symbols: &SymbolBoxes,
        source: (u64, u64),
    ) -> bool {
        let Some((id, node)) = key.and_then(|id| Some((id, node_arc(&st.doc, id)?))) else { return self.key.take().is_some() };
        let color = c32(st.doc.layer_color(id));
        if self.key.as_ref().is_some_and(|(k, entry)| *k == id && entry.current(node, color, None, source, map)) {
            return false;
        }
        self.key = Some((id, key_marks(node, color, source, xf, map, symbols)));
        true
    }

    /// Join the marks in the order they are drawn: the key object's outline, each object's marks,
    /// and then all the handles.
    fn compose(
        &mut self,
        ctx: &egui::Context,
        map: &ViewMap,
        clip: egui::Rect,
        (pixels_per_point, tessellation): (f32, TessellationOptions),
        options: &MarkOptions,
    ) {
        #[cfg(test)]
        {
            self.composes += 1;
        }
        let Self { objects, key, templates, dots, frame, order, kept, .. } = self;
        let all_handles = options.all_handles;
        let mut out = Arc::try_unwrap(std::mem::take(frame)).unwrap_or_default();
        out.clear();
        let font_image_size = ctx.fonts(|f| f.font_image_size());
        let keeper = Tessellator::new(pixels_per_point, tessellation, font_image_size, vec![]);
        let mut tessellator = keeper.clone();
        tessellator.set_clip_rect(clip);
        let feathering = if tessellation.feathering { tessellation.feathering_size_in_pixels / pixels_per_point } else { 0.0 };
        let crisp = (tessellation.feathering && tessellation.feathering_size_in_pixels == 1.0)
            .then(|| Tessellator::new(pixels_per_point, TessellationOptions { feathering: false, ..tessellation }, font_image_size, vec![]));
        let mut composer = Composer {
            ctx,
            map,
            clip,
            pixels_per_point,
            thin: 2.0 * feathering,
            tessellator,
            keeper,
            crisp,
            rounds_rects: tessellation.round_rects_to_pixels,
            look: options.look,
            templates,
            dots,
            out,
        };
        if let Some((_, entry)) = key {
            composer.marks(&mut entry.marks, entry.base);
        }
        for id in order.iter() {
            if let Some(entry) = objects.get_mut(id) {
                composer.marks(&mut entry.marks, entry.base);
            }
        }
        let with_handles: usize = order.iter().filter_map(|id| objects.get(id)).map(|entry| entry.with_handles).sum();
        if all_handles || with_handles <= 1 {
            for id in order.iter() {
                if let Some(entry) = objects.get_mut(id) {
                    composer.marks(&mut entry.handles, entry.base);
                }
            }
        }
        let out = composer.out;
        let in_entries: usize = objects
            .values()
            .chain(key.iter().map(|(_, entry)| entry))
            .flat_map(|entry| entry.marks.iter().chain(&entry.handles))
            .map(Mark::kept)
            .sum();
        *kept = in_entries + out.vertices.len();
        *frame = Arc::new(out);
    }

    fn clear(&mut self) {
        *self = Self {
            #[cfg(test)]
            builds: self.builds,
            #[cfg(test)]
            composes: self.composes,
            ..Self::default()
        };
    }
}

/// Whether `n` or anything inside it is a symbol instance, whose outline is measured from art
/// outside the node.
fn uses_symbols(n: &Node) -> bool {
    let mut found = false;
    n.walk(&mut |c| found |= matches!(c.kind, NodeKind::SymbolInstance { .. }));
    found
}

/// Build the key object's outline.
fn key_marks(node: &Arc<Node>, color: Color32, source: (u64, u64), xf: &Xf, map: &ViewMap, symbols: &SymbolBoxes) -> ObjectMarks {
    let mut list = MarkList::new(map);
    list.outline(&node_outline(node, symbols), xf, Stroke::new(KEY_OUTLINE, color));
    ObjectMarks {
        node: Arc::downgrade(node),
        color,
        partial: None,
        symbols: uses_symbols(node).then_some(source),
        base: map.offset,
        marks: list.marks,
        handles: vec![],
        with_handles: 0,
        seen: 0,
    }
}

/// Draw the key object's outline and the marks of `objects` as plain shapes, keeping nothing.
fn draw_once(st: &vectorcraft_engine::DocState, p: &egui::Painter, xf: &Xf, options: &MarkOptions, key: Option<NodeId>, objects: &[NodeId]) {
    let map = ViewMap::new(xf);
    let symbols = st.doc.symbol_boxes();
    let source = (st.uid, st.revision);
    if let Some((id, node)) = key.and_then(|id| Some((id, node_arc(&st.doc, id)?))) {
        add_marks(p, &map, options.look, key_marks(node, c32(st.doc.layer_color(id)), source, xf, &map, &symbols).marks);
    }
    let (mut handles, mut with_handles) = (vec![], 0);
    for id in objects {
        let Some(node) = node_arc(&st.doc, *id) else { continue };
        let entry = object_marks(st, *id, node, c32(st.doc.layer_color(*id)), source, xf, &map, options, &symbols);
        add_marks(p, &map, options.look, entry.marks);
        handles.push(entry.handles);
        with_handles += entry.with_handles;
    }
    if options.all_handles || with_handles <= 1 {
        for marks in handles {
            add_marks(p, &map, options.look, marks);
        }
    }
}

/// Add `marks` to the painter one shape at a time.
fn add_marks(p: &egui::Painter, map: &ViewMap, look: HandleLook, marks: Vec<Mark>) {
    for mark in marks {
        match mark {
            Mark::Rigid { shapes, .. } => {
                for shape in shapes {
                    p.add(shape);
                }
            }
            Mark::Handles { color, lines, .. } => {
                for [from, to] in lines {
                    let (from, to) = (map.screen(from), map.screen(to));
                    p.line_segment([from, to], Stroke::new(1.0, color));
                    look.draw(p, to, color);
                }
            }
            Mark::Squares { color, filled, size, at } => {
                for v in at {
                    anchor_square(p, map.screen(v), color, filled, size);
                }
            }
            Mark::Segment { a, b, stroke } => {
                p.line_segment([map.screen(a), map.screen(b)], stroke);
            }
        }
    }
}

/// Build the marks of the selected object `id`.
#[allow(clippy::too_many_arguments)]
fn object_marks(
    st: &vectorcraft_engine::DocState,
    id: NodeId,
    node: &Arc<Node>,
    color: Color32,
    source: (u64, u64),
    xf: &Xf,
    map: &ViewMap,
    options: &MarkOptions,
    symbols: &SymbolBoxes,
) -> ObjectMarks {
    let MarkOptions { direct, curvature, curve_points_from_tool, look, .. } = *options;
    let n: &Node = node;
    let partial = st.selection.partial(id);
    let anchor = |direct: bool| look.grow + if direct { 5.0 } else { 4.0 };
    let (mut marks, mut handles, mut with_handles) = (MarkList::new(map), MarkList::new(map), 0);
    // Path outlines (and area type / placed document / image / mesh / symbol outlines;
    // point type baselines and origin are drawn below).
    if !matches!(n.kind, NodeKind::Text(ref tx) if matches!(tx.kind, vectorcraft_doc::TextKind::Point)) {
        marks.outline(&node_outline(n, symbols), xf, Stroke::new(1.0, color));
    }
    // Anchors (and handles for selected anchors in direct mode); a gradient mesh's or a mesh
    // envelope's points.
    walk_drawn(n, &mut |c| {
        let envelope = vectorcraft_doc::live::envelope_overlay(c).and_then(|(_, grid)| grid);
        let grid = match &c.kind {
            NodeKind::Mesh(m) => Some(m),
            _ => envelope.as_ref(),
        };
        if let Some(grid) = grid {
            for q in &grid.points {
                marks.square(q.p, color, false, anchor(direct));
            }
            return;
        }
        let NodeKind::Path { path, .. } = &c.kind else { return };
        // The Curvature tool owns its points while drawing; existing paths use this overlay.
        if curvature && curve_points_from_tool {
            return;
        }
        let selected_segments = st.selection.segments.get(&c.id);
        if let Some(segments) = selected_segments {
            for &(si, seg) in segments {
                let Some(sp) = path.subpaths.get(si).filter(|sp| seg < sp.segment_count()) else { continue };
                let curve = sp.segment(seg);
                let mut outline = vectorcraft_geom::BezPath::new();
                outline.move_to(curve.p0);
                if sp.segment_is_line(seg) {
                    outline.line_to(curve.p3);
                } else {
                    outline.curve_to(curve.p1, curve.p2, curve.p3);
                }
                marks.outline(&outline, xf, Stroke::new(2.0, color));
            }
        }
        for (si, ai, a) in path.anchors() {
            // An object selected whole has all its anchors selected, and so do the paths inside a
            // group, compound path or clip group selected whole. Direction handles show only on a
            // path that is selected itself: those are the handles that `handle_anchors` in
            // `vectorcraft-tools` lets the tools drag.
            let (sel, handles_shown) = match partial {
                Some(set) => {
                    let sel = set.contains(&(si, ai));
                    (sel, sel)
                }
                None => (true, direct && c.id == id),
            };
            if !curvature && handles_shown {
                with_handles += 1;
                let sides = path.subpaths.get(si).map(|subpath| st.selection.handle_sides(c.id, si, ai, subpath)).unwrap_or((true, true));
                for (h, _) in [(a.h_in, sides.0), (a.h_out, sides.1)].into_iter().filter(|(h, show)| *show && h.distance(a.p) > 1e-6) {
                    handles.handle(a.p, h, color);
                }
            }
            let anchor_selected = sel && selected_segments.is_none();
            let ink = if anchor_selected && partial.is_some() { selected_anchor(color) } else { color };
            if curvature {
                let (fill, size) = (if anchor_selected && partial.is_some() { ink } else { Color32::WHITE }, look.grow + 6.0);
                marks.rigid(Shape::Circle(egui::epaint::CircleShape {
                    center: map.to_screen(a.p),
                    radius: size / 2.0,
                    fill,
                    stroke: Stroke::new(1.0, ink),
                }));
            } else {
                marks.square(a.p, ink, anchor_selected, anchor(partial.is_some() || direct));
            }
        }
    });
    // Centre point (Attributes panel → Show Center).
    if n.shows_center()
        && let Some(b) = n.geometric_bounds_with(symbols)
    {
        marks.square(b.center(), color, true, anchor(false));
    }
    // Point type: each line's baseline, where a click selects it (`hit`), whatever its
    // alignment and rotation, and the anchor (area type shows its frame instead, which may
    // not be a rectangle, and type on a path its path).
    if let NodeKind::Text(tx) = &n.kind
        && matches!(tx.kind, vectorcraft_doc::TextKind::Point)
    {
        for (a, b) in tx.baselines() {
            marks.line(tx.xf * a, tx.xf * b, Stroke::new(1.0, color));
        }
        marks.rigid(Shape::circle_filled(map.to_screen(tx.xf * Point::ZERO), 2.5, color));
    }
    ObjectMarks {
        node: Arc::downgrade(node),
        color,
        partial: partial.cloned(),
        symbols: uses_symbols(n).then_some(source),
        base: map.offset,
        marks: marks.marks,
        handles: handles.marks,
        with_handles,
        seen: 0,
    }
}

#[cfg(test)]
mod tests;
