//! What draws differently after an edit, or once placed documents' bitmaps are ready, so a view
//! can draw again only there.

use std::collections::HashSet;
use std::sync::Arc;

use vectorcraft_doc::{Document, Node, NodeKind, SymbolBoxes};
use vectorcraft_geom::{Affine, Rect};

/// What may draw differently in a document than in its version before an edit ([`changed`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Changed {
    Nothing,
    /// This area of the document, outside of which every pixel draws as before.
    Area(Rect),
    Everything,
}

impl Changed {
    /// What may draw differently after this change and `other`: everything when either is,
    /// else both areas.
    pub fn and(self, other: Changed) -> Changed {
        match (self, other) {
            (Changed::Everything, _) | (_, Changed::Everything) => Changed::Everything,
            (Changed::Area(a), Changed::Area(b)) => Changed::Area(a.union(b)),
            (Changed::Area(a), Changed::Nothing) | (Changed::Nothing, Changed::Area(a)) => Changed::Area(a),
            (Changed::Nothing, Changed::Nothing) => Changed::Nothing,
        }
    }
}

/// What draws differently in `doc` once the bitmaps of the placed documents' `files` are ready
/// ([`crate::placed_document::made_between`]): the bounds of everything the objects that show
/// them paint, measured as [`changed`] measures an object that changed. Everything when the art
/// of a symbol, a graph design, a pattern or a brush shows one of them (it draws wherever it is
/// used), or while a pattern is edited.
pub fn placed_area(doc: &Document, files: &HashSet<String>) -> Changed {
    if files.is_empty() {
        return Changed::Nothing;
    }
    let mut elsewhere =
        doc.symbols.iter().map(|s| &s.art).chain(doc.graph_designs.iter().map(|g| &g.art)).chain(doc.patterns.iter().flat_map(|p| &p.art));
    if doc.pattern_edit.is_some()
        || elsewhere.any(|n| shows(n, files))
        || vectorcraft_brush::library(doc).iter().any(|b| b.art().into_iter().any(|n| shows(n, files)))
    {
        return Changed::Everything;
    }
    let symbols = doc.symbol_boxes();
    let mut area = None;
    for layer in &doc.layers {
        add_placed(&symbols, layer, files, &mut area);
    }
    area.map_or(Changed::Nothing, Changed::Area)
}

/// Does `n`, a member of it or the art of an opacity mask in it show one of `files`?
fn shows(n: &Node, files: &HashSet<String>) -> bool {
    matches!(&n.kind, NodeKind::PlacedDocument(p) if files.contains(&p.key))
        || n.extras().mask.as_ref().is_some_and(|m| shows(&m.art, files))
        || n.children().is_some_and(|c| c.iter().any(|c| shows(c, files)))
}

/// Add to `area` what `n` paints where it shows one of `files`: each member of a group or layer
/// whose members draw each on their own (unless its opacity mask shows one), and any other object
/// whole.
fn add_placed(symbols: &SymbolBoxes, n: &Node, files: &HashSet<String>, area: &mut Option<Rect>) {
    match members(n) {
        Some(children) if !n.extras().mask.as_ref().is_some_and(|m| shows(&m.art, files)) => {
            for c in children {
                add_placed(symbols, c, files, area);
            }
        }
        _ if shows(n, files) => *area = vectorcraft_geom::union_opt(*area, crate::brush_fx::cull_bounds(symbols, n)),
        _ => {}
    }
}

/// What may draw differently in `new` than in `old`, the version before an edit: the bounds of
/// everything the objects that changed paint, before and after the edit ([`crate::painted_bounds`]),
/// or everything when more than the art in the layers changed, or while a pattern is edited (its
/// art also draws in the copies round it). Objects that an edit doesn't touch
/// keep their identity, so this costs what changed, not the size of the document.
pub fn changed(old: &Document, new: &Document) -> Changed {
    if new.pattern_edit.is_some() || !old.same_outside_layers(new) {
        return Changed::Everything;
    }
    let mut diff = Diff { old: old.symbol_boxes(), new: new.symbol_boxes(), area: None };
    diff.children(&old.layers, &new.layers);
    diff.area.map_or(Changed::Nothing, Changed::Area)
}

/// The pixels of a `w`×`h` render through `view` that drawing `area` again covers, as x0, y0, x1,
/// y1: its box in the view, grown by a pixel for the anti-aliased edges. None when it covers none.
pub fn view_pixels(area: Rect, view: Affine, w: u32, h: u32) -> Option<[u32; 4]> {
    let r = view.transform_rect_bbox(area).inflate(1.0, 1.0);
    if ![r.x0, r.y0, r.x1, r.y1].iter().all(|c| c.is_finite()) {
        return None;
    }
    let (x0, y0) = (r.x0.floor().max(0.0), r.y0.floor().max(0.0));
    let (x1, y1) = (r.x1.ceil().min(f64::from(w)), r.y1.ceil().min(f64::from(h)));
    (x1 > x0 && y1 > y0).then_some([x0 as u32, y0 as u32, x1 as u32, y1 as u32])
}

/// [`changed`] in progress: the symbols' boxes of both versions, and the area so far.
struct Diff<'a> {
    old: SymbolBoxes<'a>,
    new: SymbolBoxes<'a>,
    area: Option<Rect>,
}

impl Diff<'_> {
    /// Members in the same order compare one by one. Members added, removed or restacked count
    /// before and after, except those that kept their place unchanged.
    fn children(&mut self, old: &[Arc<Node>], new: &[Arc<Node>]) {
        if old.len() == new.len() && old.iter().zip(new).all(|(a, b)| a.id == b.id) {
            for (a, b) in old.iter().zip(new) {
                self.node(a, b);
            }
            return;
        }
        for i in 0..old.len().max(new.len()) {
            match (old.get(i), new.get(i)) {
                (Some(a), Some(b)) if Arc::ptr_eq(a, b) => {}
                (a, b) => {
                    self.add_old(a.map(|a| &**a));
                    self.add_new(b.map(|b| &**b));
                }
            }
        }
    }

    /// A group or layer that only its members changed in, without a clipping mask or effects of
    /// its own that draw from all of them, changed where they did. A path or compound path that
    /// only had anchors moved changed around them ([`moved_anchors`]). Any other object that
    /// changed counts whole, before and after.
    fn node(&mut self, old: &Arc<Node>, new: &Arc<Node>) {
        if Arc::ptr_eq(old, new) {
            return;
        }
        if let (Some(a), Some(b)) = (members(old), members(new))
            && same_but_members(old, new)
        {
            self.children(a, b);
        } else if let Some((area, moved, anchors)) = moved_anchors(old, new) {
            if moved <= FEW_MOVED && anchors >= MANY_ANCHORS {
                self.add(area);
            } else {
                // The object's box before and after holds every pixel that changed too, and so
                // does where the two overlap.
                let whole = vectorcraft_geom::union_opt(crate::brush_fx::cull_bounds(&self.old, old), crate::brush_fx::cull_bounds(&self.new, new));
                self.add(area.zip(whole).map(|(a, w)| a.intersect(w)));
            }
        } else {
            self.add_old(Some(old));
            self.add_new(Some(new));
        }
    }

    fn add_old(&mut self, n: Option<&Node>) {
        self.add(n.and_then(|n| crate::brush_fx::cull_bounds(&self.old, n)));
    }

    fn add_new(&mut self, n: Option<&Node>) {
        self.add(n.and_then(|n| crate::brush_fx::cull_bounds(&self.new, n)));
    }

    fn add(&mut self, r: Option<Rect>) {
        self.area = vectorcraft_geom::union_opt(self.area, r);
    }
}

/// The members of a group or layer whose members draw each on their own: no clipping mask, and no
/// effects of its own.
fn members(n: &Node) -> Option<&[Arc<Node>]> {
    match &n.kind {
        NodeKind::Layer { children, clip: false, .. } | NodeKind::Group { children, clip: false } if !crate::fx::has_object_fx(n) => Some(children),
        _ => None,
    }
}

/// Are `a` and `b` the same but for their members?
fn same_but_members(a: &Node, b: &Node) -> bool {
    let strip = |n: &Node| {
        let mut n = n.clone();
        if let Some(children) = n.children_mut() {
            children.clear();
        }
        n
    };
    strip(a) == strip(b)
}

/// The largest size of a coordinate, in points, that [`moved_anchors`] measures: past it, a path
/// drawn at a high zoom may stop being drawn at all (`crate::node_checks::drawable`).
const MOVED_LIMIT: f64 = 1e6;

/// The most anchors of an object that may move for [`Diff::node`] to take the area round them
/// without also measuring the whole object before and after, which costs as much as its anchors.
const FEW_MOVED: usize = 64;

/// The fewest anchors of an object that [`Diff::node`] leaves unmeasured whole when at most
/// [`FEW_MOVED`] of them moved.
const MANY_ANCHORS: usize = 1024;

/// What path or compound path `old` changed in `new` when only anchors of its paths moved (in
/// place, with as many anchors in each subpath, each closed or open as before): the box of the
/// anchors and handles of every segment that starts or ends at a moved anchor, and of the line
/// that closes the fill of an open subpath at a moved end, before and after, grown by the farthest
/// its strokes reach (`None` when no anchor moved), how many anchors moved and how many it has.
/// Outside that box the path's segments, their fill and their strokes' joints stay as they were.
/// None when anything else changed, when it paints with something its whole shape decides
/// (effects, an opacity mask, a paint other than a solid color, dashes, a width profile,
/// arrowheads or a brush), or when a coordinate measured is not finite or larger in size than
/// [`MOVED_LIMIT`].
fn moved_anchors(old: &Node, new: &Node) -> Option<(Option<Rect>, usize, usize)> {
    if !same_but_anchors(old, new) || !local_paint(old) {
        return None;
    }
    let (area, moved, anchors) = match (&old.kind, &new.kind) {
        (NodeKind::Path { path: a, .. }, NodeKind::Path { path: b, .. }) => moved_in(a, b)?,
        (NodeKind::Compound { children: a, .. }, NodeKind::Compound { children: b, .. }) => {
            let (mut area, mut moved, mut anchors) = (None, 0, 0);
            for (x, y) in a.iter().zip(b) {
                if Arc::ptr_eq(x, y) {
                    anchors += y.path_data().map_or(0, |p| p.anchor_count());
                    continue;
                }
                match (&x.kind, &y.kind) {
                    (NodeKind::Path { path: p, .. }, NodeKind::Path { path: q, .. }) if same_but_anchors(x, y) => {
                        let (a, m, n) = moved_in(p, q)?;
                        (area, moved, anchors) = (vectorcraft_geom::union_opt(area, a), moved + m, anchors + n);
                    }
                    _ => return None,
                }
            }
            (area, moved, anchors)
        }
        _ => return None,
    };
    let reach = old.appearance.outset();
    Some((area.map(|r| r.inflate(reach, reach)), moved, anchors))
}

/// Are `a` and `b` the same but for where the anchors of their paths are: two paths, or two
/// compound paths with the same members by id, without extras, live shapes, guides or clipping?
fn same_but_anchors(a: &Node, b: &Node) -> bool {
    let Node { id, name, visible, locked, opacity, blend, isolate, knockout, knockout_shape, appearance, kind, bbox_angle, extras } = a;
    let kinds = match (kind, &b.kind) {
        (
            NodeKind::Path { path: p, rule: r, live: None, clipping: false, guide: false },
            NodeKind::Path { path: q, rule: s, live: None, clipping: false, guide: false },
        ) => {
            r == s
                && p.subpaths.len() == q.subpaths.len()
                && p.subpaths.iter().zip(&q.subpaths).all(|(x, y)| x.anchors.len() == y.anchors.len() && x.closed == y.closed)
        }
        (NodeKind::Compound { children: c, rule: r }, NodeKind::Compound { children: d, rule: s }) => {
            r == s && c.len() == d.len() && c.iter().zip(d).all(|(x, y)| x.id == y.id)
        }
        _ => false,
    };
    let none = vectorcraft_doc::Extras::default();
    kinds
        && *id == b.id
        && *name == b.name
        && *visible == b.visible
        && *locked == b.locked
        && *opacity == b.opacity
        && *blend == b.blend
        && *isolate == b.isolate
        && *knockout == b.knockout
        && *knockout_shape == b.knockout_shape
        && *appearance == b.appearance
        && *bbox_angle == b.bbox_angle
        && *extras == none
        && b.extras == none
}

/// Does `n` paint each pixel from the segments near it alone: no effects, every visible fill and
/// stroke a solid color, and no stroke with dashes, a width profile, pressure, tilt, bearing or
/// rotation, arrowheads, a brush or effects?
fn local_paint(n: &Node) -> bool {
    use vectorcraft_doc::AppearanceItem;
    let solid = |p: &vectorcraft_color::Paint| matches!(p, vectorcraft_color::Paint::Solid { .. } | vectorcraft_color::Paint::None);
    n.appearance.effects.is_empty()
        && !crate::fx::has_fx(n)
        && !crate::fx::has_object_fx(n)
        && n.appearance.items.iter().all(|i| match i {
            AppearanceItem::Fill(f) => solid(&f.paint) && f.effects.is_empty(),
            AppearanceItem::Stroke(s) => {
                solid(&s.paint)
                    && s.effects.is_empty()
                    && s.dash.is_none()
                    && s.profile.is_none()
                    && s.pressure.is_none()
                    && s.tilt.is_none()
                    && s.bearing.is_none()
                    && s.rotation.is_none()
                    && s.start_arrow.is_none()
                    && s.end_arrow.is_none()
                    && s.brush.is_none()
            }
        })
}

/// The box of the anchors and handles of the segments that start or end at an anchor that moved
/// from `a` to `b` (paths with the same subpaths, by anchor count and closing), and the ends of
/// the line that closes the fill of an open subpath at a moved end, in both (`None` when none
/// moved), how many moved and how many anchors `b` has. None when one of those coordinates is not
/// finite or larger in size than [`MOVED_LIMIT`].
fn moved_in(a: &vectorcraft_geom::PathData, b: &vectorcraft_geom::PathData) -> Option<(Option<Rect>, usize, usize)> {
    let (mut area, mut moved, mut anchors): (Option<Rect>, usize, usize) = (None, 0, 0);
    for (x, y) in a.subpaths.iter().zip(&b.subpaths) {
        let n = x.anchors.len();
        anchors += n;
        for i in (0..n).filter(|&i| x.anchors[i] != y.anchors[i]) {
            moved += 1;
            // The anchors before and after it, round the end of an open subpath too: its fill
            // closes it with a line from the last anchor to the first.
            let (before, after) = ((i + n - 1) % n, (i + 1) % n);
            for sp in [x, y] {
                let (at, from, to) = (&sp.anchors[i], &sp.anchors[before], &sp.anchors[after]);
                for p in [at.p, at.h_in, at.h_out, from.p, from.h_out, to.p, to.h_in] {
                    if !(p.x.abs() <= MOVED_LIMIT && p.y.abs() <= MOVED_LIMIT) {
                        return None;
                    }
                    area = Some(area.map_or(Rect::from_points(p, p), |r| r.union_pt(p)));
                }
            }
        }
    }
    Some((area, moved, anchors))
}
