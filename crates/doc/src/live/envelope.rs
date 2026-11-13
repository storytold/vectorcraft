use super::expand::{Hooks, Outliner, Warper, map_node};
use super::mesh::{GradientMesh, MeshPoint, grid_len, grid_points, surface_of};
use super::{Affine, Arc, Color, Deserialize, Node, NodeId, NodeKind, PathData, Point, Rect, Serialize, Vec2, yes};
use vectorcraft_geom::Homography;

fn is_zero(n: &u32) -> bool {
    *n == 0
}

/// How an envelope distorts its content.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum EnvelopeKind {
    /// One of the 15 warp styles (`arc`, `arcLower`, … — see [`WarpStyle::from_id`]); bend and
    /// distortions in percent (-100..100).
    Warp {
        style: String,
        #[serde(default)]
        bend: f64,
        #[serde(default)]
        h: f64,
        #[serde(default)]
        v: f64,
        #[serde(default = "yes")]
        horizontal: bool,
        /// The map the warp draws with: [`WARP_NET`](super::WARP_NET) for its style's Bézier net,
        /// 0 (as in files from before the version existed) for [`warp_point`](super::warp_point).
        #[serde(default, skip_serializing_if = "is_zero")]
        version: u32,
    },
    /// A `(rows+1)×(cols+1)` grid of points (row-major) the content's bounding box maps onto.
    Mesh {
        rows: u32,
        cols: u32,
        points: Vec<Point>,
        /// Each point's bezier handles (offsets, see [`H_RIGHT`] …), row-major like `points`.
        /// Empty: the smooth (Catmull-Rom) mesh through the points, as before handles existed;
        /// editing the mesh's points or handles on the canvas fills them in.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        handles: Vec<[Vec2; 4]>,
    },
    /// The content's bounding box maps onto the outline of this path.
    TopObject {
        path: PathData,
        /// The anchors (indices into the outline, the path's largest subpath, in its own order)
        /// at the top left, top right, bottom right and bottom left corners, as a saved file gives
        /// them. `None`: the corners are found from the outline.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        corners: Option<[u32; 4]>,
    },
}

/// Envelope Options → Preserve Shape Using: how a distorted raster keeps the envelope's shape.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PreserveShape {
    #[default]
    ClippingMask,
    Transparency,
}

impl PreserveShape {
    pub fn id(self) -> &'static str {
        match self {
            Self::ClippingMask => "clippingMask",
            Self::Transparency => "transparency",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "clippingMask" => Some(Self::ClippingMask),
            "transparency" => Some(Self::Transparency),
            _ => None,
        }
    }
}

/// Envelope Options besides Fidelity. `Default` is what envelopes saved before these options
/// existed do (their appearance applies after the distortion); new envelopes start from
/// [`EnvelopeOptions::NEW`], as in the reference app.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct EnvelopeOptions {
    /// Rasters: Anti-Alias.
    pub anti_alias: bool,
    /// Rasters: Preserve Shape Using.
    pub preserve_shape: PreserveShape,
    /// Distort Appearance: strokes and live effects are applied before the distortion, so they
    /// bend with the art (off: they apply to the distorted paths). Type's strokes are the exception:
    /// they paint along its distorted glyph outlines at their own weight either way, as `.ai` files
    /// draw them (#6644).
    pub distort_appearance: bool,
    /// Distort Linear Gradients (with Distort Appearance): placed gradients bend with the art.
    pub distort_linear_gradients: bool,
    /// Distort Pattern Fills (with Distort Appearance): pattern tiles bend with the art.
    pub distort_pattern_fills: bool,
    /// With Distort Appearance, how far the box the content maps from reaches past its geometric
    /// bounds at the top, bottom, left and right, as shares of their height and width. An envelope
    /// opened from a `.ai` whose saved size includes its strokes has these margins. New envelopes
    /// have none. Leaving Edit Contents measures them again when the strokes reach differently.
    #[serde(skip_serializing_if = "crate::skip::is_default")]
    pub bounds_margin: [f32; 4],
}

impl Default for EnvelopeOptions {
    fn default() -> Self {
        Self {
            anti_alias: true,
            preserve_shape: PreserveShape::ClippingMask,
            distort_appearance: false,
            distort_linear_gradients: false,
            distort_pattern_fills: false,
            bounds_margin: [0.0; 4],
        }
    }
}

impl EnvelopeOptions {
    /// The options a new envelope gets (the reference app's defaults).
    pub const NEW: Self = Self {
        anti_alias: true,
        preserve_shape: PreserveShape::ClippingMask,
        distort_appearance: true,
        distort_linear_gradients: false,
        distort_pattern_fills: false,
        bounds_margin: [0.0; 4],
    };
    /// Do gradients bend with the art?
    pub fn gradients(&self) -> bool {
        self.distort_appearance && self.distort_linear_gradients
    }
    /// Do pattern tiles bend with the art?
    pub fn patterns(&self) -> bool {
        self.distort_appearance && self.distort_pattern_fills
    }
    /// The box content with geometric bounds `r` maps from. Margins that aren't finite, are
    /// negative or give a box that isn't finite count as none.
    fn source_box(&self, r: Rect) -> Rect {
        let [top, bottom, left, right] = self.bounds_margin.map(f64::from);
        if !self.distort_appearance || !self.bounds_margin.iter().all(|m| m.is_finite() && *m >= 0.0) {
            return r;
        }
        let grown = crate::marks::outset(r, [top * r.height(), bottom * r.height(), left * r.width(), right * r.width()]);
        if grown.is_finite() { grown } else { r }
    }
}

/// The bounds of `content` in the axes of `frame`, with its strokes and geometric, type measured by
/// its glyph outlines (see [`envelope_content_bounds`]). Strokes scale with the frame as
/// [`Node::transform`] scales them.
pub fn envelope_content_boxes(content: &[Arc<Node>], frame: Affine) -> Option<(Rect, Rect)> {
    envelope_content_boxes_with(content, frame, false).or_else(|| envelope_content_boxes_with(content, frame, true))
}

/// [`envelope_content_boxes`] with type that draws nothing given its laid-out box when
/// `blank_type_box` is set and no box when it is not. A caller that measures the members of an
/// envelope's contents one at a time passes `true` only when the whole contents have no box with
/// `false`, so each member is measured as the whole contents measure it.
pub fn envelope_content_boxes_with(content: &[Arc<Node>], frame: Affine, blank_type_box: bool) -> Option<(Rect, Rect)> {
    let frame = if frame.is_finite() && frame.determinant().abs() > 1e-12 { frame } else { Affine::IDENTITY };
    let to_frame = frame.inverse();
    Some((content_set_box(content, true, to_frame, blank_type_box)?, content_set_box(content, false, to_frame, blank_type_box)?))
}

/// The geometric bounds an envelope around `content` on the page axes maps from: type, alone or
/// in groups and layers, is measured by its glyph outlines, underlines and strikethroughs instead
/// of its laid-out box. Type keeps its laid-out box while nothing is registered with
/// [`set_type_ink`]. Type that draws nothing adds no box while other content has one; when
/// nothing else has a box, it keeps its laid-out box.
pub fn envelope_content_bounds(content: &[Arc<Node>]) -> Option<Rect> {
    content_set_box(content, false, Affine::IDENTITY, false).or_else(|| content_set_box(content, false, Affine::IDENTITY, true))
}

/// The union of the boxes [`content_box`] gives each of `content`.
fn content_set_box(content: &[Arc<Node>], stroked: bool, to: Affine, blank_type_box: bool) -> Option<Rect> {
    content.iter().fold(None, |acc, c| vectorcraft_geom::union_opt(acc, content_box(c, stroked, to, blank_type_box)))
}

/// The box of type's glyph outlines, underlines and strikethroughs with its text space mapped by
/// the given transform (outlines mapped point by point) and, for type in perspective, then drawn
/// through its projection and mapped by the second transform. `None` when it draws nothing. Type is
/// laid out above this crate, so the text crate registers one with [`set_type_ink`].
pub type TypeInk = fn(&crate::TextObject, Affine, Option<(Homography, Affine)>) -> Option<Rect>;

/// What [`TypeInk`] maps type's text space by: a transform, and a projection with the transform
/// after it.
type TypeMap = (Affine, Option<(Homography, Affine)>);

static TYPE_INK: std::sync::OnceLock<(TypeInk, crate::hit::LiveInputs)> = std::sync::OnceLock::new();

/// Register how envelopes measure type, and a number that changes whenever what it reads besides
/// the type changes, such as the installed fonts (the first registration stays).
pub fn set_type_ink(f: TypeInk, inputs: crate::hit::LiveInputs) {
    // Already set: the same measure, registered by another session, an import or a renderer.
    let _ = TYPE_INK.set((f, inputs));
}

/// How many entries each table of [`TypeInks`] holds before it is emptied.
const TYPE_INK_KEPT: usize = 1024;

/// `map`, bit for bit, for a table key.
fn map_bits(map: &TypeMap) -> [u64; 21] {
    let mut bits = [0; 21];
    let (m, projection) = map;
    for (b, v) in bits.iter_mut().zip(m.as_coeffs()) {
        *b = v.to_bits();
    }
    if let Some((h, after)) = projection {
        for (b, v) in bits[6..].iter_mut().zip(h.to_array().into_iter().chain(after.as_coeffs())) {
            *b = v.to_bits();
        }
    }
    bits
}

/// A key of [`TypeInks::nodes`]: the object's address, the map its outlines are measured in (an
/// affine map without its move) and the inputs number registered with [`set_type_ink`].
type NodeKey = (usize, [u64; 21], u64);

/// A key of [`TypeInks::content`]: two 64-bit hashes of the type without its transform's move and
/// its layout caches ([`content_hash`]), the map its outlines are measured in, and the inputs number.
type ContentKey = ([u64; 2], [u64; 21], u64);

/// Type measured by [`type_ink`] on one thread, by the map it is measured in and the inputs
/// number: by the object (an unchanged object is found without being read),
/// and by its content (a moved or copied object is found without laying it out again). Neither
/// table keeps a copy of the type.
#[derive(Default)]
struct TypeInks {
    /// By the object's address, with a weak pointer that keeps the address from being reused.
    nodes: std::collections::HashMap<NodeKey, (std::sync::Weak<Node>, Option<Rect>)>,
    /// By the type's content.
    content: std::collections::HashMap<ContentKey, Option<Rect>>,
}

/// Where [`TypeInks::find`] did not find type: its keys in both tables (no content key for type
/// that could not be hashed whole).
struct Missing {
    by_node: NodeKey,
    content: Option<ContentKey>,
}

impl TypeInks {
    /// The kept box of type node `n` (holding `t`) mapped by `map`, or the keys to keep it under
    /// once it is measured.
    fn find(&mut self, n: &Arc<Node>, t: &crate::TextObject, map: &TypeMap, inputs: u64) -> Result<Option<Rect>, Box<Missing>> {
        // While the table holds a weak pointer to `n`'s allocation, no other object can be at that
        // address, and `n` can't be changed in place (`Arc::get_mut` refuses it and `Arc::make_mut`
        // moves it).
        let by_node = (Arc::as_ptr(n) as usize, map_bits(map), inputs);
        if let Some((_, ink)) = self.nodes.get(&by_node) {
            return Ok(*ink);
        }
        // Shrink Text to Fit lays type out by its size on the page, so only a move keeps its layout.
        let [a, b, c, d, _, _] = t.xf.as_coeffs();
        let mut text = t.clone();
        text.xf = Affine::new([a, b, c, d, 0.0, 0.0]);
        text.cached_bounds = None;
        text.cached_baselines.clear();
        text.cached_ink = None;
        let content = content_hash(&text).map(|hash| (hash, by_node.1, inputs));
        match content.and_then(|key| self.content.get(&key)) {
            Some(ink) => {
                let ink = *ink;
                self.keep_node(n, by_node, ink);
                Ok(ink)
            }
            None => Err(Box::new(Missing { by_node, content })),
        }
    }

    /// Keep the box `ink` measured for type node `n` under the keys `missing`.
    fn keep(&mut self, n: &Arc<Node>, missing: Missing, ink: Option<Rect>) {
        if let Some(key) = missing.content {
            if self.content.len() >= TYPE_INK_KEPT {
                self.content.clear();
            }
            self.content.insert(key, ink);
        }
        self.keep_node(n, missing.by_node, ink);
    }

    fn keep_node(&mut self, n: &Arc<Node>, by_node: NodeKey, ink: Option<Rect>) {
        if self.nodes.len() >= TYPE_INK_KEPT {
            self.nodes.retain(|_, (weak, _)| weak.strong_count() > 0);
            if self.nodes.len() >= TYPE_INK_KEPT {
                self.nodes.clear();
            }
        }
        self.nodes.insert(by_node, (Arc::downgrade(n), ink));
    }
}

/// Two independent 64-bit hashes of `text` as saved (the layout caches are not saved), with what
/// layout reads that is not saved: whether its first paragraph continues a thread and the bounds
/// of its inline graphics. `None` when it can't be written whole.
fn content_hash(text: &crate::TextObject) -> Option<[u64; 2]> {
    use std::hash::Hasher;
    /// Feeds what is written to two hashers.
    struct Feed([std::collections::hash_map::DefaultHasher; 2]);
    impl std::io::Write for Feed {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            for h in &mut self.0 {
                h.write(buf);
            }
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut feed = Feed(Default::default());
    // The second hasher starts from another state.
    feed.0[1].write_u8(1);
    serde_json::to_writer(&mut feed, text).ok()?;
    let inline_bounds = text.runs.iter().filter_map(|r| r.inline.as_ref()).map(|i| i.bounds.map(|b| [b.x0, b.y0, b.x1, b.y1].map(f64::to_bits)));
    for h in &mut feed.0 {
        h.write_u8(u8::from(text.first_para_continues));
        for b in inline_bounds.clone() {
            match b {
                None => h.write_u8(0),
                Some(bits) => {
                    h.write_u8(1);
                    bits.into_iter().for_each(|v| h.write_u64(v));
                }
            }
        }
    }
    let [a, b] = feed.0;
    Some([a.finish(), b.finish()])
}

std::thread_local! {
    /// The type [`type_ink`] measured on this thread.
    static TYPE_INKS: std::cell::RefCell<TypeInks> = std::cell::RefCell::new(TypeInks::default());
}

/// The box of the glyph outlines of type node `n` (holding `t`) with its text space mapped by `m`
/// ([`set_type_ink`]) and, for type in perspective, drawn through `projection` and mapped by its
/// second transform; measured again when `t`, the maps (without a move of `m` when there is no
/// projection) or the registered inputs number change. `None` while nothing is registered.
fn type_ink(n: &Arc<Node>, t: &crate::TextObject, m: Affine, projection: Option<(Homography, Affine)>) -> Option<Option<Rect>> {
    let (f, inputs) = TYPE_INK.get()?;
    let inputs = inputs();
    // Without a projection the type is measured without the move, which is added to the box.
    let [a, b, c, d, e, g] = m.as_coeffs();
    let (map, offset) = match projection {
        None => ((Affine::new([a, b, c, d, 0.0, 0.0]), None), Vec2::new(e, g)),
        Some(p) => ((m, Some(p)), Vec2::ZERO),
    };
    let ink = match TYPE_INKS.with_borrow_mut(|kept| kept.find(n, t, &map, inputs)) {
        Ok(ink) => ink,
        Err(missing) => {
            // Measured outside the borrow: laying type out may measure other type.
            let ink = f(t, map.0, map.1);
            TYPE_INKS.with_borrow_mut(|kept| kept.keep(n, *missing, ink));
            ink
        }
    };
    Some(ink.map(|r| r + offset))
}

/// Add to `typed` the address of `n` and of each group and layer in it that holds type, which
/// [`content_box`] measures by its glyphs; is there type in `n`? One walk of the tree.
fn find_type(n: &Node, typed: &mut std::collections::HashSet<usize>) -> bool {
    let holds = match &n.kind {
        NodeKind::Text(_) => true,
        NodeKind::Layer { children, .. } | NodeKind::Group { children, .. } => children.iter().fold(false, |holds, c| find_type(c, typed) | holds),
        _ => false,
    };
    if holds {
        typed.insert(n as *const Node as usize);
    }
    holds
}

/// The box of `n` an envelope maps from, in the axes `to` takes the page to: its geometric bounds,
/// or with `stroked` its visual bounds (strokes scaled by `to` as [`Node::transform`] scales them),
/// with type measured by its glyph outlines ([`type_ink`]) instead of its laid-out box, in groups
/// and layers too. Type is laid out as it is on the page, and its outlines are then taken into
/// those axes. A character's strokes reach as far past its glyphs as past the laid-out box. Type
/// that draws nothing has its laid-out box with `blank_type_box`, and no box without it.
fn content_box(n: &Arc<Node>, stroked: bool, to: Affine, blank_type_box: bool) -> Option<Rect> {
    let mut typed = std::collections::HashSet::new();
    find_type(n, &mut typed);
    content_box_in(n, stroked, to, &typed, blank_type_box)
}

/// [`content_box`], with the addresses of the objects in `n` that hold type ([`find_type`]).
fn content_box_in(n: &Arc<Node>, stroked: bool, to: Affine, typed: &std::collections::HashSet<usize>, blank_type_box: bool) -> Option<Rect> {
    // `n` in the axes of `to`.
    let moved = || {
        if to == Affine::IDENTITY {
            return std::borrow::Cow::Borrowed(&**n);
        }
        let mut m = (**n).clone();
        m.transform(to, true);
        std::borrow::Cow::Owned(m)
    };
    let own = |m: &Node| if stroked { m.visual_bounds() } else { m.geometric_bounds() };
    if !typed.contains(&(Arc::as_ptr(n) as usize)) {
        return own(&moved());
    }
    match &n.kind {
        NodeKind::Text(t) => {
            // Type in perspective: its outlines as its projection draws them.
            let ink = match n.projection() {
                None => type_ink(n, t, to * t.xf, None),
                Some(h) => type_ink(n, t, t.xf, Some((h, to))),
            };
            let m = moved();
            let ink = match ink {
                Some(Some(ink)) => ink,
                Some(None) if !blank_type_box => return None,
                _ => return own(&m),
            };
            if !stroked {
                return Some(ink);
            }
            let (Some(v), Some(g)) = (m.visual_bounds(), m.geometric_bounds()) else { return Some(ink) };
            Some(Rect::new(
                ink.x0 - (g.x0 - v.x0).max(0.0),
                ink.y0 - (g.y0 - v.y0).max(0.0),
                ink.x1 + (v.x1 - g.x1).max(0.0),
                ink.y1 + (v.y1 - g.y1).max(0.0),
            ))
        }
        NodeKind::Group { children, clip: true } | NodeKind::Layer { children, clip: true, .. } => {
            // A clipping path that paints strokes over the clip edge, as in `Node::visual_bounds`.
            let clip = children.first()?;
            content_box_in(
                clip,
                stroked && matches!(clip.kind, NodeKind::Path { .. } | NodeKind::Compound { .. } | NodeKind::Text(_)),
                to,
                typed,
                blank_type_box,
            )
        }
        NodeKind::Layer { .. } | NodeKind::Group { .. } => {
            let layer = matches!(n.kind, NodeKind::Layer { .. });
            let members = n.editing_children().iter();
            if stroked {
                // The container's own strokes, scaled by `to` as `Node::transform` scales them,
                // without copying its members.
                let mut own_strokes = Node::new(n.id, NodeKind::Group { children: Vec::new(), clip: false });
                own_strokes.appearance = n.appearance.clone();
                own_strokes.transform(to, true);
                own_strokes.visual_of_members(members.map(|c| content_box_in(c, true, to, typed, blank_type_box)))
            } else {
                members
                    .filter(|c| c.visible || !layer)
                    .fold(None, |acc, c| vectorcraft_geom::union_opt(acc, content_box_in(c, false, to, typed, blank_type_box)))
            }
        }
        _ => own(&moved()),
    }
}

/// The [`EnvelopeOptions::bounds_margin`] of content with bounds `stroked` and `plain`. `None` for
/// content with no width or no height, or for margins an `f32` can't hold.
pub fn envelope_stroke_margin(stroked: Rect, plain: Rect) -> Option<[f32; 4]> {
    if plain.width() < 1e-6 || plain.height() < 1e-6 {
        return None;
    }
    let margin = [
        (plain.y0 - stroked.y0) / plain.height(),
        (stroked.y1 - plain.y1) / plain.height(),
        (plain.x0 - stroked.x0) / plain.width(),
        (stroked.x1 - plain.x1) / plain.width(),
    ]
    .map(|m| m as f32);
    margin.iter().all(|m| m.is_finite()).then_some(margin)
}

/// Handle slots of a [`MeshPoint`]: towards the next column, previous column, next row, previous row.
pub const H_RIGHT: usize = 0;

pub const H_LEFT: usize = 1;

pub const H_DOWN: usize = 2;

pub const H_UP: usize = 3;

/// The envelope's point map for content whose bounding box is `src` (page axes).
pub fn envelope_mapper<'a>(kind: &'a EnvelopeKind, src: Rect) -> Box<dyn Fn(Point) -> Point + 'a> {
    let s = surface_of(kind, src, Affine::IDENTITY);
    let (w, h) = (src.width().max(1e-9), src.height().max(1e-9));
    Box::new(move |q: Point| s((q.x - src.x0) / w, (q.y - src.y0) / h))
}

/// Catmull-Rom tangents of a `(rows+1)×(cols+1)` point grid (row-major) as mesh handles, a third
/// of the tangent towards each neighbour (past the borders the grid goes on straight): the curves
/// an envelope mesh without handles of its own runs along between its points.
pub fn smooth_handles(rows: u32, cols: u32, points: &[Point]) -> Vec<[Vec2; 4]> {
    let (rows, cols) = (rows as usize, cols as usize);
    let at = |r: usize, c: usize| points.get(r * (cols + 1) + c).copied().unwrap_or_default();
    let tangent = |prev: Option<Point>, p: Point, next: Option<Point>| match (prev, next) {
        (Some(a), Some(b)) => (b - a) / 2.0,
        (None, Some(b)) => b - p,
        (Some(a), None) => p - a,
        (None, None) => Vec2::ZERO,
    };
    let mut out = Vec::with_capacity(points.len());
    for r in 0..=rows {
        for c in 0..=cols {
            let p = at(r, c);
            let du = tangent(c.checked_sub(1).map(|c| at(r, c)), p, (c < cols).then(|| at(r, c + 1))) / 3.0;
            let dv = tangent(r.checked_sub(1).map(|r| at(r, c)), p, (r < rows).then(|| at(r + 1, c))) / 3.0;
            // Handles pointing out of the grid are unused: they stay at the point.
            let keep = |inside: bool, d: Vec2| if inside { d } else { Vec2::ZERO };
            let mut h = [Vec2::ZERO; 4];
            (h[H_RIGHT], h[H_LEFT], h[H_DOWN], h[H_UP]) = (keep(c < cols, du), keep(c > 0, -du), keep(r < rows, dv), keep(r > 0, -dv));
            out.push(h);
        }
    }
    out
}

/// A mesh envelope's grid (`rows`×`cols` patches, `points` and their `handles`; without handles
/// the smooth ones, see [`smooth_handles`]) as a gradient mesh with black points: what the
/// canvas draws and the Mesh tool and Direct Selection edit. `None` for a malformed grid.
pub fn envelope_grid(rows: u32, cols: u32, points: &[Point], handles: &[[Vec2; 4]]) -> Option<GradientMesh> {
    if rows == 0 || cols == 0 || grid_len(rows, cols) != Some(points.len()) {
        return None;
    }
    let smooth;
    let handles = if handles.len() == points.len() {
        handles
    } else {
        smooth = smooth_handles(rows, cols, points);
        &smooth
    };
    let points = points.iter().zip(handles).map(|(p, h)| MeshPoint { p: *p, color: Color::BLACK, opacity: 1.0, handles: *h }).collect();
    Some(GradientMesh { rows, cols, points })
}

/// The point of mesh `m` at (u, v) over the whole mesh (each patch an equal share; past the edges
/// the outer patches go on).
pub(super) fn mesh_at(m: &GradientMesh, u: f64, v: f64) -> Point {
    let (rows, cols) = (m.rows.max(1) as i64, m.cols.max(1) as i64);
    let (su, sv) = (u * cols as f64, v * rows as f64);
    let (c, r) = ((su.floor() as i64).clamp(0, cols - 1), (sv.floor() as i64).clamp(0, rows - 1));
    m.eval(r as usize, c as usize, su - c as f64, sv - r as f64)
}

/// The envelope `id` is, or the innermost one it sits in.
pub fn envelope_of(doc: &crate::Document, id: NodeId) -> Option<&Node> {
    doc.ancestry(id)?.into_iter().rev().find_map(|a| doc.node(a).filter(|n| matches!(n.kind, NodeKind::Envelope { .. })))
}

/// What a selected envelope shows on the canvas: its mesh lines (a warp's surface on its
/// [`warp_grid`], a mesh envelope's own grid) or its top object's outline, and for a warp or mesh
/// envelope the grid to edit (its points and handles; none while its contents are edited).
pub fn envelope_overlay(n: &Node) -> Option<(PathData, Option<GradientMesh>)> {
    let NodeKind::Envelope { kind, editing, .. } = &n.kind else { return None };
    let points = |grid: GradientMesh| (!*editing).then_some(grid);
    match &**kind {
        EnvelopeKind::Mesh { rows, cols, points: at, handles } => {
            let grid = envelope_grid(*rows, *cols, at, handles)?;
            Some((grid.lines(), points(grid)))
        }
        EnvelopeKind::Warp { .. } => match warp_grid(n) {
            Some(grid) => Some((grid.lines(), points(grid))),
            None => Some((EnvelopeMap::of(n)?.surface_mesh(4, 4, Color::BLACK).lines(), None)),
        },
        EnvelopeKind::TopObject { path, .. } => Some((path.clone(), None)),
    }
}

/// The grid sizes [`warp_grid`] tries, in order: patches per row and per column.
const WARP_GRIDS: [u32; 3] = [4, 8, 16];

/// How far a [`warp_grid`] may draw from the warp it replaces, as a share of the diagonal of the
/// warp's bounds.
const WARP_GRID_TOLERANCE: f64 = 1e-3;

/// The grid a warp envelope's points and handles are edited on: its surface sampled on the first
/// of 4×4, 8×8 and 16×16 patches whose curves draw within [`WARP_GRID_TOLERANCE`] of the warp, with
/// handles along it, as a gradient mesh with black points. Editing a point makes the envelope a
/// mesh envelope with this grid. `None` for other nodes, for content with no bounds, and for a warp
/// that no such grid follows closely enough or whose grid isn't finite.
/// Built again only when the envelope's content objects, warp, frame or options change, or what
/// type is measured with ([`set_type_ink`]).
pub fn warp_grid(n: &Node) -> Option<GradientMesh> {
    let NodeKind::Envelope { content, kind, frame, options, .. } = &n.kind else { return None };
    if !matches!(**kind, EnvelopeKind::Warp { .. }) {
        return None;
    }
    // `None` until a measure is registered, so a grid kept before then is built again after.
    let inputs = TYPE_INK.get().map(|(_, inputs)| inputs());
    let key: Vec<usize> = content.iter().map(|c| Arc::as_ptr(c) as usize).collect();
    let same = |k: &KeptGrid| k.kind == **kind && k.frame == **frame && k.options == *options && k.inputs == inputs;
    if let Some(grid) = WARP_GRIDS_KEPT.with_borrow(|kept| kept.get(&key).filter(|k| same(k)).map(|k| k.grid.clone())) {
        return grid;
    }
    let grid = build_warp_grid(n);
    WARP_GRIDS_KEPT.with_borrow_mut(|kept| {
        if kept.len() >= WARP_GRID_KEPT {
            // Entries for content that no longer exists go first, then one entry at a time.
            kept.retain(|_, k| k.content.iter().all(|c| c.strong_count() > 0));
            if kept.len() >= WARP_GRID_KEPT
                && let Some(any) = kept.keys().next().cloned()
            {
                kept.remove(&any);
            }
        }
        // The weak pointers keep the content's addresses from being reused while the entry is kept.
        let kept_content = content.iter().map(Arc::downgrade).collect();
        kept.insert(key, KeptGrid { content: kept_content, kind: (**kind).clone(), frame: **frame, options: *options, inputs, grid: grid.clone() });
    });
    grid
}

/// A grid [`warp_grid`] built, with what it was built from.
struct KeptGrid {
    content: Vec<std::sync::Weak<Node>>,
    kind: EnvelopeKind,
    frame: Affine,
    options: EnvelopeOptions,
    inputs: Option<u64>,
    grid: Option<GradientMesh>,
}

/// How many grids [`warp_grid`] keeps on one thread.
const WARP_GRID_KEPT: usize = 1024;

std::thread_local! {
    /// The grids [`warp_grid`] built on this thread, by the addresses of their envelopes' content.
    static WARP_GRIDS_KEPT: std::cell::RefCell<std::collections::HashMap<Vec<usize>, KeptGrid>> = Default::default();
}

/// [`warp_grid`], built from scratch.
fn build_warp_grid(n: &Node) -> Option<GradientMesh> {
    let map = EnvelopeMap::of(n)?;
    let bounds = map.bounds()?;
    let tol = bounds.width().hypot(bounds.height()) * WARP_GRID_TOLERANCE;
    if !tol.is_finite() {
        return None;
    }
    WARP_GRIDS.into_iter().find_map(|k| {
        let m = map.surface_mesh(k, k, Color::BLACK);
        let finite = m.points.iter().all(|q| q.p.is_finite() && q.handles.iter().all(|h| h.is_finite()));
        // Each patch is checked at its corners, the middles of its sides and its middle.
        let steps = 2 * k as usize;
        let at = |i: usize| i as f64 / steps as f64;
        let fits = || (0..=steps).all(|j| (0..=steps).all(|i| mesh_at(&m, at(i), at(j)).distance(map.at(at(i), at(j))) <= tol));
        (finite && fits()).then_some(m)
    })
}

/// The frame an envelope keeps after transform `m` (its frame so far composed with the
/// transform): identity when its axes stay square to the page, unflipped, since evaluating on
/// the page axes gives the same result then.
pub fn envelope_frame(m: Affine) -> Affine {
    let [a, b, c, d, _, _] = m.as_coeffs();
    if b.abs() < 1e-12 && c.abs() < 1e-12 && a > 0.0 && d > 0.0 { Affine::IDENTITY } else { m }
}

/// An envelope's point map: content placed in the envelope's own frame maps, by its bounds there,
/// onto the unit square and from there onto the envelope's surface ([`surface_of`]).
pub struct EnvelopeMap<'a> {
    src: Rect,
    frame: Affine,
    to_frame: Affine,
    surface: Box<dyn Fn(f64, f64) -> Point + 'a>,
}

impl<'a> EnvelopeMap<'a> {
    /// The map of an envelope of `kind` around `content` with `frame` and `options`, from the
    /// content's bounds with type measured by its glyph outlines ([`envelope_content_bounds`]).
    /// `None` when the content has no bounds.
    pub fn new(content: &[Arc<Node>], kind: &'a EnvelopeKind, frame: Affine, options: EnvelopeOptions) -> Option<Self> {
        let frame = if frame.is_finite() && frame.determinant().abs() > 1e-12 { frame } else { Affine::IDENTITY };
        let to_frame = frame.inverse();
        let src = content_set_box(content, false, to_frame, false).or_else(|| content_set_box(content, false, to_frame, true))?;
        let src = options.source_box(src);
        Some(Self { src, frame, to_frame, surface: surface_of(kind, src, frame) })
    }

    /// The map of envelope node `n` (`None` for other nodes).
    pub fn of(n: &'a Node) -> Option<Self> {
        match &n.kind {
            NodeKind::Envelope { content, kind, frame, options, .. } => Self::new(content, kind, **frame, *options),
            _ => None,
        }
    }

    /// The content's bounds in the envelope's frame.
    pub fn src(&self) -> Rect {
        self.src
    }

    /// The surface point at (u, v) ∈ [0,1]².
    pub fn at(&self, u: f64, v: f64) -> Point {
        (self.surface)(u, v)
    }

    /// Where the envelope puts document point `p`.
    pub fn map(&self, p: Point) -> Point {
        let q = self.to_frame * p;
        let (w, h) = (self.src.width().max(1e-9), self.src.height().max(1e-9));
        self.at((q.x - self.src.x0) / w, (q.y - self.src.y0) / h)
    }

    /// A flat `(rows+1)×(cols+1)` grid over the content, in the envelope's frame (row-major).
    pub fn grid(&self, rows: u32, cols: u32) -> Vec<Point> {
        grid_points(self.src, rows, cols).into_iter().map(|p| self.frame * p).collect()
    }

    /// The surface sampled on a `rows`×`cols` grid as a mesh whose handles follow it and whose
    /// points are `color`: Release gives it back as the envelope's shape, Reset with Mesh starts
    /// from it.
    pub fn surface_mesh(&self, rows: u32, cols: u32, color: Color) -> GradientMesh {
        GradientMesh::from_surface(rows, cols, &|u, v| self.at(u, v), &|_, _| color)
    }

    /// The surface's bounds (sampled; a mesh's points and a top object's path bound it exactly).
    fn bounds(&self) -> Option<Rect> {
        const N: usize = 16;
        let mut r: Option<Rect> = None;
        for j in 0..=N {
            for i in 0..=N {
                let q = self.at(i as f64 / N as f64, j as f64 / N as f64);
                r = Some(r.map_or(Rect::from_points(q, q), |r| r.union_pt(q)));
            }
        }
        r
    }
}

/// Evaluate an envelope of `kind` around `content` on the page axes with the options envelopes
/// had before Envelope Options (see [`expand_live_hooks`] for an envelope node).
pub fn envelope_expand(content: &[Arc<Node>], kind: &EnvelopeKind, fidelity: f64, outline: Outliner) -> Vec<Node> {
    expand_envelope(content, kind, fidelity, EnvelopeOptions::default(), Affine::IDENTITY, outline.into())
}

/// An envelope's content pushed through its map: Fidelity sets how finely curves are cut (and
/// into how many pieces images are), the options what bends with the art.
pub(super) fn expand_envelope(
    content: &[Arc<Node>],
    kind: &EnvelopeKind,
    fidelity: f64,
    options: EnvelopeOptions,
    frame: Affine,
    hooks: Hooks,
) -> Vec<Node> {
    let Some(map) = EnvelopeMap::new(content, kind, frame, options) else { return content.iter().map(|c| (**c).clone()).collect() };
    let src = map.src();
    let fid = if fidelity.is_finite() { fidelity.clamp(0.0, 100.0) / 100.0 } else { 0.5 };
    let f = |p: Point| map.map(p);
    let w = Warper {
        f: &f,
        piece: src.width().hypot(src.height()).max(1e-6) / (8.0 + fid * 56.0),
        eps: (src.width().max(src.height()) * 0.05).max(0.5),
        cells: (4.0 + fid * 12.0).round() as usize,
        hooks,
        options,
        depth: 0,
        type_outlines: false,
    };
    content.iter().map(|c| map_node(c, &w)).collect()
}

/// Envelope bounds: a mesh's points, a top object's path, the warp's surface (sampled).
pub fn envelope_bounds(content: &[Arc<Node>], kind: &EnvelopeKind, frame: Affine, options: EnvelopeOptions) -> Option<Rect> {
    match kind {
        EnvelopeKind::Mesh { rows, cols, points, handles } if !handles.is_empty() => envelope_grid(*rows, *cols, points, handles)?.outline().bounds(),
        EnvelopeKind::Mesh { points, .. } => {
            let first = *points.first()?;
            Some(points.iter().fold(Rect::from_points(first, first), |r, p| r.union_pt(*p)))
        }
        EnvelopeKind::TopObject { path, .. } => path.bounds(),
        EnvelopeKind::Warp { .. } => EnvelopeMap::new(content, kind, frame, options)?.bounds(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CharStyle, TextObject};

    /// Look `n` up in `kept`, measuring it (and counting the measures) when it isn't there.
    fn look_up(kept: &mut TypeInks, n: &Arc<Node>, linear: Affine, measured: &mut usize) -> Option<Rect> {
        let NodeKind::Text(t) = &n.kind else { panic!("type") };
        match kept.find(n, t, &(linear, None), 0) {
            Ok(ink) => ink,
            Err(missing) => {
                *measured += 1;
                let ink = Some(Rect::new(0.0, 0.0, *measured as f64, 1.0));
                kept.keep(n, *missing, ink);
                ink
            }
        }
    }

    fn type_node(i: usize, text: &str) -> Arc<Node> {
        let t = TextObject::point(Point::new(i as f64 * 10.0, 0.0), text, CharStyle::default());
        Arc::new(Node::new(NodeId(i as u64), NodeKind::Text(Box::new(t))))
    }

    /// Many type objects are each measured once however often they are looked up; a moved or copied
    /// object is found by its content, and a changed one or another transform is measured again.
    #[test]
    fn measured_type_is_kept_for_many_objects() {
        let mut kept = TypeInks::default();
        let mut measured = 0;
        let nodes: Vec<_> = (0..200).map(|i| type_node(i, &format!("Type {i}"))).collect();
        for _ in 0..5 {
            for n in &nodes {
                look_up(&mut kept, n, Affine::IDENTITY, &mut measured);
            }
        }
        assert_eq!(measured, 200);
        let mut moved = (*nodes[7]).clone();
        moved.transform(Affine::translate((300.0, 40.0)), true);
        let moved = Arc::new(moved);
        assert_eq!(look_up(&mut kept, &moved, Affine::IDENTITY, &mut measured), look_up(&mut kept, &nodes[7], Affine::IDENTITY, &mut measured));
        assert_eq!(measured, 200, "a moved object is found by its content");
        let changed = type_node(7, "Changed");
        look_up(&mut kept, &changed, Affine::IDENTITY, &mut measured);
        assert_eq!(measured, 201, "a changed object is measured again");
        look_up(&mut kept, &nodes[7], Affine::rotate(0.5), &mut measured);
        assert_eq!(measured, 202, "another transform is measured again");
    }

    /// Type that differs only in what layout reads but files don't save (a first paragraph that
    /// continues a thread, the resolved bounds of an inline graphic) is measured again.
    #[test]
    fn type_that_differs_in_what_is_not_saved_is_measured_again() {
        let mut kept = TypeInks::default();
        let mut measured = 0;
        let mut t = TextObject::point(Point::ZERO, "Type", CharStyle::default());
        t.runs.push(crate::TextRun::new("", CharStyle::default()));
        t.runs[1].inline = Some(crate::InlineArt { symbol: "S".into(), scale: 1.0, baseline_shift: 0.0, bounds: None });
        let node = |t: &TextObject| Arc::new(Node::new(NodeId(1), NodeKind::Text(Box::new(t.clone()))));
        look_up(&mut kept, &node(&t), Affine::IDENTITY, &mut measured);
        look_up(&mut kept, &node(&t), Affine::IDENTITY, &mut measured);
        assert_eq!(measured, 1, "a copy is found by its content");
        t.first_para_continues = true;
        look_up(&mut kept, &node(&t), Affine::IDENTITY, &mut measured);
        assert_eq!(measured, 2, "a first paragraph that continues a thread");
        if let Some(inline) = &mut t.runs[1].inline {
            inline.bounds = Some(Rect::new(0.0, 0.0, 10.0, 20.0));
        }
        look_up(&mut kept, &node(&t), Affine::IDENTITY, &mut measured);
        assert_eq!(measured, 3, "an inline graphic's bounds");
    }

    /// A stroked group of type in a scaled frame reaches past its members by its own strokes scaled
    /// with the frame, as the group moved into the frame does; deeply nested groups are measured in
    /// one walk each.
    #[test]
    fn a_stroked_group_of_type_is_measured_with_its_strokes_in_the_frame() {
        let text = type_node(1, "Group");
        let mut group = Node::group(NodeId(2), vec![text.clone()]);
        group.appearance = crate::Appearance::basic(vectorcraft_color::Paint::None, vectorcraft_color::Paint::solid(Color::BLACK), 4.0).into();
        let group = Arc::new(group);
        let to = Affine::scale(2.0);
        let mut moved = (*group).clone();
        moved.transform(to, true);
        let want = moved.visual_of_members(std::iter::once(content_box(&text, true, to, true)));
        assert!(want.is_some());
        assert_eq!(content_box(&group, true, to, true), want);
        let mut nested = group;
        for i in 0..300 {
            nested = Arc::new(Node::group(NodeId(10 + i), vec![nested]));
        }
        assert!(content_box(&nested, false, Affine::IDENTITY, true).is_some());
    }

    /// A warp envelope of `style` at `bend` with map `version` around a 200×100 rectangle at
    /// (100, 50).
    fn warp_node(style: &str, bend: f64, h: f64, version: u32) -> Node {
        let rect = vectorcraft_geom::shapes::rectangle(Rect::new(100.0, 50.0, 300.0, 150.0));
        let content = Arc::new(Node::path(NodeId(1), rect, crate::Appearance::default_art()));
        let kind = EnvelopeKind::Warp { style: style.into(), bend, h, v: 0.0, horizontal: true, version };
        Node::new(
            NodeId(2),
            NodeKind::Envelope {
                content: vec![content],
                kind: Box::new(kind),
                fidelity: 50.0,
                editing: false,
                options: EnvelopeOptions::NEW,
                frame: Box::new(Affine::IDENTITY),
            },
        )
    }

    const STYLES: [&str; 15] = [
        "arc",
        "arcLower",
        "arcUpper",
        "arch",
        "bulge",
        "shellLower",
        "shellUpper",
        "flag",
        "wave",
        "fish",
        "rise",
        "fisheye",
        "inflate",
        "squeeze",
        "twist",
    ];

    /// Every warp style's Bézier net has a grid to edit: its points lie on the warp, and the grid
    /// draws within a thousandth of the warp's diagonal of it, between the grid lines too. The
    /// overlay shows that grid's lines and points.
    #[test]
    fn every_warp_style_has_a_grid_that_keeps_its_shape() {
        for style in STYLES {
            for (bend, h) in [(50.0, 0.0), (100.0, 0.0), (-100.0, 60.0)] {
                let n = warp_node(style, bend, h, crate::live::WARP_NET);
                let grid = warp_grid(&n).unwrap_or_else(|| panic!("{style} {bend} {h}: no grid"));
                assert!(WARP_GRIDS.contains(&grid.rows) && grid.rows == grid.cols, "{style}: {}×{}", grid.rows, grid.cols);
                let map = EnvelopeMap::of(&n).unwrap();
                let b = map.bounds().unwrap();
                let tol = b.width().hypot(b.height()) * 1e-3;
                let k = grid.cols as usize;
                for (i, q) in grid.points.iter().enumerate() {
                    let want = map.at((i % (k + 1)) as f64 / k as f64, (i / (k + 1)) as f64 / k as f64);
                    assert!(q.p.distance(want) < 1e-9, "{style}: point {i} at {:?}, the warp at {want:?}", q.p);
                }
                for j in 0..=20 {
                    for i in 0..=20 {
                        let (u, v) = (i as f64 / 20.0, j as f64 / 20.0);
                        let off = mesh_at(&grid, u, v).distance(map.at(u, v));
                        assert!(off <= tol, "{style} {bend} {h}: {off} off at ({u}, {v})");
                    }
                }
                let (lines, shown) = envelope_overlay(&n).unwrap();
                assert_eq!(shown.as_ref(), Some(&grid), "{style}");
                assert_eq!(lines, grid.lines(), "{style}");
            }
        }
    }

    /// A kept warp grid is built again when the content, the warp, the frame or the options change,
    /// and is the grid built from scratch otherwise.
    #[test]
    fn a_kept_warp_grid_follows_changes() {
        let n = warp_node("arc", 50.0, 0.0, crate::live::WARP_NET);
        let first = warp_grid(&n);
        assert!(first.is_some());
        assert_eq!(warp_grid(&n), first);
        assert_eq!(build_warp_grid(&n), first);
        let changed = |f: &dyn Fn(&mut Node)| {
            let mut m = n.clone();
            f(&mut m);
            let grid = warp_grid(&m);
            assert_eq!(grid, build_warp_grid(&m));
            assert_ne!(grid, first);
        };
        changed(&|m| {
            if let NodeKind::Envelope { content, .. } = &mut m.kind {
                Arc::make_mut(&mut content[0]).transform(Affine::translate((40.0, 0.0)), true);
            }
        });
        changed(&|m| {
            if let NodeKind::Envelope { kind, .. } = &mut m.kind
                && let EnvelopeKind::Warp { bend, .. } = &mut **kind
            {
                *bend = -30.0;
            }
        });
        changed(&|m| {
            if let NodeKind::Envelope { frame, .. } = &mut m.kind {
                **frame = Affine::rotate(0.3);
            }
        });
        changed(&|m| {
            if let NodeKind::Envelope { options, .. } = &mut m.kind {
                options.bounds_margin = [0.1; 4];
            }
        });
    }

    /// The table of kept warp grids stays bounded: full of envelopes that still exist, it drops one
    /// entry at a time instead of emptying, and it drops the entries of envelopes that are gone
    /// first.
    #[test]
    fn the_kept_warp_grids_stay_bounded() {
        let nodes: Vec<Node> = (0..WARP_GRID_KEPT + 10).map(|i| warp_node("arc", (i % 100) as f64, 0.0, crate::live::WARP_NET)).collect();
        for n in &nodes {
            assert!(warp_grid(n).is_some());
        }
        assert_eq!(WARP_GRIDS_KEPT.with_borrow(|kept| kept.len()), WARP_GRID_KEPT);
        // Once the envelopes are gone, their entries are the ones dropped.
        drop(nodes);
        assert!(warp_grid(&warp_node("arc", 10.0, 0.0, crate::live::WARP_NET)).is_some());
        assert_eq!(WARP_GRIDS_KEPT.with_borrow(|kept| kept.len()), 1);
    }

    /// A warp drawn by the old point map that no grid of up to 16×16 patches follows closely
    /// enough, content without bounds and other envelopes have no warp grid; the overlay still
    /// draws the old warp's lines. While an envelope's contents are edited, it shows no points.
    #[test]
    fn a_warp_no_grid_follows_has_no_grid_to_edit() {
        let n = warp_node("fisheye", 100.0, 0.0, 0);
        assert!(warp_grid(&n).is_none());
        let (lines, grid) = envelope_overlay(&n).unwrap();
        assert!(grid.is_none());
        assert!(lines.anchor_count() > 0);
        // The same style on its Bézier net has one.
        assert!(warp_grid(&warp_node("fisheye", 100.0, 0.0, crate::live::WARP_NET)).is_some());
        let mut empty = warp_node("arc", 50.0, 0.0, crate::live::WARP_NET);
        if let NodeKind::Envelope { content, .. } = &mut empty.kind {
            content.clear();
        }
        assert!(warp_grid(&empty).is_none());
        let mut mesh = warp_node("arc", 50.0, 0.0, crate::live::WARP_NET);
        if let NodeKind::Envelope { kind, .. } = &mut mesh.kind {
            **kind = EnvelopeKind::Mesh { rows: 1, cols: 1, points: grid_points(Rect::new(0.0, 0.0, 1.0, 1.0), 1, 1), handles: vec![] };
        }
        assert!(warp_grid(&mesh).is_none());
        // While the contents are edited, a warp's and a mesh envelope's lines show without points.
        for mut n in [warp_node("arc", 50.0, 0.0, crate::live::WARP_NET), mesh] {
            if let NodeKind::Envelope { editing, .. } = &mut n.kind {
                *editing = true;
            }
            let (lines, grid) = envelope_overlay(&n).unwrap();
            assert!(grid.is_none() && lines.anchor_count() > 0);
        }
    }

    /// A table that fills up is emptied instead of growing, and what is looked up after that is
    /// measured again.
    #[test]
    fn the_tables_stay_bounded() {
        let mut kept = TypeInks::default();
        let mut measured = 0;
        let nodes: Vec<_> = (0..TYPE_INK_KEPT + 10).map(|i| type_node(i, &format!("Type {i}"))).collect();
        for n in &nodes {
            look_up(&mut kept, n, Affine::IDENTITY, &mut measured);
        }
        assert!(kept.content.len() <= TYPE_INK_KEPT && kept.nodes.len() <= TYPE_INK_KEPT, "{} {}", kept.content.len(), kept.nodes.len());
        assert_eq!(measured, TYPE_INK_KEPT + 10);
    }
}
