use super::{Document, MAX_COORD, NodeId, NodeKind, Selection, cmd};

/// Is `id` a path made into a guide (View → Guides → Make Guides)?
pub(crate) fn is_guide_path(d: &Document, id: NodeId) -> bool {
    d.node(id).is_some_and(|n| matches!(n.kind, NodeKind::Path { guide: true, .. }))
}

/// Deselect the ruler guides and the paths made into guides: while View → Guides → Lock Guides is
/// on, they can't be selected (#1534), whatever selects them. → whether any was selected.
pub(crate) fn deselect_guides(d: &Document, sel: &mut Selection) -> bool {
    let paths: Vec<NodeId> = sel.objects.iter().copied().filter(|id| is_guide_path(d, *id)).collect();
    let any = !paths.is_empty() || !sel.guides.is_empty();
    for id in paths {
        sel.remove(id);
    }
    sel.guides.clear();
    sel.key_guide = None;
    // A key object kept only for the guides to line up on goes with them.
    if sel.key.is_some_and(|k| sel.objects.iter().all(|o| *o == k)) {
        sel.key = None;
    }
    any
}

/// Deselect what the View menu keeps from being selected: the guides while Lock Guides is on
/// ([`deselect_guides`]), and the slices that the Slice Selection tool or a slice command selected
/// ([`Selection::slices`]) while Lock Slices or Hide Slices is on. A selected object that is an
/// object slice stays selected. → whether any was selected.
pub(crate) fn deselect_hidden(menu: &cmd::menucmds::MenuState, d: &Document, sel: &mut Selection) -> bool {
    let guides = menu.guides_locked && deselect_guides(d, sel);
    let slices = (menu.slices_locked || menu.slices_hidden) && !sel.slices.is_empty();
    if slices {
        sel.slices.clear();
    }
    guides || slices
}

/// The error of an edit that would nest objects deeper than [`vectorcraft_doc::MAX_DEPTH`].
pub const TOO_DEEP: &str = "objects can't be nested more than 32 deep";

/// Cheap sanity check after an edit: artboards and the objects just touched (the selection) must
/// have finite, in-range geometry, so saved files always reload and renderers never see NaN/∞.
pub(crate) fn doc_sane(d: &Document, sel: &Selection) -> bool {
    d.ruler_origin.is_finite()
        && d.ruler_origin.x.abs() <= MAX_COORD
        && d.ruler_origin.y.abs() <= MAX_COORD
        && d.artboards.iter().all(|a| rect_in_range(a.rect))
        && d.ruler_guides().iter().all(|g| g.pos.is_finite() && g.pos.abs() <= MAX_COORD)
        && objects_in_range(d, &sel.objects)
}

fn rect_in_range(r: vectorcraft_geom::Rect) -> bool {
    [r.x0, r.y0, r.x1, r.y1].iter().all(|v| v.is_finite() && v.abs() <= MAX_COORD)
}

/// Do the objects `ids` have finite geometry within [`MAX_COORD`] (the ones not there pass)?
///
/// A path or compound path whose anchors and handles are all finite and inside the range by a
/// margin passes without being measured: its curves stay inside their control points. Any other
/// object, and a path with a point outside that margin, is measured.
pub(crate) fn objects_in_range(d: &Document, ids: &[NodeId]) -> bool {
    let paths = d.index_paths(ids.iter().copied());
    ids.iter().all(|id| paths.get(id).and_then(|p| d.node_at(p)).is_none_or(|n| points_in_range(n) || n.geometric_bounds().is_none_or(rect_in_range)))
}

/// Coordinates at most this far from 0 keep a curve's measured bounds within [`MAX_COORD`]
/// whatever the rounding of its extrema.
const POINTS_LIMIT: f64 = MAX_COORD * (1.0 - 1e-9);

/// Whether every anchor and handle of path `n`, or of each path of compound path `n`, is finite and
/// within [`POINTS_LIMIT`]. `false` for any other object.
fn points_in_range(n: &vectorcraft_doc::Node) -> bool {
    let near = |p: vectorcraft_geom::Point| p.x.is_finite() && p.y.is_finite() && p.x.abs() <= POINTS_LIMIT && p.y.abs() <= POINTS_LIMIT;
    match &n.kind {
        NodeKind::Path { path, .. } => path.subpaths.iter().flat_map(|s| &s.anchors).all(|a| near(a.p) && near(a.h_in) && near(a.h_out)),
        NodeKind::Compound { .. } => n.editing_children().iter().all(|c| points_in_range(c)),
        _ => false,
    }
}
