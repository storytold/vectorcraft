//! Envelopes (see [`crate::ai_editing`] for the form), written in the form VectorCraft's `.ai`
//! reader opens as live envelopes again: on `%_` lines a group holding the grid that shapes the
//! envelope (while its contents are edited, the contents), then that group's `/ArtDictionary` with
//! Envelope Options as a bit word (`AIDeformOptionsKey`), Fidelity (`AIDeformFidelityKey`), Edit
//! Contents (`AIDeformEditModeKey`), a warp's style, bend, distortions and orientation
//! (`DeformStyle`, `DeformValue`, `DeformHoriz`, `DeformVert`, `Rotate`), with Distort Appearance
//! the size of the box the contents map from (`AIDeformArtWidthKey`, `AIDeformArtHeightKey`), the
//! contents in an `X=` … `X+` block (while they are edited, the grid) as `AIDeformArtKey`, and the
//! kind (`AIDeformStyle`: 1 a warp, 2 a mesh, 3 a top object); then the distorted art on plain
//! lines, and the plugin's `XP` after it. A top object's outline is the edge of the grid written
//! for it, one grid edge to each segment of its sides. An envelope the reader would open in another
//! shape is written as its distorted art.

use std::fmt::Write;
use std::sync::Arc;

use kurbo::{Point, Vec2};
use vectorcraft_doc::live::{
    Coons, EnvelopeKind, EnvelopeOptions, GradientMesh, H_DOWN, H_LEFT, H_RIGHT, H_UP, MeshPoint, PreserveShape, WARP_NET, envelope_content_boxes,
    envelope_grid, envelope_stroke_margin, warp_grid,
};
use vectorcraft_doc::{Node, NodeKind};
use vectorcraft_geom::{Affine, PathData};

use super::{MAX_NEST, Writer, Xy, hide};
use crate::import::{DEFORM_PLUGIN, EnvelopeGridPoint as GridPoint, envelope_outline, envelope_upright, warp_number};
use crate::ps;

pub(in crate::ai_editing) const ENVELOPES: &str = "envelopes of type, envelopes without contents, envelopes in opacity masks, pattern tiles, art cut to a clipping object or other envelopes' contents, envelopes on the layers of a large canvas or in a turned or mirrored frame, warps drawn without their style's net, meshes and top objects that the data would open in another shape, envelopes with a Fidelity that isn't a whole number, envelopes that map from a box other than their contents' bounds with or without strokes or keep bounds margins with Distort Appearance off, envelopes after the first 256, and envelopes nested more than 8 deep are written as their art in the editing data for other apps";

/// The most rows or columns of an envelope's grid written (Make with Mesh's own limit, and the
/// reader's).
const MAX_GRID: usize = 50;
/// How far, as a share of the diagonal of a top object's patch, the patch on the outline written for
/// it may lie from its own at any of the places checked.
const TOP_OBJECT_TOLERANCE: f64 = 1e-6;
/// How far apart two bounds margins may be and still count as the same.
const MARGIN_SLACK: f32 = 1e-5;

/// What an envelope's dictionary holds besides its contents: the entries before the contents' block,
/// each on a line of its own, the kind (`AIDeformStyle`) and the grid that shapes it (`rows`, `cols`
/// and its points with their handles, row by row from the top left, in the document), when it has
/// one.
struct Record {
    keys: String,
    style: u8,
    grid: Option<(usize, usize, Vec<GridPoint>)>,
}

/// The `XP` that ends an envelope, after its distorted art.
pub(super) fn plugin_line() -> String {
    format!("{} 1 0 0 XP", ps::bytes(DEFORM_PLUGIN.as_bytes()))
}

/// Does `n`, or anything in it, hold type?
fn holds_type(n: &Node) -> bool {
    matches!(n.kind, NodeKind::Text(_)) || n.children().is_some_and(|c| c.iter().any(|k| holds_type(k)))
}

/// The record of envelope `n` (its parts given), or `None` where the reader would open it in
/// another shape than it has, or not as an envelope.
fn record(
    n: &Node,
    content: &[Arc<Node>],
    kind: &EnvelopeKind,
    fidelity: f64,
    editing: bool,
    options: &EnvelopeOptions,
    frame: Affine,
) -> Option<Record> {
    // The reader reads Fidelity as a whole number from 0 to 100, and bounds margins only with
    // Distort Appearance on.
    let whole = (0.0..=100.0).contains(&fidelity) && fidelity.fract() == 0.0;
    let margin_kept = !options.distort_appearance && options.bounds_margin != [0.0; 4];
    if content.is_empty() || frame != Affine::IDENTITY || !whole || margin_kept || content.iter().any(|c| holds_type(c)) {
        return None;
    }
    let mut keys = String::new();
    let bit = |on: bool, at: u32| i64::from(on) << at;
    let word = bit(options.distort_appearance, 0)
        | bit(options.distort_pattern_fills, 1)
        | bit(options.distort_linear_gradients, 2)
        | bit(!options.anti_alias, 16)
        | bit(options.preserve_shape == PreserveShape::ClippingMask, 17);
    let _ = writeln!(keys, "{word} /Int (AIDeformOptionsKey) ,");
    let _ = writeln!(keys, "{} /Int (AIDeformFidelityKey) ,", fidelity as i64);
    let _ = writeln!(keys, "{} /Bool (AIDeformEditModeKey) ,", u8::from(editing));
    let real = |keys: &mut String, v: f64, key: &str| {
        ps::push_num_places(keys, v, 9);
        let _ = writeln!(keys, " /Real ({key}) ,");
    };
    let (style, grid) = match kind {
        EnvelopeKind::Warp { style, bend, h, v, horizontal, version } => {
            let fraction = |p: f64| (p.is_finite() && (-100.0..=100.0).contains(&p)).then_some(p / 100.0);
            let (number, bend, h, v) = (warp_number(style)?, fraction(*bend)?, fraction(*h)?, fraction(*v)?);
            if *version != WARP_NET {
                return None;
            }
            let _ = writeln!(keys, "{number} /Int (DeformStyle) ,");
            real(&mut keys, h, "DeformHoriz");
            real(&mut keys, bend, "DeformValue");
            real(&mut keys, v, "DeformVert");
            let _ = writeln!(keys, "{} /Bool (Rotate) ,", u8::from(!*horizontal));
            // The grid of the warp's shape, for apps that read it.
            (1, warp_grid(n).and_then(|m| as_read(&m)).and_then(|(rows, cols, pts)| sized(rows, cols).map(|(r, c)| (r, c, pts))))
        }
        EnvelopeKind::Mesh { rows, cols, points, handles } => {
            let (r, c) = sized(*rows, *cols)?;
            let mesh = envelope_grid(*rows, *cols, points, handles)?;
            let pts = as_read(&mesh)?.2;
            // The reader lays the grid out with its first row at the top and its first column at the
            // left: a grid it would lay out another way round would map the contents another way.
            if !reads_as_written(r, c, &pts) {
                return None;
            }
            (2, Some((r, c, pts)))
        }
        EnvelopeKind::TopObject { path, corners } => (3, Some(top_object_grid(path, *corners)?)),
    };
    if options.distort_appearance {
        let (stroked, plain) = envelope_content_boxes(content, Affine::IDENTITY)?;
        let margin = options.bounds_margin;
        let same = |m: [f32; 4]| m.iter().zip(margin).all(|(a, b)| (a - b).abs() <= MARGIN_SLACK);
        // The reader maps from the size written when it matches the contents' bounds with their
        // strokes, and from their geometric bounds otherwise.
        let size = if margin == [0.0; 4] {
            plain
        } else if stroked != plain && envelope_stroke_margin(stroked, plain).is_some_and(same) {
            stroked
        } else {
            return None;
        };
        real(&mut keys, size.height(), "AIDeformArtHeightKey");
        real(&mut keys, size.width(), "AIDeformArtWidthKey");
    }
    Some(Record { keys, style, grid })
}

/// `rows` and `cols` as sizes, when the reader reads a grid of them.
fn sized(rows: u32, cols: u32) -> Option<(usize, usize)> {
    let (r, c) = (usize::try_from(rows).ok()?, usize::try_from(cols).ok()?);
    ((1..=MAX_GRID).contains(&r) && (1..=MAX_GRID).contains(&c)).then_some((r, c))
}

/// The points of grid `m` with their handles as the reader reads them back: the handles that point
/// out of the grid, which nothing draws with, are left at the point. `None` for a grid of points
/// or handles that aren't finite.
fn as_read(m: &GradientMesh) -> Option<(u32, u32, Vec<GridPoint>)> {
    let (rows, cols) = (m.rows as usize, m.cols as usize);
    let finite = |v: Vec2| v.x.is_finite() && v.y.is_finite();
    let mut pts = Vec::with_capacity(m.points.len());
    for (i, q) in m.points.iter().enumerate() {
        if !finite(q.p.to_vec2()) || !q.handles.iter().all(|h| finite(*h)) {
            return None;
        }
        let (r, c) = (i / (cols + 1), i % (cols + 1));
        let mut h = q.handles;
        for (slot, out) in [(H_LEFT, c == 0), (H_RIGHT, c == cols), (H_UP, r == 0), (H_DOWN, r == rows)] {
            if out && let Some(h) = h.get_mut(slot) {
                *h = Vec2::ZERO;
            }
        }
        pts.push((q.p, h));
    }
    Some((m.rows, m.cols, pts))
}

/// Does the reader lay out grid `pts` (`rows`×`cols` patches) as it is written?
fn reads_as_written(rows: usize, cols: usize, pts: &[GridPoint]) -> bool {
    let points =
        pts.iter().map(|(p, handles)| MeshPoint { p: *p, color: vectorcraft_color::Color::BLACK, opacity: 1.0, handles: *handles }).collect();
    let mesh = GradientMesh { rows: rows as u32, cols: cols as u32, points };
    envelope_upright(&mesh).is_some_and(|(r, c, read)| (r as usize, c as usize) == (rows, cols) && read == pts)
}

/// The grid a top object with outline `path` and corners `corners` is written with: its edge runs
/// through the outline's anchors clockwise from the top left corner, one grid edge to a segment, so
/// the top and bottom sides need as many segments as each other, as do the left and right sides;
/// the points inside blend the sides. `None` where that grid's edge, read back as the top object's
/// outline, doesn't give the same patch ([`Coons::from_top_object`]) at every place checked.
fn top_object_grid(path: &PathData, corners: Option<[u32; 4]>) -> Option<(usize, usize, Vec<GridPoint>)> {
    let coons = Coons::from_top_object(path, corners)?;
    // The outline, as the patch takes it: the largest subpath.
    let mut sp = path.subpaths.iter().filter(|s| s.anchors.len() >= 2).max_by(|a, b| a.area().abs().total_cmp(&b.area().abs()))?.clone();
    let count = sp.anchors.len();
    let empty = |i: usize| {
        let c = sp.segment(i);
        [c.p1, c.p2, c.p3].iter().all(|p| p.distance(c.p0) <= 1e-9)
    };
    if !sp.closed || count < 4 || (0..count).any(empty) {
        return None;
    }
    let fits = |c: &[usize; 4]| {
        c.iter().all(|i| *i < count) && {
            let gaps = (0..4).map(|k| (c[(k + 1) % 4] + count - c[k]) % count);
            gaps.clone().all(|g| g > 0) && gaps.sum::<usize>() == count
        }
    };
    // Stored corners that fit keep the outline as drawn; otherwise it runs clockwise, and the
    // corners are the anchors the patch's corners lie on.
    let at = match corners.map(|c| c.map(|i| i as usize)).filter(fits) {
        Some(c) => c,
        None => {
            if sp.signed_area() < 0.0 {
                sp.reverse();
            }
            let find = |q: Point| sp.anchors.iter().position(|a| a.p.distance(q) <= 1e-9);
            let [tl, tr, br, bl] = coons.corners();
            let c = [find(tl)?, find(tr)?, find(br)?, find(bl)?];
            if !fits(&c) {
                return None;
            }
            c
        }
    };
    let gap = |k: usize| (at[(k + 1) % 4] + count - at[k]) % count;
    let (cols, rows) = (gap(0), gap(1));
    if gap(2) != cols || gap(3) != rows || cols > MAX_GRID || rows > MAX_GRID {
        return None;
    }
    let width = cols + 1;
    let mut pts: Vec<GridPoint> = vec![(Point::ZERO, [Vec2::ZERO; 4]); (rows + 1) * width];
    // Where the `k`th anchor from the top left corner goes, and the slots of the handles of the
    // segment from it to the next anchor: its own, and the next anchor's back.
    let place = |k: usize| -> (usize, usize, usize, usize) {
        if k < cols {
            (0, k, H_RIGHT, H_LEFT)
        } else if k < cols + rows {
            (k - cols, cols, H_DOWN, H_UP)
        } else if k < 2 * cols + rows {
            (rows, cols - (k - cols - rows), H_LEFT, H_RIGHT)
        } else {
            (rows - (k - 2 * cols - rows), 0, H_UP, H_DOWN)
        }
    };
    for k in 0..count {
        let i = (at[0] + k) % count;
        let seg = sp.segment(i);
        // A straight segment's patch spreads evenly along it, as a cubic's with its handles a third
        // of the way does.
        let (p1, p2) = if sp.segment_is_line(i) { (seg.p0.lerp(seg.p3, 1.0 / 3.0), seg.p0.lerp(seg.p3, 2.0 / 3.0)) } else { (seg.p1, seg.p2) };
        let (r, c, out, into) = place(k);
        let (nr, nc, _, _) = place((k + 1) % count);
        if let Some(q) = pts.get_mut(r * width + c) {
            q.0 = seg.p0;
            q.1[out] = p1 - seg.p0;
        }
        if let Some(q) = pts.get_mut(nr * width + nc) {
            q.1[into] = p2 - seg.p3;
        }
    }
    // The points inside, blended from the sides.
    let edge = |r: usize, c: usize| pts.get(r * width + c).map_or(Point::ZERO, |q| q.0).to_vec2();
    let mut inside = vec![];
    for r in 1..rows {
        for c in 1..cols {
            let (u, v) = (c as f64 / cols as f64, r as f64 / rows as f64);
            let p = (1.0 - v) * edge(0, c) + v * edge(rows, c) + (1.0 - u) * edge(r, 0) + u * edge(r, cols)
                - ((1.0 - u) * (1.0 - v) * edge(0, 0) + u * (1.0 - v) * edge(0, cols) + (1.0 - u) * v * edge(rows, 0) + u * v * edge(rows, cols));
            inside.push((r * width + c, p.to_point()));
        }
    }
    for (i, p) in inside {
        if let Some(q) = pts.get_mut(i) {
            q.0 = p;
        }
    }
    if !pts.iter().all(|(p, h)| p.is_finite() && h.iter().all(|h| h.is_finite())) || !reads_as_written(rows, cols, &pts) {
        return None;
    }
    // The top object the reader makes of the grid: its edge, with the corners of the grid.
    let outline = envelope_outline(rows as u32, cols as u32, &pts)?;
    let corners = [0, cols, cols + rows, 2 * cols + rows].map(|c| c as u32);
    let read = Coons::from_top_object(&outline, Some(corners))?;
    let [tl, tr, br, bl] = coons.corners();
    let reach = tl.distance(br).max(tr.distance(bl)).max(1.0) * TOP_OBJECT_TOLERANCE;
    const STEPS: usize = 8;
    let at = |i: usize| i as f64 / STEPS as f64;
    let same = (0..=STEPS).all(|j| (0..=STEPS).all(|i| coons.eval(at(i), at(j)).distance(read.eval(at(i), at(j))) <= reach));
    same.then_some((rows, cols, pts))
}

impl Writer<'_> {
    /// Envelope `n`, of transparency `xy`, as a live envelope (see the module's form), outside opacity
    /// masks' art, pattern tiles, art cut to a clipping object and the layers of a large canvas. False,
    /// writing nothing, where it is written as its art instead: where the reader would open it in
    /// another shape, or some of it is left out for the object limit ([`Self::take_objects`]).
    pub(super) fn envelope(&mut self, n: &Node, xy: Xy) -> bool {
        let NodeKind::Envelope { content, kind, fidelity, editing, options, frame } = &n.kind else { return false };
        if self.hidden > 0 || self.clip.is_some() || self.nest >= MAX_NEST || !self.symbols_live || self.envelopes_left == 0 {
            return false;
        }
        let Some(record) = record(n, content, kind, *fidelity, *editing, options, **frame) else { return false };
        let start = self.body.len();
        let (state, gs, charges, bound) = (self.state, self.gs, self.charges(), self.bound.len());
        self.nest += 1;
        self.hidden += 1;
        // The group before the dictionary: the grid, or the contents while they are edited.
        let outer = std::mem::take(&mut self.body);
        let opened = self.open_group("u");
        if opened {
            if *editing {
                self.contents(content);
            } else {
                self.grid(record.grid.as_ref());
            }
            self.line("U");
        }
        let before = std::mem::replace(&mut self.body, outer);
        hide(&before, &mut self.body);
        self.body.push_str("%_/ArtDictionary :\n");
        hide(&record.keys, &mut self.body);
        self.body.push_str("%_X=\n");
        // The dictionary's art starts unlocked, shown and opaque, as an opacity mask's does.
        let outer = std::mem::take(&mut self.body);
        let at_block = self.state;
        (self.state, self.gs) = (Some((false, false)), Some(Xy::OPAQUE));
        if *editing {
            self.grid(record.grid.as_ref());
        } else if self.open_group("u") {
            self.contents(content);
            self.end_group("U", Xy::PLAIN);
        }
        let block = std::mem::replace(&mut self.body, outer);
        hide(&block, &mut self.body);
        (self.state, self.gs, self.block_end) = (at_block, None, usize::MAX);
        let _ = writeln!(self.body, "%_X+\n%_ (AIDeformArtKey) ,\n%_{} /Int (AIDeformStyle) ,\n%_;\n%_\n%_9 () XW", record.style);
        self.hidden -= 1;
        self.nest -= 1;
        // The distorted art, which takes the envelope's lock, visibility, transparency, name and
        // opacity mask.
        self.state(n.locked, !n.visible);
        self.drawn_art(n, xy, None);
        if !opened || self.left_out_since(charges) {
            let lines = self.body.split_off(start);
            self.give_back(charges, &lines);
            self.bound.truncate(bound);
            (self.state, self.gs, self.block_end) = (state, gs, usize::MAX);
            return false;
        }
        self.deform = true;
        self.envelopes_left -= 1;
        true
    }

    /// An envelope's contents.
    fn contents(&mut self, content: &[Arc<Node>]) {
        for c in content {
            self.node(c);
        }
    }

    /// An envelope's grid, when it has one, as a gray gradient mesh (`/Mesh X!` … `/End X!`): its
    /// size (`[cols rows] /Size`), then each patch's place (`[col row] /P`), its four corners
    /// (`/N`: gray, opacity, the point, the handle towards the next corner and a weight, the handle
    /// towards the previous one and a weight, in art space), from its top left one down, across and
    /// up, and `/E`. One object.
    fn grid(&mut self, grid: Option<&(usize, usize, Vec<GridPoint>)>) {
        let Some((rows, cols, pts)) = grid else { return };
        if !self.take_objects(1) {
            return;
        }
        let (rows, cols, width) = (*rows, *cols, cols + 1);
        let to_art = self.to_art;
        let s = &mut self.body;
        let _ = writeln!(s, "/Mesh X!\n2 /Version X#\n/Cartesian /Type X#\n[{cols} {rows}] /Size X#\n/Data X#\n/DeviceGray /CS X#");
        let num = |s: &mut String, v: f64| {
            s.push(' ');
            ps::push_num(s, v);
        };
        for c in 0..cols {
            for r in 0..rows {
                let _ = writeln!(s, "[{c} {r}] /P X#");
                // Each corner with the slots of its handles towards the next corner and the previous one.
                let corners = [((r, c), H_DOWN, H_RIGHT), ((r + 1, c), H_RIGHT, H_UP), ((r + 1, c + 1), H_UP, H_LEFT), ((r, c + 1), H_LEFT, H_DOWN)];
                for ((pr, pc), next, prev) in corners {
                    let (p, h) = pts.get(pr * width + pc).copied().unwrap_or((Point::ZERO, [Vec2::ZERO; 4]));
                    let at = |v: Vec2| to_art * (p + v);
                    s.push_str("[0 1");
                    for q in [at(Vec2::ZERO), at(h[next])] {
                        num(s, q.x);
                        num(s, q.y);
                    }
                    s.push_str(" 1");
                    let q = at(h[prev]);
                    num(s, q.x);
                    num(s, q.y);
                    s.push_str(" 1] /N X#\n");
                }
                s.push_str("/E X#\n");
            }
        }
        s.push_str("/End X!\n");
    }
}
