//! Envelopes from the editing data (`(Adobe Deform Plugin) 1 0 0 XP`), read as live envelopes.
//!
//! What an envelope is and does is Illustrator's public documentation: Make with Warp, Make with
//! Mesh (rows and columns) and Make with Top Object
//! (<https://helpx.adobe.com/illustrator/desktop/manage-objects/reshape-transform-objects/distort-objects-with-envelopes.html>),
//! Edit Contents
//! (<https://helpx.adobe.com/illustrator/desktop/manage-objects/reshape-transform-objects/edit-the-contents-of-envelopes.html>),
//! Envelope Options
//! (<https://helpx.adobe.com/illustrator/desktop/manage-objects/reshape-transform-objects/envelope-panel-options.html>)
//! and the warp styles with their bend and distortions
//! (<https://helpx.adobe.com/illustrator/desktop/manage-objects/reshape-transform-objects/distort-and-transform-effects-in-illustrator.html>).
//!
//! An envelope is written in three parts. On `%_` lines, a group holding the mesh that shapes it,
//! then its `/ArtDictionary`: Envelope Options as a bit word (`AIDeformOptionsKey`), Fidelity
//! (`AIDeformFidelityKey`), Edit Contents (`AIDeformEditModeKey`), a warp's style (`DeformStyle`,
//! numbered as Effect › Warp lists them), bend and distortions (`DeformValue`, `DeformHoriz`,
//! `DeformVert`, fractions) and orientation (`Rotate`), the size of the contents, read as the size
//! of the box they map from with Distort Appearance on when it matches their bounds with strokes
//! (`AIDeformArtWidthKey`, `AIDeformArtHeightKey`), the contents (`X=` … `X+`, then
//! `(AIDeformArtKey)`) and the kind (`AIDeformStyle`: 1 a warp, 2 a mesh, 3 a top object). Then the
//! distorted art on plain lines, and the plugin's `XP`. While the contents are edited, the group
//! holds the contents and the dictionary the mesh. A top object's outline is the edge of its
//! one-patch mesh. A warp's mesh is a grid of its shape: a warp of type whose own warp doesn't come
//! out as drawn opens as a mesh envelope on that grid where that does. An envelope whose parts don't
//! read, or that doesn't come out as drawn, keeps its drawn art.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

use kurbo::{Affine, BezPath, Point, Rect, Vec2};
use vectorcraft_doc::live::{
    EnvelopeKind, EnvelopeOptions, GradientMesh, H_DOWN, H_LEFT, H_RIGHT, H_UP, PreserveShape, default_fidelity, envelope_content_boxes,
    envelope_stroke_margin,
};
use vectorcraft_doc::{Appearance, Document, Effect, ImageBlob, LayerColor, Node, NodeId, NodeKind, PatternDef};
use vectorcraft_geom::PathData;

use super::obj::Obj;
use crate::import::native::compare::{MAX_DIFFERENCE, MAX_SOLID_DIFFERENCE, compare_scale, differ, differ_in_colour, differ_on_black};

/// The plugin that writes envelopes.
pub(super) const PLUGIN: &str = "Adobe Deform Plugin"; // brand-ok: the plugin name the files use
/// The note for an envelope that keeps its drawn art.
pub(crate) const DRAWN: &str = "an envelope VectorCraft couldn't read as a live envelope comes in as its drawn art";
/// The note for an envelope of type, which keeps its drawn art with its type hidden (unless the
/// text document gives the type nothing, which a note of its own then says).
pub(crate) const TYPE_DRAWN: &str = "type in an envelope comes in as the envelope's drawn art, not as editable type";
/// The note for warped type that keeps its drawn art, before how many.
pub(crate) const WARPED_TYPE_DRAWN: &str = "warped text objects that came in as drawn art, not as editable type";

/// The note for an envelope that keeps its drawn art: [`TYPE_DRAWN`] for an envelope of type
/// (`typed`), else [`DRAWN`].
pub(crate) fn drawn_note(typed: bool) -> &'static str {
    if typed { TYPE_DRAWN } else { DRAWN }
}
/// The most envelopes waiting for their drawn art at once.
pub(super) const MAX_PENDING: usize = 64;
/// The most envelopes and warped text objects one import draws to check; more keep their drawn art.
pub(crate) const MAX_CHECKED: usize = 256;
/// The most rows or columns of a mesh read (Make with Mesh's own limit).
const MAX_GRID: u32 = 50;
/// How far, in points over width and height together, a saved size may lie from the contents'
/// bounds with their strokes and still count as that box.
const SAVED_SIZE_SLACK: f64 = 0.5;
/// The bits of the options word that are read: Distort Appearance, Distort Pattern Fills, Distort
/// Linear Gradients, Anti-Alias (set: off) and Preserve Shape Using (set: Clipping Mask).
const KNOWN_BITS: i64 = 0b111 | 1 << 16 | 1 << 17;
/// More of the inked pixels than this differing, each with all its neighbours, and the envelope
/// doesn't come out as drawn.
const MAX_SOLID: f64 = 0.03;
/// More of the inked pixels than this differing at all, and the envelope doesn't come out as drawn.
const MAX_ANY: f64 = 0.1;

/// The whole number of entry `key`: `None` when it is missing, `Some(None)` when it isn't one.
fn whole(o: &Obj, key: &str) -> Option<Option<i64>> {
    o.get(key)?;
    let n = o.nums(key);
    Some(match n.as_slice() {
        [v] if v.is_finite() && v.fract() == 0.0 && v.abs() < 1e12 => Some(*v as i64),
        _ => None,
    })
}

/// A switch (`0` or `1`), off when missing.
fn switch(o: &Obj, key: &str) -> Option<bool> {
    match whole(o, key) {
        None | Some(Some(0)) => Some(false),
        Some(Some(1)) => Some(true),
        _ => None,
    }
}

/// Envelope Options and Fidelity.
pub(super) fn options(o: &Obj) -> Option<(EnvelopeOptions, f64)> {
    let word = whole(o, "AIDeformOptionsKey")??;
    if word < 0 || word & !KNOWN_BITS != 0 {
        return None;
    }
    let fidelity = match whole(o, "AIDeformFidelityKey") {
        None => default_fidelity(),
        Some(Some(f)) if (0..=100).contains(&f) => f as f64,
        Some(_) => return None,
    };
    let bit = |n: u32| word & (1 << n) != 0;
    let options = EnvelopeOptions {
        anti_alias: !bit(16),
        preserve_shape: if bit(17) { PreserveShape::ClippingMask } else { PreserveShape::Transparency },
        distort_appearance: bit(0),
        distort_pattern_fills: bit(1),
        distort_linear_gradients: bit(2),
        bounds_margin: [0.0; 4],
    };
    Some((options, fidelity))
}

/// The margins of `content` when the size the file saves for it (`AIDeformArtWidthKey`,
/// `AIDeformArtHeightKey`) matches its bounds with its strokes within [`SAVED_SIZE_SLACK`] and lies
/// nearer them than its geometric bounds. None without that size, or for contents with no width or
/// no height.
fn saved_margin(o: &Obj, content: &[Arc<Node>]) -> Option<[f32; 4]> {
    let saved = |key: &str| match o.nums(key).as_slice() {
        [v] if v.is_finite() => Some(*v),
        _ => None,
    };
    let (w, h) = (saved("AIDeformArtWidthKey")?, saved("AIDeformArtHeightKey")?);
    let (stroked, plain) = envelope_content_boxes(content, Affine::IDENTITY)?;
    let off = |r: Rect| (r.width() - w).abs() + (r.height() - h).abs();
    if off(stroked) > SAVED_SIZE_SLACK || off(stroked) >= off(plain) {
        return None;
    }
    envelope_stroke_margin(stroked, plain)
}

/// Envelope `kind` mapping from its contents' bounds with their strokes, for an envelope with
/// Distort Appearance whose contents have strokes it doesn't map from already; `None` otherwise.
fn with_strokes(kind: &NodeKind) -> Option<NodeKind> {
    let NodeKind::Envelope { content, options, .. } = kind else { return None };
    if !options.distort_appearance {
        return None;
    }
    let (stroked, plain) = envelope_content_boxes(content, Affine::IDENTITY)?;
    let margin = envelope_stroke_margin(stroked, plain)?;
    if margin == options.bounds_margin || margin.iter().any(|m| *m < 0.0) || margin.iter().all(|m| *m == 0.0) {
        return None;
    }
    let mut kind = kind.clone();
    if let NodeKind::Envelope { options, .. } = &mut kind {
        options.bounds_margin = margin;
    }
    Some(kind)
}

/// A warp: its style, bend, distortions and orientation.
fn warp(o: &Obj) -> Option<EnvelopeKind> {
    let style = whole(o, "DeformStyle")??;
    let id = super::style::warp_style(style)?;
    let percent = |key: &str| -> Option<f64> {
        if o.get(key).is_none() {
            return Some(0.0);
        }
        match o.nums(key).as_slice() {
            [v] if v.is_finite() && (-1.0..=1.0).contains(v) => Some(v * 100.0),
            _ => None,
        }
    };
    o.get("DeformValue")?;
    let vertical = switch(o, "Rotate")?;
    Some(EnvelopeKind::Warp {
        style: id.into(),
        bend: percent("DeformValue")?,
        h: percent("DeformHoriz")?,
        v: percent("DeformVert")?,
        horizontal: !vertical,
    })
}

/// The gradient mesh `n` is, or the one its plain groups hold alone.
fn only_mesh(n: &Node) -> Option<&GradientMesh> {
    match &n.kind {
        NodeKind::Mesh(m) => Some(m),
        NodeKind::Group { children, clip: false } => match children.as_slice() {
            [c] => only_mesh(c),
            _ => None,
        },
        _ => None,
    }
}

/// A grid point and its handles.
type GridPoint = (Point, [Vec2; 4]);

/// The points of mesh `m` as a `rows`×`cols` grid with its first row at the top and its first
/// column at the left (y down), as an envelope's mesh lays them out; `None` for a grid out of range
/// or with points that aren't finite.
fn upright(m: &GradientMesh) -> Option<(u32, u32, Vec<GridPoint>)> {
    let (mut rows, mut cols) = (m.rows, m.cols);
    if rows == 0 || cols == 0 || rows > MAX_GRID || cols > MAX_GRID {
        return None;
    }
    let len = (rows as usize + 1) * (cols as usize + 1);
    if m.points.len() != len {
        return None;
    }
    let finite = |v: Vec2| v.x.is_finite() && v.y.is_finite();
    if !m.points.iter().all(|q| finite(q.p.to_vec2()) && q.handles.iter().all(|h| finite(*h))) {
        return None;
    }
    let mut pts: Vec<GridPoint> = m.points.iter().map(|q| (q.p, q.handles)).collect();
    let at = |pts: &[GridPoint], cols: u32, r: u32, c: u32| pts.get((r * (cols + 1) + c) as usize).map_or(Point::ZERO, |q| q.0);
    let remap = |h: [Vec2; 4], slots: [usize; 4]| slots.map(|s| h.get(s).copied().unwrap_or(Vec2::ZERO));
    // How the grid runs along its rows (its first and last rows, end to end added up) and down its
    // columns (its first and last columns): read from both sides, a grid bent far round (its end
    // columns leaning more than halfway over) still reads the way it runs.
    let runs = |pts: &[GridPoint], rows: u32, cols: u32| {
        let along = (at(pts, cols, 0, cols) - at(pts, cols, 0, 0)) + (at(pts, cols, rows, cols) - at(pts, cols, rows, 0));
        let down = (at(pts, cols, rows, 0) - at(pts, cols, 0, 0)) + (at(pts, cols, rows, cols) - at(pts, cols, 0, cols));
        (along, down)
    };
    // How much of a run is across the page.
    let level = |v: Vec2| if v.hypot() > 0.0 { v.x.abs() / v.hypot() } else { 0.0 };
    // Columns that run across the page more than rows do: the grid goes the other way round.
    let (along, down) = runs(&pts, rows, cols);
    if level(down) > level(along) {
        let mut t = Vec::with_capacity(len);
        for r in 0..=cols {
            for c in 0..=rows {
                let (p, h) = pts.get((c * (cols + 1) + r) as usize).copied()?;
                // The new next column is the old next row.
                let mut slots = [0; 4];
                (slots[H_RIGHT], slots[H_LEFT], slots[H_DOWN], slots[H_UP]) = (H_DOWN, H_UP, H_RIGHT, H_LEFT);
                t.push((p, remap(h, slots)));
            }
        }
        (rows, cols) = (cols, rows);
        pts = t;
    }
    if runs(&pts, rows, cols).0.x < 0.0 {
        for row in pts.chunks_mut(cols as usize + 1) {
            row.reverse();
            for (_, h) in row.iter_mut() {
                h.swap(H_RIGHT, H_LEFT);
            }
        }
    }
    if runs(&pts, rows, cols).1.y < 0.0 {
        let mut flipped: Vec<GridPoint> = pts.chunks(cols as usize + 1).rev().flatten().copied().collect();
        for (_, h) in &mut flipped {
            h.swap(H_DOWN, H_UP);
        }
        pts = flipped;
    }
    Some((rows, cols, pts))
}

/// The outline of an upright grid: its edge, clockwise from the top left corner.
fn outline(rows: u32, cols: u32, pts: &[GridPoint]) -> Option<PathData> {
    let at = |r: u32, c: u32| pts.get((r * (cols + 1) + c) as usize).copied();
    // Each edge: from a point by one of its handles to the next point by one of its handles.
    type Edge = ((u32, u32), usize, (u32, u32), usize);
    let mut ring: Vec<Edge> = vec![];
    ring.extend((0..cols).map(|c| ((0, c), H_RIGHT, (0, c + 1), H_LEFT)));
    ring.extend((0..rows).map(|r| ((r, cols), H_DOWN, (r + 1, cols), H_UP)));
    ring.extend((1..=cols).rev().map(|c| ((rows, c), H_LEFT, (rows, c - 1), H_RIGHT)));
    ring.extend((1..=rows).rev().map(|r| ((r, 0), H_UP, (r - 1, 0), H_DOWN)));
    let mut bp = BezPath::new();
    bp.move_to(at(0, 0)?.0);
    for ((ar, ac), out, (br, bc), into) in ring {
        let ((a, ha), (b, hb)) = (at(ar, ac)?, at(br, bc)?);
        bp.curve_to(a + *ha.get(out)?, b + *hb.get(into)?, b);
    }
    bp.close_path();
    Some(PathData::from_bezpath(&bp))
}

/// Has group `n` nothing of its own: no name, lock, transparency, appearance or other properties,
/// shown?
fn plain_group(n: &Node) -> bool {
    let plain = Node::new(n.id, NodeKind::Group { children: vec![], clip: false });
    matches!(n.kind, NodeKind::Group { clip: false, .. })
        && (&n.name, n.visible, n.locked, n.opacity, n.blend, n.isolate, n.knockout, n.knockout_shape)
            == (&plain.name, plain.visible, plain.locked, plain.opacity, plain.blend, plain.isolate, plain.knockout, plain.knockout_shape)
        && n.appearance == plain.appearance
        && n.extras() == plain.extras()
}

/// `nodes` without the plain group the editing data puts round an envelope's contents.
fn unwrapped(mut nodes: Vec<Node>) -> Vec<Arc<Node>> {
    if nodes.len() == 1
        && let Some(n) = nodes.first()
        && plain_group(n)
        && let Some(n) = nodes.pop()
        && let NodeKind::Group { children, .. } = n.kind
    {
        return children;
    }
    nodes.into_iter().map(Arc::new).collect()
}

/// The live envelope of dictionary `o`, the group on `%_` lines before it (`before`) and the art
/// in its `AIDeformArtKey` block (`inner`); `None` where they don't make one.
pub(super) fn live(o: &Obj, before: Option<Node>, inner: Vec<Node>) -> Option<NodeKind> {
    envelope_as(o, before, inner, false)
}

/// Is the envelope of dictionary `o` a warp?
fn is_warp(o: &Obj) -> bool {
    whole(o, "AIDeformStyle") == Some(Some(1))
}

/// [`live`] for a warp, as a mesh envelope on the grid its editing data saves (the mesh that shapes
/// it), its points and handles as saved: it draws as that grid does, without the warp's style, bend
/// and distortions. `None` for an envelope that isn't a warp, or whose grid doesn't read.
fn live_on_saved_grid(o: &Obj, before: Option<Node>, inner: Vec<Node>) -> Option<NodeKind> {
    envelope_as(o, before, inner, true)
}

/// [`live`], or with `on_grid` [`live_on_saved_grid`].
fn envelope_as(o: &Obj, before: Option<Node>, inner: Vec<Node>, on_grid: bool) -> Option<NodeKind> {
    let editing = switch(o, "AIDeformEditModeKey")?;
    let (mut options, fidelity) = options(o)?;
    let before = before?;
    // While the contents are edited, they are the group before the dictionary and the mesh is in it.
    let (content, shape): (Vec<Arc<Node>>, Option<&GradientMesh>) = if editing {
        let NodeKind::Group { children, clip: false } = &before.kind else { return None };
        let shape = match inner.as_slice() {
            [n] => only_mesh(n),
            _ => None,
        };
        (children.clone(), shape)
    } else {
        let shape = only_mesh(&before);
        (unwrapped(inner), shape)
    };
    if content.is_empty() {
        return None;
    }
    if options.distort_appearance {
        options.bounds_margin = saved_margin(o, &content).unwrap_or_default();
    }
    let kind = match (whole(o, "AIDeformStyle")??, on_grid) {
        (1, false) => warp(o)?,
        (1, true) | (2, false) => {
            let (rows, cols, pts) = upright(shape?)?;
            let (points, handles) = pts.into_iter().unzip();
            EnvelopeKind::Mesh { rows, cols, points, handles }
        }
        (3, false) => {
            let (rows, cols, pts) = upright(shape?)?;
            // The outline runs clockwise from the top left corner, one segment per grid edge.
            let corners = [0, cols, cols.checked_add(rows)?, cols.checked_mul(2)?.checked_add(rows)?];
            EnvelopeKind::TopObject { path: outline(rows, cols, &pts)?, corners: Some(corners) }
        }
        _ => return None,
    };
    Some(NodeKind::Envelope { content, kind: Box::new(kind), fidelity, editing, options, frame: Box::default() })
}

/// What the import's documents hold that art refers to: the patterns and image blobs.
pub(super) struct Shared<'a> {
    pub(super) patterns: &'a [PatternDef],
    pub(super) images: &'a BTreeMap<String, ImageBlob>,
}

/// A document of `node` alone, on a layer with id `layer`, with the patterns and images it shows.
fn alone(node: &Node, layer: NodeId, shared: &Shared<'_>) -> Document {
    let mut doc = Document::new(1.0, 1.0);
    let mut l = Node::layer(layer, "", LayerColor::Preset(0));
    if let Some(c) = l.children_mut() {
        c.push(Arc::new(node.clone()));
    }
    doc.layers = vec![Arc::new(l)];
    doc.patterns = shared.patterns.to_vec();
    let mut keys = vec![];
    doc.visit_images(|_, im| keys.push(im.key.clone()));
    for k in keys {
        if let Some(blob) = shared.images.get(&k) {
            doc.images.insert(k, blob.clone());
        }
    }
    doc
}

/// How `live` and `drawn`, drawn over `region` at `scale`, differ: the pixels that differ in
/// lightness or colour, seen on white or on black (each is drawn on its own: white art shows only on
/// black), those whose neighbours all differ too (where one draws something the other doesn't, not
/// an edge drawn a little apart), and the pixels either inks. `None` when they can't be drawn.
fn differing(live: &Document, drawn: &Document, region: Rect, scale: f64) -> Option<(usize, usize, usize)> {
    let x = vectorcraft_render::Renderer::new().render_region(live, region, scale, false);
    let y = vectorcraft_render::Renderer::new().render_region(drawn, region, scale, false);
    if x.width == 0 || x.height == 0 || x.width != y.width || x.height != y.height {
        return None;
    }
    let (w, h) = (x.width as usize, x.height as usize);
    let (xs, ys) = (x.pixels.as_chunks::<4>().0, y.pixels.as_chunks::<4>().0);
    let differs: Vec<bool> = xs.iter().zip(ys).map(|(p, q)| differ(p, q) || differ_in_colour(p, q) || differ_on_black(p, q)).collect();
    let inked = xs.iter().zip(ys).filter(|(p, q)| p[3] > 8 || q[3] > 8).count();
    let at = |i: usize, j: usize| differs.get(j * w + i).copied().unwrap_or(false);
    let any = differs.iter().filter(|d| **d).count();
    let solid = (1..h.saturating_sub(1))
        .flat_map(|j| (1..w.saturating_sub(1)).map(move |i| (i, j)))
        .filter(|&(i, j)| (j - 1..=j + 1).all(|n| (i - 1..=i + 1).all(|m| at(m, n))))
        .count();
    Some((any, solid, inked))
}

/// The differences from their drawn art that the live envelopes and warped type of one import may
/// add up to, as shares of the page drawn as the page check draws it: half of what that check allows
/// ([`MAX_DIFFERENCE`], [`MAX_SOLID_DIFFERENCE`]), so they never make the file open from its PDF part
/// instead.
#[derive(Debug, Default)]
pub(crate) struct Spent {
    pub(super) any: f64,
    pub(super) solid: f64,
}

/// A text object read with a Warp.
#[derive(Debug)]
pub(super) struct WarpedLook {
    /// The id of its slot.
    pub(super) slot: NodeId,
    /// The drawn art its style's look saved.
    pub(super) look: Node,
    /// That art's box, when the comparisons with the page leave it out ([`super::Structure::restyled`]).
    pub(super) left_out: Option<Rect>,
}

/// An envelope of type read with its drawn art: its dictionary, the group on `%_` lines before it
/// and the art in its `AIDeformArtKey` block, as [`live`] takes them (their text slots still
/// empty), and the id of its drawn art in the document. Type is laid out after the layers are read,
/// so it waits for that ([`WarpedType::settle_envelopes`]).
#[derive(Debug)]
pub(crate) struct TypedEnvelope {
    pub(super) obj: Obj,
    pub(super) before: Option<Node>,
    pub(super) inner: Vec<Node>,
    pub(super) drawn: NodeId,
}

/// Why an envelope of type kept its drawn art.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TypeKept {
    /// Its type is set in a font that isn't available: the fallback font doesn't draw as the file.
    Font,
    /// A text slot of it has no type of its own (none made, or type a look draws, threaded type,
    /// type bound to a variable or a wrap object).
    Type,
    /// Its parts don't make a live envelope, or its drawn art is gone.
    Read,
    /// The import has no check left.
    Budget,
    /// Its live envelope doesn't draw as its drawn art.
    Differs,
}

/// What [`WarpedType::settle_envelopes`] did.
#[derive(Debug, Default)]
pub(crate) struct TypedSettled {
    /// How many envelopes of type opened live.
    pub(crate) live: usize,
    /// Why each of the others kept its drawn art.
    pub(crate) kept: Vec<TypeKept>,
    /// Each live envelope's id, with what takes its place again when the layers with it don't draw
    /// as the page: its hidden type, then its drawn art.
    pub(crate) put_back: Vec<(NodeId, Vec<Node>)>,
}

/// The text objects read with a Warp and the envelopes of type, and what the import has left of the
/// budget for checks ([`drawn_alike`]). Type is laid out after the layers are read, so these wait
/// for that ([`Self::settle`], [`Self::settle_envelopes`]).
#[derive(Debug, Default)]
pub struct WarpedType {
    pub(super) looks: Vec<WarpedLook>,
    pub(super) envelopes: Vec<TypedEnvelope>,
    pub(super) checks: usize,
    pub(super) spent: Spent,
    pub(super) page: Option<Rect>,
}

impl WarpedType {
    /// Are envelopes of type waiting for their type?
    pub(crate) fn has_envelopes(&self) -> bool {
        !self.envelopes.is_empty()
    }

    /// These, for layers `compared` with a page. Layers that aren't leave the envelopes of type
    /// their drawn art and their hidden type, with the note ([`TYPE_DRAWN`]) added to `notes`: a
    /// live envelope opens only where the page chooses it.
    pub(crate) fn compared(mut self, compared: bool, notes: &mut Vec<String>) -> Self {
        if !compared && !std::mem::take(&mut self.envelopes).is_empty() && !notes.iter().any(|n| n == TYPE_DRAWN) {
            notes.push(TYPE_DRAWN.into());
        }
        self
    }

    /// Make each envelope of type live where it draws as its drawn art, its contents' text slots
    /// holding their type: `type_of` gives, for a slot's id, the hidden text object it became in
    /// `doc` and the type the contents take in its place (the slot's visibility and lock are the
    /// type's). The live envelope takes its drawn art's place and the hidden type goes. A warp that
    /// doesn't draw as its drawn art is checked again as a mesh envelope on the grid the file saves
    /// for it ([`live_on_saved_grid`]), which opens in its place where it does: its type stays
    /// editable in that shape, and the warp's style and bend are lost. With Distort Appearance, each
    /// is also checked mapping from its contents' bounds with their strokes ([`with_strokes`]). Each
    /// check uses one of the import's checks and, for an envelope that shows, adds to what the page
    /// check allows ([`drawn_alike`]). Elsewhere, and where the type is set in a font that isn't
    /// available, the drawn art and the hidden type stay.
    pub(crate) fn settle_envelopes(&mut self, doc: &mut Document, type_of: impl Fn(&Document, NodeId) -> Option<(NodeId, Node)>) -> TypedSettled {
        let mut settled = TypedSettled::default();
        if self.envelopes.is_empty() {
            return settled;
        }
        let mut shown = BTreeSet::new();
        doc.layers.iter().for_each(|l| collect_shown(l, &mut shown));
        for envelope in std::mem::take(&mut self.envelopes) {
            match self.typed_live(doc, envelope, &type_of, &shown) {
                Ok(put_back) => {
                    settled.live += 1;
                    settled.put_back.push(put_back);
                }
                Err(why) => settled.kept.push(why),
            }
        }
        settled
    }

    /// [`Self::settle_envelopes`] for `envelope`, the shown objects of `doc` being `shown`: the live
    /// envelope's id and what takes its place again, or why its drawn art stays.
    fn typed_live(
        &mut self,
        doc: &mut Document,
        envelope: TypedEnvelope,
        type_of: &impl Fn(&Document, NodeId) -> Option<(NodeId, Node)>,
        shown: &BTreeSet<NodeId>,
    ) -> Result<(NodeId, Vec<Node>), TypeKept> {
        let TypedEnvelope { obj, mut before, mut inner, drawn } = envelope;
        let art = doc.node(drawn).cloned().ok_or(TypeKept::Read)?;
        // The text slots in the contents, in the order the reader put their hidden copies in.
        let mut slots = vec![];
        before.iter().chain(&inner).for_each(|n| text_slots_in(n, &mut slots));
        if slots.is_empty() {
            return Err(TypeKept::Type);
        }
        let db = vectorcraft_text::FontDb::global();
        let (mut texts, mut hidden) = (HashMap::new(), vec![]);
        for (slot, look) in slots {
            let (id, mut text) = type_of(doc, slot).filter(|_| !look).ok_or(TypeKept::Type)?;
            let NodeKind::Text(t) = &text.kind else { return Err(TypeKept::Type) };
            if t.runs.iter().any(|r| r.style.font_family.is_empty() || !db.has_family(&r.style.font_family)) {
                return Err(TypeKept::Font);
            }
            text.id = doc.alloc_id();
            texts.insert(slot, text);
            hidden.push(id);
        }
        for n in before.iter_mut().chain(inner.iter_mut()) {
            put_type(n, &texts);
        }
        // A warp that VectorCraft's own warp doesn't draw as its drawn art may still draw as the
        // grid the file saves for it: then it opens as a mesh envelope on that grid.
        let on_grid = if is_warp(&obj) { live_on_saved_grid(&obj, before.clone(), inner.clone()) } else { None };
        // With Distort Appearance, a file's envelope may map its contents from their bounds with
        // their strokes or without them, and for type the size the file saves for that box is often
        // out of date (not the size of the type there now): each is checked.
        let kinds: Vec<NodeKind> = live(&obj, before, inner)
            .into_iter()
            .chain(on_grid)
            .flat_map(|k| {
                let stroked = with_strokes(&k);
                std::iter::once(k).chain(stroked)
            })
            .collect();
        if kinds.is_empty() {
            return Err(TypeKept::Read);
        }
        let (id, layer) = (doc.alloc_id(), doc.alloc_id());
        let mut plain = art.clone();
        (plain.visible, plain.opacity) = (true, 1.0);
        let page = self.page.filter(|_| shown.contains(&drawn));
        let mut alike = None;
        for kind in kinds {
            if self.checks == 0 {
                return Err(TypeKept::Budget);
            }
            self.checks -= 1;
            let mut envelope = art.clone();
            (envelope.id, envelope.kind) = (id, kind);
            let shared = Shared { patterns: &doc.patterns, images: &doc.images };
            let mut live_one = envelope.clone();
            (live_one.visible, live_one.opacity) = (true, 1.0);
            if drawn_alike(&live_one, &plain, layer, &shared, page, &mut self.spent) {
                alike = Some(envelope);
                break;
            }
        }
        let envelope = alike.ok_or(TypeKept::Differs)?;
        if doc.node(drawn).is_none() {
            return Err(TypeKept::Read);
        }
        // The type is the envelope's now: its hidden copy goes, kept for when the drawn art comes back.
        let mut put_back: Vec<Node> = hidden.into_iter().filter_map(|id| doc.remove(id).ok()).map(Arc::unwrap_or_clone).collect();
        let id = envelope.id;
        let Some(slot) = doc.node_mut(drawn) else { return Err(TypeKept::Read) };
        *slot = envelope;
        put_back.push(art);
        Ok((id, put_back))
    }

    /// Replace each warped text object of `doc` that doesn't draw as its saved look (or that the
    /// budget has no check left for) with that look: `made` gives the text object a slot became. A
    /// frame in `threaded` stays type, since the frames of a thread share one story. A replaced
    /// object's box goes back into the comparisons with the page (out of `restyled`), and only what
    /// `doc` shows counts against the page's share of the budget. Returns how many were replaced.
    pub(crate) fn settle(
        self,
        doc: &mut Document,
        made: impl Fn(NodeId) -> Option<NodeId>,
        threaded: &BTreeSet<NodeId>,
        restyled: &mut Vec<Rect>,
    ) -> usize {
        let Self { looks, mut checks, mut spent, page, .. } = self;
        let mut shown_ids = BTreeSet::new();
        doc.layers.iter().for_each(|l| collect_shown(l, &mut shown_ids));
        let mut replaced = 0;
        for WarpedLook { slot, mut look, left_out } in looks {
            let Some(id) = made(slot).filter(|id| !threaded.contains(id)) else { continue };
            let Some(text) = doc.node(id).filter(|n| matches!(n.kind, NodeKind::Text(_)) && warps(&n.appearance)).cloned() else { continue };
            let mut alike = false;
            if checks > 0 {
                checks -= 1;
                let layer = doc.alloc_id();
                let shared = Shared { patterns: &doc.patterns, images: &doc.images };
                let (mut shown, mut plain) = (text.clone(), look.clone());
                (shown.visible, shown.opacity) = (true, 1.0);
                (plain.visible, plain.opacity) = (true, 1.0);
                alike = warped_alike(&shown, &plain, layer, &shared, page.filter(|_| shown_ids.contains(&id)), &mut spent);
            }
            if alike {
                continue;
            }
            (look.id, look.name, look.visible, look.locked) = (id, text.name.clone(), text.visible, text.locked);
            if let Some(wrap) = text.extras().wrap {
                look.extras_mut().wrap = Some(wrap);
            }
            if let Some(n) = doc.node_mut(id) {
                *n = look;
                replaced += 1;
                if let Some(at) = left_out.and_then(|b| restyled.iter().position(|r| *r == b)) {
                    restyled.remove(at);
                }
            }
        }
        replaced
    }
}

#[cfg(test)]
impl WarpedType {
    /// The text objects of `looks` (slot and saved look) with no check left.
    pub(crate) fn unchecked(looks: Vec<(NodeId, Node)>) -> Self {
        Self { looks: looks.into_iter().map(|(slot, look)| WarpedLook { slot, look, left_out: None }).collect(), ..Default::default() }
    }
}

/// The ids of the text slots in `n` and its groups, in order, each with whether it is a slot of type
/// that a look draws ([`super::is_look_slot`]).
fn text_slots_in(n: &Node, out: &mut Vec<(NodeId, bool)>) {
    let look = super::is_look_slot(n.name.as_deref());
    if look || super::slot_of(n.name.as_deref()).is_some() {
        out.push((n.id, look));
    } else if !matches!(n.kind, NodeKind::Compound { .. }) {
        n.children().into_iter().flatten().for_each(|c| text_slots_in(c, out));
    }
}

/// `n` with each text slot in it or its groups replaced by its type in `texts`, by the slot's id,
/// shown and locked as the slot is.
fn put_type(n: &mut Node, texts: &HashMap<NodeId, Node>) {
    if let Some(text) = texts.get(&n.id) {
        let (visible, locked) = (n.visible, n.locked);
        *n = text.clone();
        (n.visible, n.locked) = (visible, locked);
    } else if !matches!(n.kind, NodeKind::Compound { .. }) {
        n.children_mut().into_iter().flatten().for_each(|c| put_type(Arc::make_mut(c), texts));
    }
}

/// Add to `out` the ids of `n` and what is in it, unless hidden.
fn collect_shown(n: &Node, out: &mut BTreeSet<NodeId>) {
    if n.visible {
        out.insert(n.id);
        n.children().into_iter().flatten().for_each(|c| collect_shown(c, out));
    }
}

/// Does `appearance` have a Warp that shows, on the object or on one of its fills or strokes?
pub(super) fn warps(appearance: &Appearance) -> bool {
    let warp = |e: &Effect| e.visible && e.id.starts_with("warp.");
    appearance.effects.iter().any(warp) || appearance.items.iter().any(|i| i.visible() && i.effects().iter().any(warp))
}

/// Do `live` and `drawn` draw alike: few of the pixels either inks differ, and fewer still where a
/// whole patch differs; and, on `page` (when the file has one), do they differ little enough that
/// with the differences in `spent` the page check still takes the layers' art (it is added to
/// `spent` when they do)? Both are drawn on `layer`, a layer id no other node has.
pub(super) fn drawn_alike(live: &Node, drawn: &Node, layer: NodeId, shared: &Shared<'_>, page: Option<Rect>, spent: &mut Spent) -> bool {
    alike_within(live, drawn, live.visual_bounds(), layer, shared, page, spent)
}

/// [`drawn_alike`] for type under a Warp, compared over all that its effects draw, which can reach
/// past its own box.
pub(super) fn warped_alike(live: &Node, drawn: &Node, layer: NodeId, shared: &Shared<'_>, page: Option<Rect>, spent: &mut Spent) -> bool {
    let reach = vectorcraft_render::effect_bounds(&alone(live, layer, shared).symbol_boxes(), live);
    alike_within(live, drawn, vectorcraft_geom::union_opt(live.visual_bounds(), reach), layer, shared, page, spent)
}

/// [`drawn_alike`], with `live` measured as `reach`.
fn alike_within(live: &Node, drawn: &Node, reach: Option<Rect>, layer: NodeId, shared: &Shared<'_>, page: Option<Rect>, spent: &mut Spent) -> bool {
    let (Some(a), Some(b)) = (reach, drawn.visual_bounds()) else { return false };
    let region = a.union(b).inflate(2.0, 2.0);
    if !(region.is_finite() && region.width() > 0.0 && region.height() > 0.0) || region.width().max(region.height()) > 1e6 {
        return false;
    }
    let (x, y) = (alone(live, layer, shared), alone(drawn, layer, shared));
    let Some((any, solid, inked)) = differing(&x, &y, region, compare_scale(region)) else { return false };
    if inked == 0 {
        return false;
    }
    let share = |n: usize| n as f64 / inked as f64;
    if share(solid) > MAX_SOLID || share(any) > MAX_ANY {
        return false;
    }
    let Some(page) = page.filter(|p| p.is_finite() && p.width() > 0.0 && p.height() > 0.0) else { return true };
    // Only what is on the page counts, drawn as the page check draws it (about 400 pixels along the
    // page's longer side).
    let on_page = region.intersect(page);
    let scale = compare_scale(page);
    if on_page.width() * scale < 1.0 || on_page.height() * scale < 1.0 {
        return true;
    }
    let Some((any, solid, _)) = differing(&x, &y, on_page, scale) else { return false };
    let pixels = (page.width() * scale * page.height() * scale).max(1.0);
    let (any, solid) = (spent.any + any as f64 / pixels, spent.solid + solid as f64 / pixels);
    if any > MAX_DIFFERENCE / 2.0 || solid > MAX_SOLID_DIFFERENCE / 2.0 {
        return false;
    }
    *spent = Spent { any, solid };
    true
}
