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
//! holds the contents and the dictionary the mesh, which in some files lies away from the drawn art
//! by a move, as if it hadn't followed the envelope: such a mesh or top object whose grid draws its
//! contents as large as the drawn art but elsewhere is tried again moved onto that art. A top
//! object's outline is the edge of its one-patch mesh. A warp's mesh is a grid of its shape: a warp
//! of type whose own warp doesn't come out as drawn opens as a mesh envelope on that grid where that
//! does. An envelope whose parts don't read, or that doesn't come out as drawn, keeps its drawn art.

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
/// How far, in points over width and height together, what an envelope draws may differ in size
/// from its drawn art and still count as that art moved, and how far it must lie from it to count
/// as moved at all ([`moved_onto`]).
const MOVED_SLACK: f64 = 0.5;
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

/// The box of `content` an envelope of type with Distort Appearance maps from in a file: type by
/// its glyph outlines grown by as far as its strokes paint along a side (half the weight of a
/// centred stroke), without what miter joins add at corners, as are groups by their own strokes;
/// other objects by their bounds with strokes. `None` when nothing in it has bounds.
fn half_stroked_box(content: &[Arc<Node>]) -> Option<Rect> {
    /// How far the visible strokes of `appearance` paint along the sides of closed outlines.
    fn along(appearance: &Appearance) -> f64 {
        appearance.painted_strokes().map(|s| s.side_reach(true)).fold(0.0, f64::max)
    }
    /// `r` grown by `by` on each side, where that is a distance.
    fn grown(r: Rect, by: f64) -> Rect {
        if by.is_finite() && by > 0.0 { r.inflate(by, by) } else { r }
    }
    fn walk(n: &Arc<Node>, acc: Option<Rect>) -> Option<Rect> {
        let one = std::slice::from_ref(n);
        match &n.kind {
            NodeKind::Text(t) => {
                let Some((_, ink)) = envelope_content_boxes(one, Affine::IDENTITY) else { return acc };
                // The characters' own strokes as the type's bounds with strokes measure them (they
                // have no miter allowance there), and the strokes of its appearance along a side.
                vectorcraft_geom::union_opt(acc, Some(grown(ink, t.char_stroke_outset().max(along(&n.appearance)))))
            }
            NodeKind::Group { children, clip: false } => {
                let members = children.iter().fold(None, |acc, c| walk(c, acc));
                vectorcraft_geom::union_opt(acc, members.map(|b| grown(b, along(&n.appearance))))
            }
            _ => vectorcraft_geom::union_opt(acc, envelope_content_boxes(one, Affine::IDENTITY).map(|(stroked, _)| stroked)),
        }
    }
    content.iter().fold(None, |acc, c| walk(c, acc))
}

/// Envelope `kind` mapping from its contents' glyphs grown by as far as their strokes paint along a
/// side ([`half_stroked_box`]), for an envelope with Distort Appearance whose contents have strokes it
/// doesn't map from already; `None` otherwise.
fn with_strokes(kind: &NodeKind) -> Option<NodeKind> {
    let NodeKind::Envelope { content, options, .. } = kind else { return None };
    if !options.distort_appearance {
        return None;
    }
    let (_, plain) = envelope_content_boxes(content, Affine::IDENTITY)?;
    let margin = envelope_stroke_margin(half_stroked_box(content)?, plain)?;
    if margin == options.bounds_margin || margin.iter().any(|m| *m < 0.0) || margin.iter().all(|m| *m == 0.0) {
        return None;
    }
    let mut kind = kind.clone();
    if let NodeKind::Envelope { options, .. } = &mut kind {
        options.bounds_margin = margin;
    }
    Some(kind)
}

/// How far, in points, the sides of two boxes may lie apart and still count as the same side: the
/// glyphs' box (from type's ink) and the box of what type paints (from its outlines) measure one
/// outline two ways, and where they meet they differ by rounding only ([`painted_margin`],
/// [`same_box`]).
const PAINTED_SLACK: f64 = 1e-4;

/// The box of what the pieces of type's outlines `outlines` ([`vectorcraft_effects::outline_text`])
/// paint as an envelope with Distort Appearance bends them: each piece baked as the envelope bakes
/// it ([`vectorcraft_effects::bake_type_appearance`]: geometry effects on its fills and strokes
/// applied, so a stroke a Transform effect moves counts where it is moved to), measured by its
/// visual bounds. Those are exact for miter joins (a tip counts where the miter limit keeps it) and
/// round joins, and an upper bound for bevel joins, miters the limit cuts, width profiles and
/// square-capped dashes; a contour of zero length, which paints nothing, doesn't count. Effects on
/// the type object itself are left out: VectorCraft applies them after the envelope bends the type.
/// `None` when nothing in it has bounds.
fn painted_outlines(outlines: &Node) -> Option<Rect> {
    outlines.children().into_iter().flatten().fold(None, |acc, piece| {
        let baked = vectorcraft_effects::bake_type_appearance(None, piece);
        vectorcraft_geom::union_opt(acc, baked.as_ref().unwrap_or(piece).visual_bounds())
    })
}

/// The box of `content` an envelope of type with Distort Appearance may map from in a file besides
/// [`half_stroked_box`]: what it paints, type by its outlines ([`painted_outlines`]), groups by
/// their members grown by as far as their own strokes paint along a side, other objects by their
/// bounds with strokes. `None` when nothing in it has bounds.
fn painted_box(content: &[Arc<Node>]) -> Option<Rect> {
    fn walk(n: &Arc<Node>, acc: Option<Rect>) -> Option<Rect> {
        match &n.kind {
            NodeKind::Text(_) => vectorcraft_geom::union_opt(acc, vectorcraft_effects::outline_text(n).as_ref().and_then(painted_outlines)),
            NodeKind::Group { children, clip: false } => {
                let along = n.appearance.painted_strokes().map(|s| s.side_reach(true)).fold(0.0, f64::max);
                let members = children.iter().fold(None, |acc, c| walk(c, acc));
                let grown = |r: Rect| if along.is_finite() && along > 0.0 { r.inflate(along, along) } else { r };
                vectorcraft_geom::union_opt(acc, members.map(grown))
            }
            _ => vectorcraft_geom::union_opt(acc, envelope_content_boxes(std::slice::from_ref(n), Affine::IDENTITY).map(|(stroked, _)| stroked)),
        }
    }
    content.iter().fold(None, |acc, c| walk(c, acc))
}

/// The bounds margin of `painted`, the box of what contents with geometric box `plain` paint, a side
/// within [`PAINTED_SLACK`] of the geometric box's counting as that side. `None` where it lies
/// inside the geometric box on a side, where it is that box, and for margins that don't fit.
fn painted_margin(painted: Rect, plain: Rect) -> Option<[f32; 4]> {
    let side = |p: f64, g: f64| if (p - g).abs() <= PAINTED_SLACK { g } else { p };
    let painted = Rect::new(side(painted.x0, plain.x0), side(painted.y0, plain.y0), side(painted.x1, plain.x1), side(painted.y1, plain.y1));
    let margin = envelope_stroke_margin(painted, plain)?;
    (margin.iter().all(|m| *m >= 0.0) && margin.iter().any(|m| *m > 0.0)).then_some(margin)
}

/// Do bounds margins `a` and `b` give contents with geometric box `plain` the same box, each side
/// within [`PAINTED_SLACK`]?
fn same_box(a: [f32; 4], b: [f32; 4], plain: Rect) -> bool {
    let sizes = [plain.height(), plain.height(), plain.width(), plain.width()];
    a.iter().zip(b).zip(sizes).all(|((a, b), size)| (f64::from(*a) - f64::from(b)).abs() * size <= PAINTED_SLACK)
}

/// What the contents of an envelope of type paint ([`painted_box`]): their geometric box and the
/// bounds margin of what they paint from it.
#[derive(Clone, Copy)]
struct Painted {
    plain: Rect,
    margin: [f32; 4],
}

/// [`Painted`] for envelope `kind` with Distort Appearance; `None` without it, and where
/// [`painted_margin`] gives none.
fn painted_of(kind: &NodeKind) -> Option<Painted> {
    let NodeKind::Envelope { content, options, .. } = kind else { return None };
    if !options.distort_appearance {
        return None;
    }
    let (_, plain) = envelope_content_boxes(content, Affine::IDENTITY)?;
    Some(Painted { plain, margin: painted_margin(painted_box(content)?, plain)? })
}

/// Envelope `kind` mapping from what its contents paint (`painted`, [`painted_of`]), for an
/// envelope with Distort Appearance where that box is neither the one it maps from already nor its
/// contents' glyphs grown by half their strokes' weight ([`with_strokes`]); `None` otherwise.
fn as_painted(kind: &NodeKind, painted: &Painted) -> Option<NodeKind> {
    let NodeKind::Envelope { options, .. } = kind else { return None };
    let half = match with_strokes(kind) {
        Some(NodeKind::Envelope { options, .. }) => Some(options.bounds_margin),
        _ => None,
    };
    let Painted { plain, margin } = *painted;
    if !options.distort_appearance || same_box(margin, options.bounds_margin, plain) || half.is_some_and(|h| same_box(margin, h, plain)) {
        return None;
    }
    let mut kind = kind.clone();
    if let NodeKind::Envelope { options, .. } = &mut kind {
        options.bounds_margin = margin;
    }
    Some(kind)
}

/// A way an envelope of type is tried: a kind, or a kind mapping from what its type paints
/// ([`as_painted`]), worked out when its turn comes.
enum Way {
    As(NodeKind),
    Painted(NodeKind),
}

/// The kind of `envelope`, a mesh or a top object in Edit Contents, with its grid moved onto
/// `drawn`, the drawn art the file saves for it, where what it draws is as large as that art (within
/// [`MOVED_SLACK`]) but lies elsewhere (further than that): moved by as far as the drawn art's box
/// lies from the box of what it draws. While the contents are edited, the grid is kept in the
/// envelope's dictionary, and in some files it lies away from the drawn art by a move, the contents
/// with the drawn art, as if the grid hadn't followed the envelope when it moved. `None` outside
/// Edit Contents, for a warp (it maps from its contents' box, which lies with them), and where
/// nothing is drawn. Symbol instances and pattern fills in the contents are measured as they are,
/// not drawn out: where that makes the sizes differ, no move is tried.
pub(super) fn moved_onto(envelope: &Node, drawn: &Node) -> Option<NodeKind> {
    let NodeKind::Envelope { kind, editing: true, .. } = &envelope.kind else { return None };
    if matches!(**kind, EnvelopeKind::Warp { .. }) {
        return None;
    }
    // What it draws, measured as its drawn art is: the pieces in a group with its own appearance.
    let mut drew = envelope.clone();
    drew.kind = NodeKind::Group { children: vectorcraft_render::expand_live(envelope).into_iter().map(Arc::new).collect(), clip: false };
    let (from, to) = (drew.visual_bounds()?, drawn.visual_bounds()?);
    let by = to.center() - from.center();
    let resized = (from.width() - to.width()).abs() + (from.height() - to.height()).abs();
    if !(resized <= MOVED_SLACK && by.x.abs() + by.y.abs() > MOVED_SLACK && by.x.is_finite() && by.y.is_finite()) {
        return None;
    }
    let mut moved = envelope.kind.clone();
    if let NodeKind::Envelope { kind, .. } = &mut moved {
        match &mut **kind {
            EnvelopeKind::Mesh { points, .. } => points.iter_mut().for_each(|p| *p += by),
            EnvelopeKind::TopObject { path, .. } => path.transform(Affine::translate(by)),
            EnvelopeKind::Warp { .. } => return None,
        }
    }
    Some(moved)
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
        version: vectorcraft_doc::live::WARP_NET,
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
    // How far the grid reaches: a run much shorter than that is rounding, not a direction.
    let reach = {
        let first = pts.first().map_or(Point::ZERO, |q| q.0);
        let bounds = pts.iter().fold(Rect::from_points(first, first), |b, q| b.union_pt(q.0));
        bounds.width().hypot(bounds.height()) * 1e-6
    };
    // How the grid runs along its rows (every row end to end, added up) and down its columns (every
    // column): read from all of them, a grid bent far round (its end columns leaning more than
    // halfway over) still reads the way it runs. Each sum comes with how long its parts are: where
    // they nearly cancel out, as in a ring, what is left is rounding, not a direction, and the grid
    // reads by its first row or column instead.
    let runs = |pts: &[GridPoint], rows: u32, cols: u32| {
        let (mut along, mut down, mut along_len, mut down_len) = (Vec2::ZERO, Vec2::ZERO, 0.0, 0.0);
        for r in 0..=rows {
            let v = at(pts, cols, r, cols) - at(pts, cols, r, 0);
            (along, along_len) = (along + v, along_len + v.hypot());
        }
        for c in 0..=cols {
            let v = at(pts, cols, rows, c) - at(pts, cols, 0, c);
            (down, down_len) = (down + v, down_len + v.hypot());
        }
        let clear = |v: Vec2, len: f64| (v.hypot() >= len * 0.01 && v.hypot() > reach).then_some(v);
        let first_row = at(pts, cols, 0, cols) - at(pts, cols, 0, 0);
        let first_col = at(pts, cols, rows, 0) - at(pts, cols, 0, 0);
        (clear(along, along_len).unwrap_or(first_row), clear(down, down_len).unwrap_or(first_col))
    };
    // How much of a run is across the page.
    let level = |v: Vec2| if v.hypot() > 0.0 { v.x.abs() / v.hypot() } else { 0.0 };
    // Columns that run across the page more than rows do: the grid goes the other way round. Nearly
    // as much (a grid turned an eighth round) keeps the file's way round.
    let (along, down) = runs(&pts, rows, cols);
    if level(down) > level(along) + 1e-3 {
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
    /// is also checked mapping from its type's glyphs grown by half their strokes' weight
    /// ([`with_strokes`]) and, after all of those, from what its type paints where that is another
    /// box ([`as_painted`]). In Edit Contents, a mesh or top object whose grid draws the type
    /// elsewhere than its drawn art is checked last moved onto that art ([`moved_onto`]). Each
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
        let envelopes = std::mem::take(&mut self.envelopes);
        let mut waiting = envelopes.len();
        for envelope in envelopes {
            waiting -= 1;
            match self.typed_live(doc, envelope, &type_of, &shown, waiting) {
                Ok(put_back) => {
                    settled.live += 1;
                    settled.put_back.push(put_back);
                }
                Err(why) => settled.kept.push(why),
            }
        }
        settled
    }

    /// [`Self::settle_envelopes`] for `envelope`, the shown objects of `doc` being `shown` and
    /// `waiting` envelopes of type still to come: the live envelope's id and what takes its place
    /// again, or why its drawn art stays.
    fn typed_live(
        &mut self,
        doc: &mut Document,
        envelope: TypedEnvelope,
        type_of: &impl Fn(&Document, NodeId) -> Option<(NodeId, Node)>,
        shown: &BTreeSet<NodeId>,
        waiting: usize,
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
        // With Distort Appearance, a file's envelope may map its type from its glyphs grown by half
        // their strokes' weight or from the glyphs alone, and for type the size the file saves for
        // that box is often out of date (not the size of the type there now): each is checked.
        // After all of those, each is checked mapping from what its type paints, where that box is
        // another: it is measured only when such a check comes and one is left.
        let kinds: Vec<NodeKind> = live(&obj, before, inner).into_iter().chain(on_grid).collect();
        if kinds.is_empty() {
            return Err(TypeKept::Read);
        }
        let mut ways: Vec<Way> = kinds.iter().flat_map(|k| std::iter::once(Way::As(k.clone())).chain(with_strokes(k).map(Way::As))).collect();
        ways.extend(kinds.into_iter().map(Way::Painted));
        // What the type paints, measured once: the kinds share their contents.
        let mut painted: Option<Option<Painted>> = None;
        let (id, layer) = (doc.alloc_id(), doc.alloc_id());
        let mut plain = art.clone();
        (plain.visible, plain.opacity) = (true, 1.0);
        let page = self.page.filter(|_| shown.contains(&drawn));
        let mut alike = None;
        // In Edit Contents, a grid that draws the type elsewhere than its drawn art is tried again
        // moved onto that art ([`moved_onto`]), after every other way.
        let (mut tries, mut moved, mut again, mut i) = (ways.into_iter(), vec![], false, 0);
        loop {
            let Some(way) = tries.next() else {
                if again || moved.is_empty() {
                    break;
                }
                (tries, again) = (std::mem::take(&mut moved).into_iter(), true);
                continue;
            };
            // A second try is only made while more checks are left than there are envelopes of
            // type and warped text objects still to come (some of which may need none): one
            // envelope's tries don't use up the checks of the others.
            if self.checks == 0 || (i > 0 && self.checks <= waiting + self.looks.len()) {
                return Err(TypeKept::Budget);
            }
            let kind = match way {
                Way::As(kind) => kind,
                Way::Painted(base) => match painted.get_or_insert_with(|| painted_of(&base)).as_ref().and_then(|p| as_painted(&base, p)) {
                    Some(kind) => kind,
                    None => continue,
                },
            };
            self.checks -= 1;
            i += 1;
            let mut envelope = art.clone();
            (envelope.id, envelope.kind) = (id, kind);
            let shared = Shared { patterns: &doc.patterns, images: &doc.images };
            let mut live_one = envelope.clone();
            (live_one.visible, live_one.opacity) = (true, 1.0);
            if drawn_alike(&live_one, &plain, layer, &shared, page, &mut self.spent) {
                alike = Some(envelope);
                break;
            }
            if !again && let Some(kind) = moved_onto(&live_one, &plain) {
                moved.push(Way::As(kind));
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

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_color::{Color, Paint};
    use vectorcraft_doc::{AppearanceItem, CharStyle, LineCap, LineJoin, StrokeLayer, TextObject};

    /// An envelope of `content` with Distort Appearance, on a flat one-patch mesh.
    fn envelope_of(content: Vec<Arc<Node>>) -> NodeKind {
        let points = vec![Point::new(0.0, 0.0), Point::new(100.0, 0.0), Point::new(0.0, 50.0), Point::new(100.0, 50.0)];
        NodeKind::Envelope {
            content,
            kind: Box::new(EnvelopeKind::Mesh { rows: 1, cols: 1, points, handles: vec![] }),
            fidelity: 50.0,
            editing: false,
            options: EnvelopeOptions::NEW,
            frame: Box::default(),
        }
    }

    /// The bounds margin of the stroked candidate for an envelope of `content`.
    fn stroked_margin(content: Vec<Arc<Node>>) -> Option<[f32; 4]> {
        match with_strokes(&envelope_of(content)) {
            Some(NodeKind::Envelope { options, .. }) => Some(options.bounds_margin),
            _ => None,
        }
    }

    /// "AVA" at 40 points, at (10, 60).
    fn type_node(id: u64, style: CharStyle) -> Node {
        Node::new(NodeId(id), NodeKind::Text(Box::new(TextObject::point(Point::new(10.0, 60.0), "AVA", CharStyle { size: 40.0, ..style }))))
    }

    /// A 4 point stroke with miter joins at a limit of 4.
    fn mitred() -> StrokeLayer {
        let mut stroke = StrokeLayer::new(Paint::solid(Color::BLACK), 4.0);
        (stroke.join, stroke.miter_limit) = (LineJoin::Miter, 4.0);
        stroke
    }

    /// Type with Distort Appearance and a stroke from its appearance is tried mapped from its glyphs
    /// grown by half the stroke's weight, not from its bounds with strokes, which reach as far as a
    /// miter join may at a corner.
    #[test]
    fn stroked_type_is_tried_mapped_from_its_glyphs_grown_by_half_its_stroke() {
        let mut text = type_node(1, CharStyle::default());
        let mut appearance = (*text.appearance).clone();
        appearance.items.push(AppearanceItem::Stroke(mitred()));
        text.appearance = appearance.into();
        let content = vec![Arc::new(text)];
        let (with, glyphs) = envelope_content_boxes(&content, Affine::IDENTITY).unwrap();
        let half = glyphs.inflate(2.0, 2.0);
        assert!(with.width() > half.width() + 1.0, "{with:?} {half:?}");
        assert_eq!(stroked_margin(content), envelope_stroke_margin(half, glyphs));
    }

    /// Type stroked by its characters alone is tried mapped from its bounds with strokes, as before,
    /// turned and stretched as well as upright.
    #[test]
    fn type_with_a_character_stroke_is_tried_mapped_from_its_bounds_with_strokes() {
        let style = CharStyle { stroke: Paint::solid(Color::BLACK), stroke_width: 4.0, ..CharStyle::default() };
        for xf in [
            Affine::IDENTITY,
            Affine::rotate(std::f64::consts::FRAC_PI_4),
            Affine::scale_non_uniform(std::f64::consts::SQRT_2, std::f64::consts::FRAC_1_SQRT_2),
        ] {
            let mut text = type_node(1, style.clone());
            if let NodeKind::Text(t) = &mut text.kind {
                t.xf = xf * t.xf;
            }
            let content = vec![Arc::new(text)];
            let (with, glyphs) = envelope_content_boxes(&content, Affine::IDENTITY).unwrap();
            assert_eq!(stroked_margin(content), envelope_stroke_margin(with, glyphs), "{xf:?}");
        }
    }

    /// A stroke on a group's own appearance round type grows the box by half its weight too.
    #[test]
    fn a_stroke_on_a_group_of_type_grows_the_box_by_half_its_weight() {
        let mut group = Node::new(NodeId(2), NodeKind::Group { children: vec![Arc::new(type_node(1, CharStyle::default()))], clip: false });
        let mut appearance = (*group.appearance).clone();
        appearance.items.push(AppearanceItem::Stroke(mitred()));
        group.appearance = appearance.into();
        let content = vec![Arc::new(group)];
        let (_, glyphs) = envelope_content_boxes(&content, Affine::IDENTITY).unwrap();
        assert_eq!(stroked_margin(content), envelope_stroke_margin(glyphs.inflate(2.0, 2.0), glyphs));
    }

    /// Is the font type takes by default installed? Type that must draw as itself is tested only
    /// where it is: the tests below measure its glyphs.
    fn default_font() -> bool {
        vectorcraft_text::FontDb::global().has_family(&CharStyle::default().font_family)
    }

    /// The bounds margin of the candidate mapping from what `content` paints, for an envelope of it,
    /// type measured by its glyphs as the import measures it; `None` where there is none.
    fn painted_candidate(content: Vec<Arc<Node>>) -> Option<[f32; 4]> {
        vectorcraft_text::register_envelope_ink();
        let kind = envelope_of(content);
        match as_painted(&kind, &painted_of(&kind)?) {
            Some(NodeKind::Envelope { options, .. }) => Some(options.bounds_margin),
            _ => None,
        }
    }

    /// The box contents with geometric box `plain` map from with bounds margin `m`.
    fn box_of(m: [f32; 4], plain: Rect) -> Rect {
        let [top, bottom, left, right] = m.map(f64::from);
        Rect::new(
            plain.x0 - left * plain.width(),
            plain.y0 - top * plain.height(),
            plain.x1 + right * plain.width(),
            plain.y1 + bottom * plain.height(),
        )
    }

    /// Are the sides of `a` and `b` within `tol` of each other?
    fn close(a: Rect, b: Rect, tol: f64) -> bool {
        [a.x0 - b.x0, a.y0 - b.y0, a.x1 - b.x1, a.y1 - b.y1].iter().all(|d| d.abs() <= tol)
    }

    /// The geometric box of `content`, type measured by its glyphs as the import measures it.
    fn glyphs_of(content: &[Arc<Node>]) -> (Rect, Rect) {
        vectorcraft_text::register_envelope_ink();
        envelope_content_boxes(content, Affine::IDENTITY).unwrap()
    }

    /// What the type in `content` paints, measured apart from the import's own measure: each piece
    /// of its outlines filled, and outlined by kurbo's stroker with each of its strokes' weight,
    /// join and miter limit.
    fn stroked_by_kurbo(content: &[Arc<Node>]) -> Rect {
        use kurbo::Shape;
        let mut out: Option<Rect> = None;
        for n in content {
            let outlines = vectorcraft_effects::outline_text(n).unwrap();
            for piece in outlines.children().unwrap() {
                let bp = piece.stroke_path().unwrap();
                out = vectorcraft_geom::union_opt(out, Some(bp.bounding_box()));
                for s in piece.appearance.painted_strokes() {
                    let join = match s.join {
                        LineJoin::Miter => kurbo::Join::Miter,
                        LineJoin::Round => kurbo::Join::Round,
                        LineJoin::Bevel => kurbo::Join::Bevel,
                    };
                    let stroke = kurbo::Stroke::new(s.width).with_join(join).with_miter_limit(s.miter_limit);
                    let outline = kurbo::stroke(bp.iter(), &stroke, &kurbo::StrokeOpts::default(), 1e-4);
                    out = vectorcraft_geom::union_opt(out, Some(outline.bounding_box()));
                }
            }
        }
        out.unwrap()
    }

    /// Type with Distort Appearance and a stroke with miter joins on its appearance is also tried
    /// mapped from what that stroke paints: past half its weight where a miter tip pokes out at a
    /// sharp corner (the feet of the A's), though not as far as its bounds with strokes, which allow
    /// for a miter as long as the limit lets one be at every corner.
    #[test]
    fn mitred_type_is_also_tried_mapped_from_what_its_stroke_paints_miter_tips_included() {
        if !default_font() {
            return;
        }
        let mut text = type_node(1, CharStyle::default());
        let mut appearance = (*text.appearance).clone();
        appearance.items.push(AppearanceItem::Stroke(mitred()));
        text.appearance = appearance.into();
        let content = vec![Arc::new(text)];
        let (with, glyphs) = glyphs_of(&content);
        let (half, painted) = (glyphs.inflate(2.0, 2.0), stroked_by_kurbo(&content));
        assert!(painted.x0 < half.x0 - 0.5 && painted.x1 > half.x1 + 0.5, "{painted:?} {half:?}");
        assert!(painted.width() < with.width() - 0.5, "{painted:?} {with:?}");
        let from = box_of(painted_candidate(content).unwrap(), glyphs);
        assert!(close(from, painted, 0.01), "{from:?} {painted:?}");
    }

    /// Type stroked by its characters with miter joins is also tried mapped from what those strokes
    /// paint, turned and stretched as well as upright.
    #[test]
    fn type_with_a_mitred_character_stroke_is_also_tried_mapped_from_what_it_paints() {
        if !default_font() {
            return;
        }
        let style = CharStyle { stroke: Paint::solid(Color::BLACK), stroke_width: 4.0, stroke_join: LineJoin::Miter, ..CharStyle::default() };
        for xf in [
            Affine::IDENTITY,
            Affine::rotate(std::f64::consts::FRAC_PI_4),
            Affine::scale_non_uniform(std::f64::consts::SQRT_2, std::f64::consts::FRAC_1_SQRT_2),
        ] {
            let mut text = type_node(1, style.clone());
            if let NodeKind::Text(t) = &mut text.kind {
                t.xf = xf * t.xf;
            }
            let content = vec![Arc::new(text)];
            let (_, glyphs) = glyphs_of(&content);
            let painted = stroked_by_kurbo(&content);
            let from = box_of(painted_candidate(content).unwrap(), glyphs);
            assert!(close(from, painted, 0.01), "{xf:?}: {from:?} {painted:?}");
        }
    }

    /// A stroke that a Transform effect moves paints where it is moved to: type filled over its
    /// glyphs with a round 4 point stroke moved 10 points down is also tried mapped from its glyphs
    /// grown by 2 points on the left and right and 12 at the bottom, and not at the top.
    #[test]
    fn a_stroke_moved_by_a_transform_effect_counts_where_it_paints() {
        if !default_font() {
            return;
        }
        let mut text = type_node(1, CharStyle::default());
        let mut stroke = StrokeLayer::new(Paint::solid(Color::BLACK), 4.0);
        (stroke.join, stroke.cap) = (LineJoin::Round, LineCap::Round);
        stroke.effects.push(Effect { id: "distort.transform".into(), params: serde_json::json!({ "moveV": 10.0 }), visible: true });
        let mut appearance = (*text.appearance).clone();
        appearance.items.push(AppearanceItem::Stroke(stroke));
        text.appearance = appearance.into();
        let content = vec![Arc::new(text)];
        let (_, glyphs) = glyphs_of(&content);
        let painted = Rect::new(glyphs.x0 - 2.0, glyphs.y0, glyphs.x1 + 2.0, glyphs.y1 + 12.0);
        let from = box_of(painted_candidate(content).unwrap(), glyphs);
        assert!(close(from, painted, 0.01), "{from:?} {painted:?}");
    }

    /// Type whose round-joined stroke paints half its weight past its glyphs everywhere is not tried
    /// a second time mapped from what it paints: that is the box of its glyphs grown by half the
    /// weight, already tried.
    #[test]
    fn round_joined_type_is_not_tried_twice_for_the_same_box() {
        if !default_font() {
            return;
        }
        let style = CharStyle { stroke: Paint::solid(Color::BLACK), stroke_width: 4.0, stroke_join: LineJoin::Round, ..CharStyle::default() };
        let content = vec![Arc::new(type_node(1, style))];
        vectorcraft_text::register_envelope_ink();
        assert!(with_strokes(&envelope_of(content.clone())).is_some());
        assert_eq!(painted_candidate(content), None);
    }

    /// A contour of zero length paints nothing, so it isn't part of what type's outlines paint: a
    /// filled square with a round 2 point stroke and such a contour 5 points above it paints the
    /// square grown by 1 point.
    #[test]
    fn a_contour_of_zero_length_paints_nothing() {
        let mut bp = BezPath::new();
        bp.move_to((10.0, 10.0));
        bp.line_to((20.0, 10.0));
        bp.line_to((20.0, 20.0));
        bp.line_to((10.0, 20.0));
        bp.close_path();
        bp.move_to((15.0, 5.0));
        bp.line_to((15.0, 5.0));
        bp.close_path();
        let mut stroke = StrokeLayer::new(Paint::solid(Color::BLACK), 2.0);
        (stroke.join, stroke.cap) = (LineJoin::Round, LineCap::Round);
        let mut appearance = Appearance::default();
        appearance.items.push(AppearanceItem::Fill(vectorcraft_doc::FillLayer::new(Paint::solid(Color::BLACK))));
        appearance.items.push(AppearanceItem::Stroke(stroke));
        let piece = Node::path(NodeId(1), PathData::from_bezpath(&bp), appearance);
        let outlines = Node::group(NodeId(2), vec![Arc::new(piece)]);
        assert_eq!(painted_outlines(&outlines), Some(Rect::new(9.0, 9.0, 21.0, 21.0)));
    }

    /// A side of what contents paint within a ten-thousandth of a point of their geometric box's is
    /// that side, inside it or out: the two boxes measure the same outline two ways. Further inside,
    /// there is no box; further out, it counts.
    #[test]
    fn a_side_within_the_slack_of_the_geometric_box_is_that_side() {
        let plain = Rect::new(10.0, 20.0, 110.0, 70.0);
        let painted = Rect::new(8.0, 20.0 + 1e-9, 112.0, 72.0);
        assert_eq!(painted_margin(painted, plain), Some([0.0, 0.04, 0.02, 0.02]));
        assert_eq!(painted_margin(Rect::new(8.0, 20.001, 112.0, 72.0), plain), None);
        assert_eq!(painted_margin(plain, plain), None);
        let half = [0.04_f32, 0.04, 0.02, 0.02];
        assert!(same_box(half, [0.04 + 1e-7, 0.04, 0.02, 0.02], plain));
        assert!(!same_box(half, [0.041, 0.04, 0.02, 0.02], plain));
    }
}
