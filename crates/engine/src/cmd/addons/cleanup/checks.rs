//! Document Health Check: the kinds of issue Artwork Cleanup looks for, and how each is fixed. A
//! check runs a kind's fix on a copy of the document, so what a check finds is what Fix changes.

use std::collections::{HashMap, HashSet};

use vectorcraft_color::BlendMode;
use vectorcraft_doc::appearance::AppearanceItem;
use vectorcraft_doc::{Appearance, Document, Knockout, Node, NodeExtras, NodeId, NodeKind, Selection, SymbolBoxes};
use vectorcraft_geom::{PathData, Point, Rect};
use vectorcraft_pathops::addons as po;

use super::super::super::edit::outermost;
use super::super::super::layer::empty_artboards;
use crate::Result;

/// Sine of the turn below which an anchor on a straight run is collinear.
const COLLINEAR: f64 = 0.005;
/// Distance (pt) within which anchors coincide, handles are tiny and path ends meet.
const NEAR: f64 = 0.01;
/// Most objects a check looks at; the rest of a larger document is left as it is.
pub(super) const MAX_OBJECTS: usize = 250_000;
/// Most anchors Near Anchors compares.
const MAX_ANCHORS: usize = 1_000_000;
/// Most rounds of unwrapping nested groups.
const MAX_PASSES: usize = 64;

/// One kind of issue.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    EmptyText,
    StrayPoints,
    Unpainted,
    ZeroOpacity,
    DuplicateObjects,
    OffArtboard,
    DuplicateAnchors,
    TinyHandles,
    CollinearAnchors,
    NearPoints,
    NearlyClosed,
    FilledOpenPaths,
    NeedlessClipGroups,
    NeedlessCompounds,
    NeedlessGroups,
    EmptyArtboards,
}

impl Kind {
    /// Every kind, in the order Fix runs them (removing art first can leave groups to unwrap).
    pub(super) const ALL: [Kind; 16] = [
        Kind::EmptyText,
        Kind::StrayPoints,
        Kind::Unpainted,
        Kind::ZeroOpacity,
        Kind::DuplicateObjects,
        Kind::OffArtboard,
        Kind::DuplicateAnchors,
        Kind::TinyHandles,
        Kind::CollinearAnchors,
        Kind::NearPoints,
        Kind::NearlyClosed,
        Kind::FilledOpenPaths,
        Kind::NeedlessClipGroups,
        Kind::NeedlessCompounds,
        Kind::NeedlessGroups,
        Kind::EmptyArtboards,
    ];

    /// (id, label) of the kind.
    fn names(self) -> (&'static str, &'static str) {
        match self {
            Kind::EmptyText => ("emptyText", "Empty Text"),
            Kind::StrayPoints => ("strayPoints", "Stray Points"),
            Kind::Unpainted => ("unpainted", "Unpainted Objects"),
            Kind::ZeroOpacity => ("zeroOpacity", "Zero-Opacity Objects"),
            Kind::DuplicateObjects => ("duplicateObjects", "Duplicate Objects"),
            Kind::OffArtboard => ("offArtboard", "Off-Artboard Art"),
            Kind::DuplicateAnchors => ("duplicateAnchors", "Duplicate Anchors"),
            Kind::TinyHandles => ("tinyHandles", "Tiny Handles"),
            Kind::CollinearAnchors => ("collinearAnchors", "Collinear Anchors"),
            Kind::NearPoints => ("nearPoints", "Nearly Coincident Anchors"),
            Kind::NearlyClosed => ("nearlyClosed", "Nearly Closed Paths"),
            Kind::FilledOpenPaths => ("filledOpenPaths", "Filled Open Paths"),
            Kind::NeedlessClipGroups => ("needlessClipGroups", "Needless Clipping Groups"),
            Kind::NeedlessCompounds => ("needlessCompounds", "Needless Compound Paths"),
            Kind::NeedlessGroups => ("needlessGroups", "Needless Groups"),
            Kind::EmptyArtboards => ("emptyArtboards", "Empty Artboards"),
        }
    }

    pub(super) fn id(self) -> &'static str {
        self.names().0
    }

    pub(super) fn label(self) -> &'static str {
        self.names().1
    }

    pub(super) fn parse(id: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|k| k.id() == id)
    }

    /// Fix this kind of issue in `scope` of `d` → what changed.
    pub(super) fn fix(self, d: &mut Document, sel: &mut Selection, scope: &mut Scope) -> Result<Outcome> {
        match self {
            Kind::DuplicateAnchors => reshape(d, scope, |_, p| p.subpaths.iter_mut().map(|sp| po::merge_duplicate_anchors(sp, NEAR)).sum()),
            Kind::TinyHandles => reshape(d, scope, |_, p| p.subpaths.iter_mut().map(|sp| po::retract_tiny_handles(sp, NEAR)).sum()),
            Kind::CollinearAnchors => reshape(d, scope, |_, p| p.subpaths.iter_mut().map(|sp| po::remove_collinear_anchors(sp, COLLINEAR)).sum()),
            Kind::NearlyClosed => reshape(d, scope, |_, p| p.subpaths.iter_mut().map(|sp| usize::from(po::close_nearly_closed(sp, NEAR))).sum()),
            Kind::FilledOpenPaths => reshape(d, scope, close_filled),
            Kind::NearPoints => near_points(d, scope),
            Kind::EmptyArtboards => artboards(d, sel, scope),
            _ => restructure(d, scope, self),
        }
    }

    /// What replaces object `n` (at `place`) when it is this kind of issue: nothing to remove it,
    /// its members to unwrap it. `None` when it isn't one.
    fn replacement(self, cx: &Context, n: &Node, place: Place) -> Option<Vec<Node>> {
        let removable = !place.clips && !is_guide_or_clip(n);
        let gone = |is: bool| is.then(Vec::new);
        match self {
            Kind::EmptyText => gone(matches!(&n.kind, NodeKind::Text(t) if t.plain_text().trim().is_empty())),
            Kind::StrayPoints => gone(removable && matches!(&n.kind, NodeKind::Path { path, .. } if path.anchor_count() <= 1)),
            Kind::Unpainted => gone(removable && !place.in_compound && is_shape(n) && !is_painted(&n.appearance)),
            Kind::ZeroOpacity => gone(removable && !place.in_compound && n.opacity <= 0.0),
            Kind::DuplicateObjects => gone(removable && duplicates_the_one_below(cx.d, n)),
            Kind::OffArtboard => gone(place.top && !is_guide_or_clip(n) && off_artboards(cx, n)),
            Kind::NeedlessGroups => needless_group(n),
            Kind::NeedlessCompounds => needless_compound(n),
            Kind::NeedlessClipGroups => needless_clip_group(cx, n),
            _ => None,
        }
    }
}

/// What a fix changed (or a check found): how many issues, the objects they are on and the
/// artboards (by index) they are.
#[derive(Clone, Debug, Default)]
pub(super) struct Outcome {
    pub count: usize,
    pub ids: Vec<NodeId>,
    pub artboards: Vec<usize>,
}

/// What a check looks at: the whole document, or the objects given and what is inside them.
#[derive(Clone, Debug)]
pub(super) enum Scope {
    Document,
    Objects(Vec<NodeId>),
}

/// Where an object sits.
#[derive(Clone, Copy, Default)]
struct Place {
    /// A member of a compound path.
    in_compound: bool,
    /// The clipping path of its group or layer.
    clips: bool,
    /// A top-level object of the scope (on a layer, or given).
    top: bool,
}

/// The document a pass looks at, with its symbols measured once.
struct Context<'a> {
    d: &'a Document,
    symbols: SymbolBoxes<'a>,
}

/// The visible, unlocked objects in `scope` with where they sit, in paint order: groups, layers,
/// clip groups and compound paths are looked into; live and generated art (blends, envelopes,
/// graphs, symbols…) is taken as it is.
fn objects<'a>(d: &'a Document, scope: &Scope) -> Vec<(&'a Node, Place)> {
    let mut out = vec![];
    let top = Place { top: true, ..Place::default() };
    match scope {
        Scope::Document => d.layers.iter().for_each(|l| visit(l, top, &mut out)),
        Scope::Objects(ids) => ids.iter().filter_map(|id| d.node(*id)).for_each(|n| visit(n, top, &mut out)),
    }
    let editable = d.editable();
    out.retain(|(n, _)| editable(n.id));
    out
}

fn visit<'a>(n: &'a Node, place: Place, out: &mut Vec<(&'a Node, Place)>) {
    if !n.visible || n.locked || out.len() >= MAX_OBJECTS {
        return;
    }
    let inner = |clip: bool, i: usize, top: bool| Place { clips: clip && i == 0, top, in_compound: false };
    match &n.kind {
        // A layer's objects are top-level ones.
        NodeKind::Layer { children, clip, .. } => children.iter().enumerate().for_each(|(i, c)| visit(c, inner(*clip, i, true), out)),
        NodeKind::Group { children, clip } if !is_generated(n) => {
            out.push((n, place));
            children.iter().enumerate().for_each(|(i, c)| visit(c, inner(*clip, i, false), out));
        }
        NodeKind::Compound { children, .. } => {
            out.push((n, place));
            children.iter().for_each(|c| visit(c, Place { in_compound: true, ..Place::default() }, out));
        }
        _ => out.push((n, place)),
    }
}

/// A group that stands for live or generated art: its members aren't loose art to clean.
fn is_generated(n: &Node) -> bool {
    let e = n.extras();
    e.graph.is_some() || e.shaper.is_some() || e.dimension.is_some() || e.objects_on_path.is_some() || e.live_paint_gaps.is_some() || e.compound_shape
}

fn is_guide_or_clip(n: &Node) -> bool {
    matches!(n.kind, NodeKind::Path { guide: true, .. } | NodeKind::Path { clipping: true, .. })
}

fn is_shape(n: &Node) -> bool {
    matches!(n.kind, NodeKind::Path { .. } | NodeKind::Compound { .. })
}

fn is_painted(a: &Appearance) -> bool {
    a.items.iter().any(|i| match i {
        AppearanceItem::Fill(f) => f.visible && !f.paint.is_none(),
        AppearanceItem::Stroke(s) => s.visible && !s.paint.is_none() && s.width > 0.0,
    })
}

/// A container with nothing of its own to show: full opacity, normal blending, no fill, stroke,
/// effect, mask, knockout or anything else attached.
fn is_plain(n: &Node) -> bool {
    n.opacity >= 1.0
        && n.blend == BlendMode::Normal
        && n.appearance.items.is_empty()
        && n.appearance.effects.is_empty()
        && !n.isolate
        && n.knockout == Knockout::Neutral
        && *n.extras() == NodeExtras::default()
}

/// Is `n` a plain path (full opacity, normal blending) identical to the object just below it?
fn duplicates_the_one_below(d: &Document, n: &Node) -> bool {
    if !matches!(n.kind, NodeKind::Path { .. }) || n.opacity < 1.0 || n.blend != BlendMode::Normal {
        return false;
    }
    let below = d.position(n.id).and_then(|(parent, i, _)| d.children(parent)?.get(i.checked_sub(1)?).cloned());
    below.is_some_and(|b| {
        b.visible && b.kind == n.kind && b.appearance == n.appearance && b.opacity == n.opacity && b.blend == n.blend && b.extras() == n.extras()
    })
}

fn inside(a: Rect, b: Rect) -> bool {
    a.x0 >= b.x0 && a.y0 >= b.y0 && a.x1 <= b.x1 && a.y1 <= b.y1
}

fn meets(a: Rect, b: Rect) -> bool {
    a.x0 <= b.x1 && b.x0 <= a.x1 && a.y0 <= b.y1 && b.y0 <= a.y1
}

/// Does `n` lie wholly off every artboard (a document without artboards has none off them)?
fn off_artboards(cx: &Context, n: &Node) -> bool {
    let Some(b) = n.visual_bounds_with(&cx.symbols) else { return false };
    // As Delete Empty Artboards measures: a millionth of a point covers rounding at the edges.
    !cx.d.artboards.is_empty() && !cx.d.artboards.iter().any(|a| meets(a.rect.inflate(1e-6, 1e-6), b))
}

/// A plain group of one member or none: its member, or nothing.
fn needless_group(n: &Node) -> Option<Vec<Node>> {
    let NodeKind::Group { children, clip: false } = &n.kind else { return None };
    (children.len() <= 1 && is_plain(n)).then(|| children.iter().map(|c| (**c).clone()).collect())
}

/// A compound path of one path or none: that path, painted as the compound was, or nothing.
fn needless_compound(n: &Node) -> Option<Vec<Node>> {
    let NodeKind::Compound { children, rule } = &n.kind else { return None };
    let mut path = match children.as_slice() {
        [] => return Some(vec![]),
        [one] => (**one).clone(),
        _ => return None,
    };
    let NodeKind::Path { rule: r, .. } = &mut path.kind else { return None };
    *r = *rule;
    path.appearance = n.appearance.clone();
    path.opacity = n.opacity;
    path.blend = n.blend;
    path.extras = n.extras.clone();
    Some(vec![path])
}

/// A plain clip group whose unpainted clipping path changes nothing: it clips nothing or lies wholly
/// away from its art (nothing shows: nothing is left), or it is a rectangle around all its art (its
/// art is left).
fn needless_clip_group(cx: &Context, n: &Node) -> Option<Vec<Node>> {
    let NodeKind::Group { children, clip: true } = &n.kind else { return None };
    if !is_plain(n) {
        return None;
    }
    let Some((clip, art)) = children.split_first() else { return Some(vec![]) };
    let paint = clip.clip_paint();
    if paint.fill.is_some() || paint.stroke.is_some() {
        return None;
    }
    let clip_box = clip.geometric_bounds()?;
    let art_box = art.iter().filter_map(|a| a.visual_bounds_with(&cx.symbols)).reduce(|a, b| a.union(b));
    match art_box {
        None => Some(vec![]),
        Some(b) if !meets(clip_box, b) => Some(vec![]),
        Some(b) if is_rectangle(clip) && inside(b, clip_box.inflate(1e-6, 1e-6)) => Some(art.iter().map(|a| (**a).clone()).collect()),
        _ => None,
    }
}

/// Is `n` a path drawing a square-to-the-page rectangle: four corners on its bounds, no curves?
fn is_rectangle(n: &Node) -> bool {
    let NodeKind::Path { path, .. } = &n.kind else { return false };
    let (Some(b), [sp]) = (path.bounds(), path.subpaths.as_slice()) else { return false };
    let corner = |p: Point| ((p.x - b.x0).abs() < 1e-6 || (p.x - b.x1).abs() < 1e-6) && ((p.y - b.y0).abs() < 1e-6 || (p.y - b.y1).abs() < 1e-6);
    sp.closed && sp.anchors.len() == 4 && sp.anchors.iter().all(|a| !a.has_in() && !a.has_out() && corner(a.p))
}

/// Replace the objects with this kind of issue (the outermost ones each round, until none is
/// left) by what [`Kind::replacement`] gives.
fn restructure(d: &mut Document, scope: &mut Scope, kind: Kind) -> Result<Outcome> {
    let mut out = Outcome::default();
    for _ in 0..MAX_PASSES {
        let found: Vec<(NodeId, Vec<Node>)> = {
            let d: &Document = d;
            let cx = Context { d, symbols: d.symbol_boxes() };
            let all: Vec<(NodeId, Vec<Node>)> =
                objects(d, scope).into_iter().filter_map(|(n, pl)| Some((n.id, kind.replacement(&cx, n, pl)?))).collect();
            let keep: HashSet<NodeId> = outermost(d, all.iter().map(|(id, _)| *id).collect()).into_iter().collect();
            all.into_iter().filter(|(id, _)| keep.contains(id)).collect()
        };
        if found.is_empty() {
            break;
        }
        if let Scope::Objects(roots) = scope {
            for (id, nodes) in &found {
                if let Some(at) = roots.iter().position(|r| r == id) {
                    roots.splice(at..=at, nodes.iter().map(|n| n.id));
                }
            }
        }
        out.ids.extend(found.iter().map(|(id, _)| *id));
        d.insert_beside(found, true)?;
    }
    out.count = out.ids.len();
    Ok(out)
}

/// Run `f` on a copy of each path in scope (not guides), and keep the copies it changed: `f`
/// returns how many issues it fixed.
fn reshape(d: &mut Document, scope: &Scope, f: impl Fn(&Node, &mut PathData) -> usize) -> Result<Outcome> {
    let changed: Vec<(NodeId, PathData, usize)> = objects(d, scope)
        .into_iter()
        .filter_map(|(n, _)| {
            let NodeKind::Path { path, guide: false, .. } = &n.kind else { return None };
            let mut p = path.clone();
            let k = f(n, &mut p);
            (k > 0).then_some((n.id, p, k))
        })
        .collect();
    let mut out = Outcome::default();
    for (id, p, k) in changed {
        set_path(d, id, p);
        out.count += k;
        out.ids.push(id);
    }
    Ok(out)
}

/// Give path `id` the geometry `p` (it stops being a live shape).
pub(super) fn set_path(d: &mut Document, id: NodeId, p: PathData) {
    if let Some(NodeKind::Path { path, live, .. }) = d.node_mut(id).map(|n| &mut n.kind) {
        *path = p;
        *live = None;
    }
}

/// Filled Open Paths: close the open subpaths of a path with a fill and no stroke.
fn close_filled(n: &Node, p: &mut PathData) -> usize {
    let items = &n.appearance.items;
    let filled = items.iter().any(|i| matches!(i, AppearanceItem::Fill(f) if f.visible && !f.paint.is_none()));
    let stroked = items.iter().any(|i| matches!(i, AppearanceItem::Stroke(s) if s.visible && !s.paint.is_none() && s.width > 0.0));
    if !filled || stroked {
        return 0;
    }
    let mut count = 0;
    for sp in p.subpaths.iter_mut().filter(|sp| !sp.closed && sp.anchors.len() >= 2) {
        sp.closed = true;
        count += 1;
    }
    count
}

/// Nearly Coincident Anchors: anchors of the paths in scope within [`NEAR`] of one another (but
/// not neighbours on one path, which Duplicate Anchors merges) move onto the first of them.
fn near_points(d: &mut Document, scope: &Scope) -> Result<Outcome> {
    // Each anchor: (path, subpath, anchor, its subpath's length and closedness).
    let mut refs: Vec<(NodeId, usize, usize, usize, bool)> = vec![];
    let mut points: Vec<Point> = vec![];
    for (n, _) in objects(d, scope) {
        let NodeKind::Path { path, guide: false, .. } = &n.kind else { continue };
        for (si, sp) in path.subpaths.iter().enumerate() {
            for (ai, a) in sp.anchors.iter().enumerate().take(MAX_ANCHORS.saturating_sub(points.len())) {
                refs.push((n.id, si, ai, sp.anchors.len(), sp.closed));
                points.push(a.p);
            }
        }
    }
    let neighbours = |i: usize, j: usize| {
        let (Some(a), Some(b)) = (refs.get(i), refs.get(j)) else { return false };
        let gap = a.2.abs_diff(b.2);
        a.0 == b.0 && a.1 == b.1 && (gap == 1 || (a.4 && gap + 1 == a.3))
    };
    let mut moves: HashMap<NodeId, Vec<(usize, usize, Point)>> = HashMap::new();
    let mut count = 0;
    for group in po::near_point_groups(&points, NEAR, neighbours) {
        let Some(&to) = group.first().and_then(|&i| points.get(i)) else { continue };
        for &i in &group {
            if let (Some(&(id, si, ai, ..)), Some(&p)) = (refs.get(i), points.get(i))
                && p != to
            {
                moves.entry(id).or_default().push((si, ai, to));
                count += 1;
            }
        }
    }
    let mut ids: Vec<NodeId> = moves.keys().copied().collect();
    ids.sort_unstable();
    for id in &ids {
        let Some(mut p) = d.node(*id).and_then(Node::path_data).cloned() else { continue };
        for &(si, ai, to) in moves.get(id).into_iter().flatten() {
            if let Some(a) = p.anchor_mut(si, ai) {
                a.translate(to - a.p);
            }
        }
        set_path(d, *id, p);
    }
    Ok(Outcome { count, ids, artboards: vec![] })
}

/// Empty Artboards: the artboards with no art on them (as Delete Empty Artboards finds them),
/// deleted with their guides. Only a check of the whole document looks at artboards.
fn artboards(d: &mut Document, sel: &mut Selection, scope: &Scope) -> Result<Outcome> {
    let Scope::Document = scope else { return Ok(Outcome::default()) };
    let empty = empty_artboards(d);
    // The last first, so the others keep their places.
    for &i in empty.iter().rev() {
        if i < d.artboards.len() {
            let id = d.artboards.remove(i).id;
            d.retire_artboard_id(id);
            d.retain_guides(sel, |_, g| g.artboard != Some(id));
        }
    }
    Ok(Outcome { count: empty.len(), ids: vec![], artboards: empty })
}
