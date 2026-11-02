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
//! `DeformVert`, fractions) and orientation (`Rotate`), the contents (`X=` … `X+`, then
//! `(AIDeformArtKey)`) and the kind (`AIDeformStyle`: 1 a warp, 2 a mesh, 3 a top object). Then the
//! distorted art on plain lines, and the plugin's `XP`. While the contents are edited, the group
//! holds the contents and the dictionary the mesh. A top object's outline is the edge of its
//! one-patch mesh. An envelope whose parts don't read, or that doesn't come out as drawn, keeps its
//! drawn art.

use std::collections::BTreeMap;
use std::sync::Arc;

use kurbo::{BezPath, Point, Rect, Vec2};
use vectorcraft_doc::live::{EnvelopeKind, EnvelopeOptions, GradientMesh, H_DOWN, H_LEFT, H_RIGHT, H_UP, PreserveShape, default_fidelity};
use vectorcraft_doc::{Document, ImageBlob, LayerColor, Node, NodeId, NodeKind, PatternDef};
use vectorcraft_geom::PathData;

use super::obj::Obj;
use crate::import::native::compare::{MAX_DIFFERENCE, MAX_SOLID_DIFFERENCE, compare_scale, differ, differ_in_colour};

/// The plugin that writes envelopes.
pub(super) const PLUGIN: &str = "Adobe Deform Plugin"; // brand-ok: the plugin name the files use
/// The note for an envelope that keeps its drawn art.
pub(crate) const DRAWN: &str = "an envelope VectorCraft couldn't read as a live envelope comes in as its drawn art";
/// The most envelopes waiting for their drawn art at once.
pub(super) const MAX_PENDING: usize = 64;
/// The most envelopes one import draws to check; more keep their drawn art.
pub(super) const MAX_CHECKED: usize = 256;
/// The most rows or columns of a mesh read (Make with Mesh's own limit).
const MAX_GRID: u32 = 50;
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
    };
    Some((options, fidelity))
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
    // Columns that run down the page: the grid goes the other way round.
    let along = at(&pts, cols, 0, cols) - at(&pts, cols, 0, 0);
    if along.y.abs() > along.x.abs() {
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
    if (at(&pts, cols, 0, cols) - at(&pts, cols, 0, 0)).x < 0.0 {
        for row in pts.chunks_mut(cols as usize + 1) {
            row.reverse();
            for (_, h) in row.iter_mut() {
                h.swap(H_RIGHT, H_LEFT);
            }
        }
    }
    if (at(&pts, cols, rows, 0) - at(&pts, cols, 0, 0)).y < 0.0 {
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
    let editing = switch(o, "AIDeformEditModeKey")?;
    let (options, fidelity) = options(o)?;
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
    let kind = match whole(o, "AIDeformStyle")?? {
        1 => warp(o)?,
        2 => {
            let (rows, cols, pts) = upright(shape?)?;
            let (points, handles) = pts.into_iter().unzip();
            EnvelopeKind::Mesh { rows, cols, points, handles }
        }
        3 => {
            let (rows, cols, pts) = upright(shape?)?;
            EnvelopeKind::TopObject { path: outline(rows, cols, &pts)? }
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
/// lightness or colour, those whose neighbours all differ too (where one draws something the other
/// doesn't, not an edge drawn a little apart), and the pixels either inks. `None` when they can't
/// be drawn.
fn differing(live: &Document, drawn: &Document, region: Rect, scale: f64) -> Option<(usize, usize, usize)> {
    let x = vectorcraft_render::Renderer::new().render_region(live, region, scale, false);
    let y = vectorcraft_render::Renderer::new().render_region(drawn, region, scale, false);
    if x.width == 0 || x.height == 0 || x.width != y.width || x.height != y.height {
        return None;
    }
    let (w, h) = (x.width as usize, x.height as usize);
    let (xs, ys) = (x.pixels.as_chunks::<4>().0, y.pixels.as_chunks::<4>().0);
    let differs: Vec<bool> = xs.iter().zip(ys).map(|(p, q)| differ(p, q) || differ_in_colour(p, q)).collect();
    let inked = xs.iter().zip(ys).filter(|(p, q)| p[3] > 8 || q[3] > 8).count();
    let at = |i: usize, j: usize| differs.get(j * w + i).copied().unwrap_or(false);
    let any = differs.iter().filter(|d| **d).count();
    let solid = (1..h.saturating_sub(1))
        .flat_map(|j| (1..w.saturating_sub(1)).map(move |i| (i, j)))
        .filter(|&(i, j)| (j - 1..=j + 1).all(|n| (i - 1..=i + 1).all(|m| at(m, n))))
        .count();
    Some((any, solid, inked))
}

/// The differences from their drawn art that the live envelopes of one import may add up to, as
/// shares of the page drawn as the page check draws it: half of what that check allows
/// ([`MAX_DIFFERENCE`], [`MAX_SOLID_DIFFERENCE`]), so live envelopes never make the file open from
/// its PDF part instead.
#[derive(Debug, Default)]
pub(super) struct Spent {
    any: f64,
    solid: f64,
}

/// Do `live` and `drawn` draw alike: few of the pixels either inks differ, and fewer still where a
/// whole patch differs; and, on `page` (when the file has one), do they differ little enough that
/// with the differences in `spent` the page check still takes the layers' art (it is added to
/// `spent` when they do)? Both are drawn on `layer`, a layer id no other node has.
pub(super) fn drawn_alike(live: &Node, drawn: &Node, layer: NodeId, shared: &Shared<'_>, page: Option<Rect>, spent: &mut Spent) -> bool {
    let (Some(a), Some(b)) = (live.visual_bounds(), drawn.visual_bounds()) else { return false };
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
