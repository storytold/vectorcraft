use vectorcraft_color::BlendMode;
use vectorcraft_doc::swatches::walk_with_masks;
use vectorcraft_geom::Shape;

use super::axis::radar_axis;
use super::generate::is_dark;
use super::labels::{Line, Role, filled_lines_on, kept_line_rows, label_para, label_size, label_style, line_slot};
use super::{
    Affine, Arc, CharStyle, Color, Document, GraphKind, GraphSpec, HashMap, Node, NodeId, NodeKind, Paint, Rect, Result, Session, Value, bad,
    refresh_bounds,
};

// ---------- generation ----------

/// Series colours: black, then greys (graphs start in greyscale).
pub(super) fn default_series_color(i: usize) -> Color {
    const LEVELS: [f32; 8] = [0.0, 0.55, 0.8, 0.3, 0.68, 0.15, 0.9, 0.42];
    let l = LEVELS[i % LEVELS.len()];
    Color::rgb(l, l, l)
}

pub(super) fn default_series_paint(i: usize) -> Paint {
    Paint::solid(default_series_color(i))
}

/// Fill and stroke for one generated mark of series `i`. A stored fill is used only where the
/// generator paints with the series colour (`series_fill`). A stored stroke replaces that part's
/// own stroke, so a column's stroke stays a stroke and a line's markers do not repaint its line.
pub(super) fn series_marks(g: &GraphSpec, i: usize, series_fill: bool, stroke: Paint, width: f64) -> (Paint, Paint, f64) {
    let slot = g.series_paints.get(i);
    let fill = if series_fill { slot.and_then(|p| p.fill.clone()).unwrap_or_else(|| default_series_paint(i)) } else { Paint::None };
    let stroke = slot.and_then(|p| p.stroke.clone()).unwrap_or(stroke);
    let width = if slot.is_some_and(|p| p.stroke.is_some()) { slot.and_then(|p| p.stroke_width).unwrap_or(width) } else { width };
    (fill, stroke, width)
}

/// The paths directly inside series `i`'s group (`items`) with the series' kept fill and stroke opacity and blend mode on
/// their topmost fill and stroke, and its kept object opacity and blend mode. Other art, such as a graph design, keeps its
/// own. An opacity read from a file is held to 0..1, and one that is not a number is left out.
pub(super) fn with_series_opacity(g: &GraphSpec, i: usize, mut items: Vec<Arc<Node>>) -> Vec<Arc<Node>> {
    let Some(slot) = g.series_paints.get(i) else { return items };
    let kept = |o: Option<f32>| o.filter(|o| o.is_finite()).map(|o| o.clamp(0.0, 1.0));
    let (fill, stroke, object) = (kept(slot.fill_opacity), kept(slot.stroke_opacity), kept(slot.opacity));
    if fill.is_none() && stroke.is_none() && object.is_none() && slot.fill_blend.is_none() && slot.stroke_blend.is_none() && slot.blend.is_none() {
        return items;
    }
    for item in &mut items {
        if !matches!(item.kind, NodeKind::Path { .. }) {
            continue;
        }
        let n = Arc::make_mut(item);
        if let Some(o) = object {
            n.opacity = o;
        }
        if let Some(b) = slot.blend {
            n.blend = b;
        }
        let ap = &mut n.appearance;
        if let Some(f) = ap.fill_mut() {
            if let Some(o) = fill {
                f.opacity = o;
            }
            if let Some(b) = slot.fill_blend {
                f.blend = b;
            }
        }
        if let Some(st) = ap.stroke_mut() {
            if let Some(o) = stroke {
                st.opacity = o;
            }
            if let Some(b) = slot.stroke_blend {
                st.blend = b;
            }
        }
    }
    items
}

/// Where a node inside a graph's generated series art belongs: the graph, the series group and
/// the series index stored on that group.
#[derive(Clone, Copy)]
struct SeriesRef {
    graph: NodeId,
    group: NodeId,
    index: usize,
}

/// How many series groups `spec`'s graph has: a group whose index is past it (the index comes from the file) is not one.
fn series_count(spec: &GraphSpec) -> usize {
    spec.rows.iter().map(Vec::len).max().unwrap_or(0).clamp(1, vectorcraft_doc::MAX_GRAPH_SERIES)
}

/// Every node in a graph's series groups (the groups too), found in one pass over the document.
/// Axes, the legend and groups whose index is past the graph's series (the index comes from the
/// file) are left out.
fn series_members(doc: &Document) -> HashMap<NodeId, SeriesRef> {
    let mut out = HashMap::new();
    doc.walk(|n| {
        let Some(spec) = n.extras().graph.as_deref() else { return };
        let count = series_count(spec);
        for group in n.children().into_iter().flatten() {
            let Some(index) = group.extras().series_index.and_then(|i| usize::try_from(i).ok()).filter(|i| *i < count) else { continue };
            let r = SeriesRef { graph: n.id, group: group.id, index };
            group.walk(&mut |m| {
                out.insert(m.id, r);
            });
        }
    });
    out
}

/// The objects directly in a series group among `roots` and inside them, for an edit that reaches graph series marks
/// without naming them. They may include objects of groups that only look like series groups; the capture functions
/// check each one.
pub(crate) fn series_marks_in(doc: &Document, roots: &[NodeId]) -> Vec<NodeId> {
    let mut out = Vec::new();
    for id in roots {
        if doc.parent_of(*id).and_then(|p| doc.node(p)).is_some_and(|g| g.extras().series_index.is_some()) {
            out.push(*id);
        }
        let Some(root) = doc.node(*id) else { continue };
        root.walk(&mut |n| {
            if n.extras().series_index.is_some() {
                out.extend(n.children().into_iter().flatten().map(|c| c.id));
            }
        });
    }
    out
}

/// Group Selection on a graph series targets its constituent marks and legend swatch, not a
/// group-level appearance which the generated art does not inherit. Other ids pass through.
pub(crate) fn paint_targets(doc: &Document, ids: &[NodeId]) -> Vec<NodeId> {
    let members = series_members(doc);
    if members.is_empty() {
        return ids.to_vec();
    }
    let mut out = Vec::new();
    for id in ids {
        // The series' own marks: the art of a graph design inside it keeps the design's paint (painting it would
        // last only until the graph is drawn again), so edit the design to change it.
        match members.get(id) {
            Some(r) if r.group == *id => {
                if let Some(group) = doc.node(*id) {
                    out.extend(group.children().into_iter().flatten().filter(|m| matches!(m.kind, NodeKind::Path { .. })).map(|m| m.id));
                }
            }
            Some(r) if doc.parent_of(*id) != Some(r.group) || doc.node(*id).is_some_and(|n| !matches!(n.kind, NodeKind::Path { .. })) => {}
            _ => out.push(*id),
        }
    }
    out.sort_unstable();
    out.dedup();
    out
}

/// `ids` with each graph series group replaced by the paths directly inside it, as [`paint_targets`] reads a series
/// group: the graph draws its series groups again with no rows of their own. A graph's Axes group is replaced by the
/// lines and labels directly inside it, and its Legend group by the labels directly inside it, which the graph keeps
/// rows for ([`capture_label_rows`]). A group with no such member, other ids and the art of a graph design pass
/// through, and the order of `ids` is kept.
pub(crate) fn series_group_marks(doc: &Document, ids: &[NodeId]) -> Vec<NodeId> {
    series_group_marks_where(doc, ids, |_| true)
}

/// The lines and labels directly in `n` when it is the Axes group of a graph, or the labels directly in it when it is
/// the graph's Legend group, in paint order. `None` for any other object.
pub(crate) fn graph_part_members(doc: &Document, n: &Node) -> Option<Vec<NodeId>> {
    if !matches!(n.kind, NodeKind::Group { .. }) || n.extras().series_index.is_some() {
        return None;
    }
    let axes = match n.name.as_deref() {
        Some("Axes") => true,
        Some("Legend") => false,
        _ => return None,
    };
    let spec = doc.parent_of(n.id).and_then(|parent| doc.node(parent)).and_then(|parent| parent.extras().graph.as_deref())?;
    let member = |c: &Node| matches!(c.kind, NodeKind::Text(_)) || (axes && line_in(spec, n, c).is_some());
    Some(n.children().into_iter().flatten().filter(|c| member(c)).map(|c| c.id).collect())
}

/// [`series_group_marks`] for the series groups, Axes groups and Legend groups `stands` accepts; the others pass
/// through.
pub(crate) fn series_group_marks_where(doc: &Document, ids: &[NodeId], stands: impl Fn(&Node) -> bool) -> Vec<NodeId> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for id in ids {
        match doc.node(*id) {
            Some(group) if is_series_group(doc, group) && stands(group) => {
                let marks: Vec<NodeId> =
                    group.children().into_iter().flatten().filter(|m| matches!(m.kind, NodeKind::Path { .. })).map(|m| m.id).collect();
                if marks.is_empty() { out.push(*id) } else { out.extend(marks) }
            }
            Some(part) if stands(part) => match graph_part_members(doc, part) {
                Some(members) if !members.is_empty() => out.extend(members),
                _ => out.push(*id),
            },
            _ => out.push(*id),
        }
    }
    out.retain(|id| seen.insert(*id));
    out
}

/// Whether `n` is a graph series group: a child of a graph with one of the graph's series indexes ([`series_members`]).
/// It is looked up through its parent, with no pass over the document: the panels call this while they draw.
fn is_series_group(doc: &Document, n: &Node) -> bool {
    let Some(index) = n.extras().series_index.and_then(|i| usize::try_from(i).ok()) else { return false };
    let graph = doc.parent_of(n.id).and_then(|parent| doc.node(parent)).and_then(|parent| parent.extras().graph.as_deref());
    graph.is_some_and(|spec| index < series_count(spec))
}

/// Whether `id` is the art of a graph design drawn in a graph series (Object › Graph › Column or Marker): an object
/// inside a series group other than a path directly in it. The graph draws that art again from the design.
pub(crate) fn is_design_art(doc: &Document, id: NodeId) -> bool {
    let Some(chain) = doc.ancestry(id) else { return false };
    let Some(at) = chain.iter().position(|a| doc.node(*a).is_some_and(|n| is_series_group(doc, n))) else { return false };
    match chain.len() - at {
        1 => false,
        2 => doc.node(id).is_some_and(|n| !matches!(n.kind, NodeKind::Path { .. })),
        _ => true,
    }
}

/// The paint of the topmost visible fill (`true`) or stroke (`false`) of `node`, and a stroke's weight (0 without paint).
/// With no such row: no paint.
pub(crate) fn shown_paint(node: &Node, fill: bool) -> (Paint, f64) {
    match node.appearance.items.iter().rev().find(|it| it.is_fill() == fill && it.visible()) {
        Some(vectorcraft_doc::AppearanceItem::Stroke(st)) if !st.paint.is_none() => (st.paint.clone(), st.width),
        Some(it) => (it.paint().clone(), 0.0),
        None => (Paint::None, 0.0),
    }
}

/// Copy the paint of the topmost visible fill (`true`) or stroke (`false`), and a stroke's weight, of generated series
/// art just edited (`edited`: each object with the kind of row, later entries replacing earlier ones) into that series'
/// fill or stroke; art left without such a row keeps no paint. Gradients, patterns and swatch links are kept as
/// [`Paint`], not reduced to a solid color. A series that draws Filled Lines bands stores no stroke: its
/// bands keep their own outline. Called inside the same document edit, so both undo together. Returns whether a series'
/// captured fill or stroke changed.
pub(crate) fn capture_series_top_paints(doc: &mut Document, edited: &[(NodeId, bool)]) -> bool {
    if edited.is_empty() {
        return false;
    }
    let members = series_members(doc);
    if members.is_empty() {
        return false;
    }
    // Each series is checked once: the check reads the graph's whole grid.
    let mut bands: HashMap<(NodeId, usize), bool> = HashMap::new();
    let mut captures = Vec::new();
    for (id, fill) in edited {
        // Only the paths the graph draws again, not the copy of a graph design.
        let Some(r) = members.get(id).filter(|r| doc.parent_of(*id) == Some(r.group)) else { continue };
        let Some(node) = doc.node(*id).filter(|n| matches!(n.kind, NodeKind::Path { .. })) else { continue };
        let drawn = || doc.node(r.graph).and_then(|n| n.extras().graph.as_deref()).is_some_and(|g| series_draws_bands(g, r.index));
        if !*fill && *bands.entry((r.graph, r.index)).or_insert_with(drawn) {
            continue;
        }
        let (paint, width) = shown_paint(node, *fill);
        let width = (!*fill).then_some(width);
        captures.push((r.graph, r.index, *fill, paint, width, paint_box(node, width)));
    }
    store_series_paints(doc, captures)
}

/// Does series `i` draw any Filled Lines band? A line needs two neighboring values, a scatter series two neighboring points
/// in different places (its index counts pairs of columns), and a radar series two categories with a value plotted away
/// from the center.
fn series_draws_bands(g: &GraphSpec, i: usize) -> bool {
    let series = g.rows.iter().map(Vec::len).max().unwrap_or(0).clamp(1, vectorcraft_doc::MAX_GRAPH_SERIES);
    if !g.connect_points || !filled_lines_on(g, series) {
        return false;
    }
    let cells = g.cells();
    let cell = |row: &[Option<f64>], s: usize| row.get(s).copied().flatten();
    match g.series_kind(i) {
        GraphKind::Line => cells.windows(2).any(|w| w.iter().all(|row| cell(row, i).is_some())),
        GraphKind::Radar => {
            let (lo, hi, _) = radar_axis(g);
            cells.len() > 1 && cells.iter().any(|row| (cell(row, i).unwrap_or(0.0) - lo) / (hi - lo) > 0.0)
        }
        GraphKind::Scatter => {
            let s = i.saturating_mul(2);
            let point = |row: &[Option<f64>]| Some((cell(row, s)?, cell(row, s.saturating_add(1))?));
            cells.windows(2).any(|w| matches!(w, [a, b] if point(a).is_some() && point(b).is_some() && point(a) != point(b)))
        }
        _ => false,
    }
}

/// Each graph's stored series paints before an edit's first capture into it. A later capture can put back what an earlier
/// one changed, so an edit compares what it leaves with these ([`settle_series_paints`]).
type SeriesPaintsBefore = HashMap<NodeId, Vec<vectorcraft_doc::SeriesPaint>>;

/// What an edit gave the characters of graph label `id`, applied to a character style ([`capture_label_text`]).
pub(crate) type LabelTextChange<'a> = &'a dyn Fn(NodeId, &mut CharStyle);

/// Does any graph in `before` now store other series paints than it did before the edit's first capture? Slots the
/// captures added past the old end and left empty are removed again, so they do not count as a change.
fn settle_series_paints(doc: &mut Document, before: SeriesPaintsBefore) -> bool {
    let mut changed = false;
    for (graph, was) in before {
        let Some(spec) = doc.node_mut(graph).and_then(|n| n.extras_if_any().and_then(|e| e.graph.as_deref_mut())) else { continue };
        let empty = vectorcraft_doc::SeriesPaint::default();
        if spec.series_paints.get(was.len()..).is_some_and(|added| added.iter().all(|slot| *slot == empty)) {
            spec.series_paints.truncate(was.len());
        }
        changed |= spec.series_paints != was;
    }
    changed
}

/// The box a series keeps a gradient relative to: (0, 0)–(1, 1), as graphic styles keep theirs.
const UNIT_BOX: Rect = vectorcraft_doc::GraphicStyle::UNIT_BOX;

/// The gradient `slot` keeps for its fill (`fill`) or stroke as the graph draws it on box `on`: one kept relative to the
/// unit box placed on `on`, also when `on` has no width or no height, and one kept in document coordinates as it is kept.
/// None when the slot keeps no gradient, or keeps one relative to the unit box and there is no box.
fn kept_on(slot: &vectorcraft_doc::SeriesPaint, fill: bool, on: Option<Rect>) -> Option<Paint> {
    let (kept, unit) = if fill { (&slot.fill, slot.fill_unit_box) } else { (&slot.stroke, slot.stroke_unit_box) };
    let Some(Paint::Gradient(g)) = kept else { return None };
    if !unit {
        return Some(Paint::Gradient(g.clone()));
    }
    let on = on?;
    let mut g = g.clone();
    g.rebase(UNIT_BOX, on);
    Some(Paint::Gradient(g))
}

/// Whether `a` and `b` are the same paint, a gradient's vector, focal point and freeform points each within 1e-6 pt (the
/// rounding a move or a placement on another box leaves).
fn same_paint_placed(a: &Paint, b: &Paint) -> bool {
    let (Paint::Gradient(a), Paint::Gradient(b)) = (a, b) else { return a == b };
    let near = |p: vectorcraft_geom::Point, q: vectorcraft_geom::Point| (p - q).hypot() <= 1e-6;
    let mut a = (**a).clone();
    match (&mut a.geom, &b.geom) {
        (Some(ag), Some(bg)) => {
            let focal = match (ag.focal, bg.focal) {
                (Some(p), Some(q)) => near(p, q),
                (p, q) => p.is_none() && q.is_none(),
            };
            if !(near(ag.start, bg.start) && near(ag.end, bg.end) && focal) {
                return false;
            }
            (ag.start, ag.end, ag.focal) = (bg.start, bg.end, bg.focal);
        }
        (None, None) => {}
        _ => return false,
    }
    match (&mut a.freeform, &b.freeform) {
        (Some(af), Some(bf)) => {
            if af.points.len() != bf.points.len() || !af.points.iter().zip(&bf.points).all(|(p, q)| near(p.at, q.at)) {
                return false;
            }
            for (p, q) in af.points.iter_mut().zip(&bf.points) {
                p.at = q.at;
            }
        }
        (None, None) => {}
        _ => return false,
    }
    a == **b
}

/// The box a gradient on `node`'s fill (`width` None) or on its stroke of `width` is placed on: the geometric bounds, grown
/// by half the weight for a stroke. The box can have no width or no height.
fn paint_box(node: &Node, width: Option<f64>) -> Option<Rect> {
    let b = node.geometric_bounds()?;
    Some(width.map_or(b, |w| vectorcraft_doc::appearance::stroke_paint_bounds(b, w)))
}

/// `paint` as a series keeps it: a gradient placed on `from` (the edited mark's [`paint_box`]) is moved to the same place
/// relative to [`UNIT_BOX`], so each mark takes it on its own box. A box with no width or no height, which a gradient's
/// place cannot be kept relative to, leaves the paint in document coordinates. Returns the paint and whether it was moved.
fn unit_box_paint(paint: Paint, from: Option<Rect>) -> (Paint, bool) {
    match (paint, from.filter(|b| b.width() > 1e-9 && b.height() > 1e-9)) {
        (Paint::Gradient(mut g), Some(from)) => {
            g.rebase(from, UNIT_BOX);
            (Paint::Gradient(g), true)
        }
        (paint, _) => (paint, false),
    }
}

/// The paths directly inside series `i`'s group (`items`) with the series' kept gradients, which are kept relative to the
/// unit box, placed on each path's own box: a fill on its geometric bounds, a stroke on its stroke's box. Paints kept in
/// document coordinates (from files saved before the unit box) are left as they are.
pub(super) fn with_series_gradients(g: &GraphSpec, i: usize, mut items: Vec<Arc<Node>>) -> Vec<Arc<Node>> {
    let Some(slot) = g.series_paints.get(i) else { return items };
    fn kept(p: &Option<Paint>, unit: bool) -> Option<&Paint> {
        p.as_ref().filter(|p| unit && matches!(p, Paint::Gradient(_)))
    }
    let (fill, stroke) = (kept(&slot.fill, slot.fill_unit_box), kept(&slot.stroke, slot.stroke_unit_box));
    if fill.is_none() && stroke.is_none() {
        return items;
    }
    // A Filled Lines band takes the kept stroke as its fill when no fill is kept.
    let band_fill = if slot.fill.is_none() { stroke } else { None };
    let fill_from_store = |p: &Paint| fill == Some(p) || band_fill == Some(p);
    let stroke_from_store = |p: &Paint| stroke == Some(p);
    for item in &mut items {
        if !matches!(item.kind, NodeKind::Path { .. }) {
            continue;
        }
        let Some(b) = item.geometric_bounds() else { continue };
        let ap = &item.appearance;
        if !ap.fill().is_some_and(|f| fill_from_store(&f.paint)) && !ap.stroke().is_some_and(|st| stroke_from_store(&st.paint)) {
            continue;
        }
        let ap = &mut Arc::make_mut(item).appearance;
        if let Some(f) = ap.fill_mut().filter(|f| fill_from_store(&f.paint))
            && let Paint::Gradient(gp) = &mut f.paint
        {
            gp.rebase(UNIT_BOX, b);
        }
        if let Some(st) = ap.stroke_mut().filter(|st| stroke_from_store(&st.paint)) {
            let to = st.paint_bounds(b);
            if let Paint::Gradient(gp) = &mut st.paint {
                gp.rebase(UNIT_BOX, to);
            }
        }
    }
    items
}

/// A paint taken from a series mark: graph, series index, fill or stroke, paint, stroke weight and the mark's
/// [`paint_box`].
type Capture = (NodeId, usize, bool, Paint, Option<f64>, Option<Rect>);

/// Store each capture in its series; a gradient is kept relative to the unit box ([`unit_box_paint`]). Returns whether a
/// series' captured fill or stroke differs from what the graph stored before the first capture.
fn store_series_paints(doc: &mut Document, captures: Vec<Capture>) -> bool {
    let mut before = SeriesPaintsBefore::new();
    for (graph, index, fill, paint, width, from) in captures {
        let Some(spec) = doc.node_mut(graph).and_then(|n| n.extras_if_any().and_then(|e| e.graph.as_deref_mut())) else { continue };
        before.entry(graph).or_insert_with(|| spec.series_paints.clone());
        // `index` is below the graph's series count (`series_members`).
        if spec.series_paints.len() <= index {
            spec.series_paints.resize(index + 1, vectorcraft_doc::SeriesPaint::default());
        }
        let Some(slot) = spec.series_paints.get_mut(index) else { continue };
        // A mark that shows the kept gradient as it was drawn leaves the kept gradient as it is.
        let drawn = kept_on(slot, fill, from).is_some_and(|kept| same_paint_placed(&kept, &paint));
        let (paint, unit) = unit_box_paint(paint, from);
        if fill {
            if !drawn {
                (slot.fill, slot.fill_unit_box) = (Some(paint), unit);
            }
        } else {
            if !drawn {
                (slot.stroke, slot.stroke_unit_box) = (Some(paint), unit);
            }
            slot.stroke_width = width;
        }
    }
    settle_series_paints(doc, before)
}

/// Copy the opacity and blend mode of the topmost visible fill (`true`) or stroke (`false`) of generated series art just
/// edited (`edited`: each object with the kind of row) into that series' fill or stroke opacity and blend mode; art left
/// without a row of that kind keeps neither. Called inside the same document edit, so both undo together. Returns
/// whether a series' kept opacity or blend mode changed.
pub(crate) fn capture_series_opacity(doc: &mut Document, edited: &[(NodeId, bool)]) -> bool {
    if edited.is_empty() {
        return false;
    }
    let members = series_members(doc);
    if members.is_empty() {
        return false;
    }
    let captures: Vec<_> = edited
        .iter()
        .filter_map(|(id, fill)| {
            // Only the paths the graph draws again, not the copy of a graph design.
            let r = members.get(id).filter(|r| doc.parent_of(*id) == Some(r.group))?;
            let node = doc.node(*id).filter(|n| matches!(n.kind, NodeKind::Path { .. }))?;
            let row = node.appearance.items.iter().rev().find(|it| it.is_fill() == *fill && it.visible());
            let opacity = row.map_or(1.0, vectorcraft_doc::AppearanceItem::opacity);
            let blend = row.map_or(BlendMode::Normal, vectorcraft_doc::AppearanceItem::blend);
            Some((r.graph, r.index, *fill, (opacity < 1.0).then_some(opacity), (blend != BlendMode::Normal).then_some(blend)))
        })
        .collect();
    let mut before = SeriesPaintsBefore::new();
    for (graph, index, fill, kept, blend) in captures {
        let Some(spec) = doc.node_mut(graph).and_then(|n| n.extras_if_any().and_then(|e| e.graph.as_deref_mut())) else { continue };
        // A fully opaque Normal row on a series the graph keeps nothing for leaves the graph as it is.
        if kept.is_none() && blend.is_none() && spec.series_paints.len() <= index {
            continue;
        }
        before.entry(graph).or_insert_with(|| spec.series_paints.clone());
        // `index` is below the graph's series count (`series_members`).
        if spec.series_paints.len() <= index {
            spec.series_paints.resize(index + 1, vectorcraft_doc::SeriesPaint::default());
        }
        let Some(slot) = spec.series_paints.get_mut(index) else { continue };
        if fill {
            (slot.fill_opacity, slot.fill_blend) = (kept, blend);
        } else {
            (slot.stroke_opacity, slot.stroke_blend) = (kept, blend);
        }
    }
    settle_series_paints(doc, before)
}

/// Copy the object transparency of generated series art just edited (`edited`) into what that series keeps: a path
/// directly in a series group sets the paths' object opacity and blend mode, and the series group itself sets the
/// group's opacity, blend mode, isolation, knockout and knockout shape. Called inside the same document edit, so both undo
/// together. Returns whether what a series keeps changed.
pub(crate) fn capture_series_object_opacity(doc: &mut Document, edited: &[NodeId]) -> bool {
    let mut before = SeriesPaintsBefore::new();
    for (graph, index, group, look) in object_opacity_captures(doc, edited) {
        let Some(spec) = doc.node_mut(graph).and_then(|n| n.extras_if_any().and_then(|e| e.graph.as_deref_mut())) else { continue };
        // A fully opaque Normal object on a series the graph keeps nothing for leaves the graph as it is.
        if look == ObjectOpacity::default() && spec.series_paints.len() <= index {
            continue;
        }
        before.entry(graph).or_insert_with(|| spec.series_paints.clone());
        // `index` is below the graph's series count (`series_members`).
        if spec.series_paints.len() <= index {
            spec.series_paints.resize(index + 1, vectorcraft_doc::SeriesPaint::default());
        }
        let Some(slot) = spec.series_paints.get_mut(index) else { continue };
        if group {
            (slot.group_opacity, slot.group_blend) = (look.opacity, look.blend);
            (slot.group_isolate, slot.group_knockout, slot.group_knockout_shape) = (look.isolate, look.knockout, look.knockout_shape);
        } else {
            (slot.opacity, slot.blend) = (look.opacity, look.blend);
        }
    }
    settle_series_paints(doc, before)
}

/// Would [`capture_series_object_opacity`] change what the graph stores for `edited`? Reads the document without copying
/// it.
pub(crate) fn series_object_opacity_differs(doc: &Document, edited: &[NodeId]) -> bool {
    let mut after: HashMap<(NodeId, usize, bool), ObjectOpacity> = HashMap::new();
    let stored = |graph: NodeId, index: usize, group: bool| {
        let spec = doc.node(graph).and_then(|n| n.extras().graph.as_deref());
        let slot = spec.and_then(|g| g.series_paints.get(index));
        slot.map_or_else(ObjectOpacity::default, |slot| {
            if group {
                ObjectOpacity {
                    opacity: slot.group_opacity,
                    blend: slot.group_blend,
                    isolate: slot.group_isolate,
                    knockout: slot.group_knockout,
                    knockout_shape: slot.group_knockout_shape,
                }
            } else {
                ObjectOpacity { opacity: slot.opacity, blend: slot.blend, ..ObjectOpacity::default() }
            }
        })
    };
    for (graph, index, group, look) in object_opacity_captures(doc, edited) {
        after.insert((graph, index, group), look);
    }
    after.into_iter().any(|((graph, index, group), look)| stored(graph, index, group) != look)
}

/// The series groups among `ids`, for an edit that changes only what a group's transparency has beyond its opacity and
/// blend mode (isolation, knockout and knockout shape), which the graph keeps for series groups alone.
pub(crate) fn series_groups_among(doc: &Document, ids: &[NodeId]) -> Vec<NodeId> {
    ids.iter().copied().filter(|id| doc.node(*id).is_some_and(|n| n.extras().series_index.is_some())).collect()
}

/// An object's transparency as a graph keeps it: the object opacity and blend mode (None for 100% and Normal), and for a
/// series group its isolation, knockout (None for neutral) and knockout shape. Paths keep the first two only.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct ObjectOpacity {
    opacity: Option<f32>,
    blend: Option<BlendMode>,
    isolate: bool,
    knockout: Option<vectorcraft_doc::Knockout>,
    knockout_shape: bool,
}

/// The graph, series, whether it is the series group itself (`true`) or a path directly in it, and the object
/// transparency of each of `edited` that is one of those, in order.
fn object_opacity_captures(doc: &Document, edited: &[NodeId]) -> Vec<(NodeId, usize, bool, ObjectOpacity)> {
    if edited.is_empty() {
        return Vec::new();
    }
    let members = series_members(doc);
    if members.is_empty() {
        return Vec::new();
    }
    edited
        .iter()
        .filter_map(|id| {
            let r = members.get(id)?;
            let group = r.group == *id;
            // Only the series group and the paths the graph draws again, not the copy of a graph design.
            let node = doc.node(*id).filter(|n| group || (doc.parent_of(*id) == Some(r.group) && matches!(n.kind, NodeKind::Path { .. })))?;
            let mut look = ObjectOpacity {
                opacity: (node.opacity < 1.0).then_some(node.opacity),
                blend: (node.blend != BlendMode::Normal).then_some(node.blend),
                ..ObjectOpacity::default()
            };
            if group {
                look.isolate = node.isolate;
                look.knockout = (node.knockout != vectorcraft_doc::Knockout::Neutral).then_some(node.knockout);
                look.knockout_shape = node.knockout_shape;
            }
            Some((r.graph, r.index, group, look))
        })
        .collect()
}

/// Series `i`'s group (`group`) with the object opacity, blend mode, isolation, knockout and knockout shape the graph
/// keeps for it. An opacity read from a file is held to 0..1, and one that is not a number is left out.
pub(super) fn with_series_group_opacity(g: &GraphSpec, i: usize, mut group: Arc<Node>) -> Arc<Node> {
    let Some(slot) = g.series_paints.get(i) else { return group };
    let opacity = slot.group_opacity.filter(|o| o.is_finite()).map(|o| o.clamp(0.0, 1.0));
    if opacity.is_none() && slot.group_blend.is_none() && !slot.group_isolate && slot.group_knockout.is_none() && !slot.group_knockout_shape {
        return group;
    }
    let n = Arc::make_mut(&mut group);
    if let Some(o) = opacity {
        n.opacity = o;
    }
    if let Some(b) = slot.group_blend {
        n.blend = b;
    }
    if let Some(k) = slot.group_knockout {
        n.knockout = k;
    }
    n.isolate |= slot.group_isolate;
    n.knockout_shape |= slot.group_knockout_shape;
    group
}

/// Graph Data › Transpose row/column: categories become series and series categories.
pub(super) fn transpose(g: &mut GraphSpec) {
    let cells = g.cells();
    let width = cells.iter().map(Vec::len).max().unwrap_or(0);
    let flipped: Vec<Vec<Option<f64>>> = (0..width).map(|s| cells.iter().map(|row| row.get(s).copied().flatten()).collect()).collect();
    std::mem::swap(&mut g.series, &mut g.categories);
    g.set_cells(flipped);
}

/// Graph Data › Switch x/y (scatter graphs): each series' y and x columns trade places, with their labels.
pub(super) fn switch_xy(g: &mut GraphSpec) {
    let mut cells = g.cells();
    // Labels for every column, so each pair's labels trade places with its data.
    let width = cells.iter().map(Vec::len).max().unwrap_or(0).max(g.series.len());
    g.series.resize(width, String::new());
    for row in &mut cells {
        for pair in row.as_chunks_mut::<2>().0 {
            pair.swap(0, 1);
        }
    }
    for pair in g.series.as_chunks_mut::<2>().0 {
        pair.swap(0, 1);
    }
    while g.series.last().is_some_and(String::is_empty) {
        g.series.pop();
    }
    g.set_cells(cells);
}

/// Every generated label of the document's graphs: its graph and what it shows. Inside a graph's Axes group the
/// value axis numbers come first and the category labels (one per category with a label) last; a scatter graph's
/// are all numbers, a radar graph's all categories. Legend groups hold legend labels, and a pie graph's own texts
/// name its pies.
fn graph_labels(doc: &Document) -> HashMap<NodeId, (NodeId, Role)> {
    let mut out = HashMap::new();
    doc.walk(|n| {
        let Some(g) = n.extras().graph.as_deref() else { return };
        for (id, role) in own_labels(n, g) {
            out.insert(id, (n.id, role));
        }
    });
    out
}

/// The generated labels of graph `n` (with spec `g`) and what each shows, as [`graph_labels`] finds them.
fn own_labels(n: &Node, g: &GraphSpec) -> Vec<(NodeId, Role)> {
    let mut out = vec![];
    for c in n.children().into_iter().flatten() {
        match (&c.kind, c.name.as_deref(), c.extras().series_index) {
            (NodeKind::Text(_), _, _) if g.kind == GraphKind::Pie => {
                out.push((c.id, Role::Category));
            }
            (NodeKind::Group { .. }, Some("Legend"), None) => {
                for t in c.children().into_iter().flatten().filter(|t| matches!(t.kind, NodeKind::Text(_))) {
                    out.push((t.id, Role::Legend));
                }
            }
            (NodeKind::Group { .. }, Some("Axes"), None) => {
                let texts: Vec<NodeId> = c.children().into_iter().flatten().filter(|t| matches!(t.kind, NodeKind::Text(_))).map(|t| t.id).collect();
                let cats = match g.kind {
                    GraphKind::Scatter => 0,
                    GraphKind::Radar => texts.len(),
                    _ => g.categories.iter().take(g.rows.len().max(1)).filter(|c| !c.is_empty()).count().min(texts.len()),
                };
                for (i, t) in texts.iter().enumerate() {
                    out.push((*t, if i + cats >= texts.len() { Role::Category } else { Role::Value }));
                }
            }
            _ => {}
        }
    }
    out
}

/// Set the labels of graph node `n` (a copy outside any document) in a new graph's label formats: the character and
/// paragraph formats the graph keeps for them are dropped, with the mark that its legend labels' fill is their own, and
/// each label takes its kind's default character style, unscaled (keeping its rotation), and default paragraph
/// attributes, keeping its alignment and where it starts. A pie's legend label on a dark wedge is white, as a redraw
/// sets it. Every other object in the graph goes to `other`. The rest of the graph's art, and what its labels say, stay
/// as they are.
pub(crate) fn plain_labels(n: &mut Node, other: &dyn Fn(&mut Node)) {
    fn plain(l: &mut vectorcraft_doc::LabelStyles) {
        (l.values, l.categories, l.legend) = (None, None, None);
        (l.values_para, l.categories_para, l.legend_para) = (None, None, None);
        l.legend_fill_set = false;
    }
    // Which legend labels a redraw sets on a dark wedge, read while the labels are as the graph drew them.
    let on_dark = match n.extras().graph.as_deref() {
        Some(g) if g.kind == GraphKind::Pie && g.legend && g.pie_legend_in_wedges => {
            let mut redraw = g.clone();
            plain(&mut redraw.label_styles);
            redrawn_on_dark(n, &redraw).unwrap_or_else(|| legend_on_dark(n, g))
        }
        _ => HashMap::new(),
    };
    let Some(g) = n.extras_if_any().and_then(|e| e.graph.as_deref_mut()) else { return };
    plain(&mut g.label_styles);
    let Some(g) = n.extras().graph.as_deref() else { return };
    let roles: HashMap<NodeId, Role> = own_labels(n, g).into_iter().collect();
    struct Plain<'a> {
        roles: HashMap<NodeId, Role>,
        other: &'a dyn Fn(&mut Node),
    }
    fn visit(c: &mut Node, p: &Plain) {
        let role = p.roles.get(&c.id).copied();
        if let (Some(role), NodeKind::Text(t)) = (role, &mut c.kind) {
            let plain = vectorcraft_doc::LabelStyles::default();
            let justify = t.para_at(0).justify;
            let text = t.plain_text();
            t.runs = vec![vectorcraft_doc::TextRun { text, style: label_style(&plain, role), inline: None }];
            let mut para = label_para(&plain, role);
            para.justify = justify;
            t.set_all_paras(para);
            // The label loses its scale and keeps its rotation (and a reflection), so it stays turned with the
            // rest of a rotated graph's art.
            let [a, b, c, d, _, _] = t.xf.as_coeffs();
            let flip = if a * d - b * c < 0.0 { Affine::scale_non_uniform(1.0, -1.0) } else { Affine::IDENTITY };
            t.xf = Affine::translate(t.xf.translation()) * Affine::rotate(b.atan2(a)) * flip;
            refresh_bounds(t);
            return;
        }
        // The Axes and Legend groups, and any other group of the graph's own, are looked through.
        if role.is_none() && matches!(c.kind, NodeKind::Group { .. }) && c.extras().graph.is_none() {
            for k in c.children_mut().into_iter().flatten() {
                visit(Arc::make_mut(k), p);
            }
            return;
        }
        (p.other)(c);
    }
    let p = Plain { roles, other };
    for c in n.children_mut().into_iter().flatten() {
        visit(Arc::make_mut(c), &p);
    }
    set_legend_fills(n, &on_dark);
}

/// Give each legend label of graph `n` in `on_dark` white characters (`true`) or black ones.
fn set_legend_fills(n: &mut Node, on_dark: &HashMap<NodeId, bool>) {
    if on_dark.is_empty() {
        return;
    }
    // The legend labels are in the graph's Legend group.
    for c in n.children_mut().into_iter().flatten() {
        if !c.children().into_iter().flatten().any(|t| on_dark.contains_key(&t.id)) {
            continue;
        }
        for t in Arc::make_mut(c).children_mut().into_iter().flatten() {
            let Some(dark) = on_dark.get(&t.id) else { continue };
            let fill = Paint::solid(if *dark { Color::WHITE } else { Color::BLACK });
            if matches!(&t.kind, NodeKind::Text(text) if text.runs.iter().all(|r| r.style.fill == fill)) {
                continue;
            }
            if let NodeKind::Text(text) = &mut Arc::make_mut(t).kind {
                text.runs.iter_mut().for_each(|r| r.style.fill = fill.clone());
            }
        }
    }
}

/// Whether each wedge shows a dark fill, for each pie in `n` (`n` itself or one inside it, opacity-mask art included)
/// with Legends in Wedges whose legend labels' fill is not their own, so a redraw sets it by its wedge. Read before a
/// paste links the pie's colors to the document's swatches, for [`follow_wedge_shades`].
pub(crate) fn wedge_shades(n: &Node) -> HashMap<NodeId, Vec<bool>> {
    let mut out = HashMap::new();
    walk_with_masks(n, &mut |m| {
        if let Some(g) = m.extras().graph.as_deref()
            && g.kind == GraphKind::Pie
            && g.legend
            && g.pie_legend_in_wedges
            && !g.label_styles.legend_fill_is_own()
        {
            out.insert(m.id, pie_wedges(m).iter().map(|w| w.dark).collect());
        }
    });
    out
}

/// Give the legend labels of each pie in `n` that [`wedge_shades`] read as `was` the contrast a redraw sets, where a
/// wedge now shows a fill of the other darkness: white on a dark wedge, black on a light one. Pies whose wedges all kept
/// their darkness keep their labels as they are.
pub(crate) fn follow_wedge_shades(n: &mut Node, was: &HashMap<NodeId, Vec<bool>>) {
    let mut changed = vec![];
    walk_with_masks(n, &mut |m| {
        if let Some(before) = was.get(&m.id)
            && pie_wedges(m).iter().map(|w| w.dark).ne(before.iter().copied())
        {
            changed.push(m.id);
        }
    });
    if !changed.is_empty() {
        restore_contrast(n, &changed);
    }
}

/// Set the legend labels of the pies `ids` in `n` (`n` itself or ones inside it, opacity-mask art included) as a redraw
/// sets them on their wedges.
fn restore_contrast(n: &mut Node, ids: &[NodeId]) {
    if ids.contains(&n.id) {
        let on_dark = match n.extras().graph.as_deref() {
            Some(g) => redrawn_on_dark(n, g).unwrap_or_else(|| legend_on_dark(n, g)),
            None => HashMap::new(),
        };
        set_legend_fills(n, &on_dark);
    }
    let holds = |c: &Node| {
        let mut found = false;
        walk_with_masks(c, &mut |m| found |= ids.contains(&m.id));
        found
    };
    if n.extras().mask.as_ref().is_some_and(|m| holds(&m.art))
        && let Some(m) = n.extras_if_any().and_then(|e| e.mask.as_mut())
    {
        restore_contrast(Arc::make_mut(&mut m.art), ids);
    }
    for c in n.children_mut().into_iter().flatten() {
        if holds(c) {
            restore_contrast(Arc::make_mut(c), ids);
        }
    }
}

/// The legend labels of graph `n`: the type in its Legend group, as [`own_labels`] finds them.
fn legend_labels(n: &Node) -> Vec<&Node> {
    legend_in(n.children().map_or(&[], |c| c.as_slice()))
}

/// The type in the Legend group among a graph's `children`.
fn legend_in(children: &[Arc<Node>]) -> Vec<&Node> {
    children
        .iter()
        .filter(|c| matches!(c.kind, NodeKind::Group { .. }) && c.name.as_deref() == Some("Legend") && c.extras().series_index.is_none())
        .flat_map(|c| c.children().into_iter().flatten())
        .map(|t| t.as_ref())
        .filter(|t| matches!(t.kind, NodeKind::Text(_)))
        .collect()
}

/// A pie wedge: its series, its outline and whether the fill it shows is dark.
struct Wedge {
    series: u32,
    bounds: vectorcraft_geom::Rect,
    path: vectorcraft_geom::BezPath,
    dark: bool,
}

/// The wedges of pie `n` (the paths of its series groups), in paint order.
fn pie_wedges(n: &Node) -> Vec<Wedge> {
    wedges_in(n.children().map_or(&[], |c| c.as_slice()))
}

/// The wedges among a pie's `children`, as [`pie_wedges`] finds them.
fn wedges_in(children: &[Arc<Node>]) -> Vec<Wedge> {
    let mut wedges = vec![];
    for c in children {
        let Some(series) = c.extras().series_index else { continue };
        c.walk(&mut |m| {
            if let NodeKind::Path { path, .. } = &m.kind {
                let path = path.to_bezpath();
                wedges.push(Wedge { series, bounds: path.bounding_box(), path, dark: is_dark(&shown_paint(m, true).0) });
            }
        });
    }
    wedges
}

/// The text of the label of each series of graph `g` in its wedges, with the series that show it (several when series
/// share a name).
fn series_by_label(g: &GraphSpec) -> HashMap<String, Vec<u32>> {
    let mut names: HashMap<String, Vec<u32>> = HashMap::new();
    for (s, name) in g.series.iter().enumerate() {
        if let Ok(s) = u32::try_from(s) {
            names.entry(name.replace('|', "\n")).or_default().push(s);
        }
    }
    names
}

/// The legend labels of pie `n`, each with whether a redraw from spec `g` sets it on a dark wedge, by the fill each wedge
/// of `n` shows now. The pie is drawn again apart from any document, which places each label in its wedge as `g` sets
/// it and tells each label's series. Each redrawn wedge takes the fill of the wedge of `n` of its series with the same
/// bounds, and each legend label of `n` is the nearest redrawn one with the same text, places measured in the bounds of
/// all the wedges. `None` when a wedge or a label has no counterpart of its own, or a wedge out of its place in paint
/// order has several (art or labels added, removed, edited or restyled since the pie was drawn).
fn redrawn_on_dark(n: &Node, g: &GraphSpec) -> Option<HashMap<NodeId, bool>> {
    use vectorcraft_geom::kurbo::ParamCurveNearest;
    // Each series name that shows a label is drawn with a mark and its index after it, which moves no line, so each
    // redrawn label tells its series.
    const MARK: char = '\u{1f}';
    let mut marked = g.clone();
    for (s, name) in marked.series.iter_mut().enumerate() {
        if !super::labels::blank_label(name) {
            name.push(MARK);
            name.push_str(&s.to_string());
        }
    }
    let legend = legend_labels(n);
    let mut scratch = Document::new(1.0, 1.0);
    let art = super::generate::generate(&mut scratch, &marked);
    let redrawn = legend_in(&art);
    let now = pie_wedges(n);
    let mut wedges = wedges_in(&art);
    if redrawn.len() != legend.len() || wedges.len() != now.len() {
        return None;
    }
    // Places are measured in the bounds of all the wedges, since the pie may have been moved or resized since it was drawn.
    let frame = |ws: &[Wedge]| ws.iter().map(|w| w.bounds).reduce(|a, b| a.union(b));
    let (Some(was), Some(is)) = (frame(&wedges), frame(&now)) else { return None };
    let at = |f: vectorcraft_geom::Rect, p: vectorcraft_geom::Point| ((p.x - f.x0) / f.width().max(1e-9), (p.y - f.y0) / f.height().max(1e-9));
    let corners = |f: vectorcraft_geom::Rect, w: &Wedge| {
        let (a, b) = (at(f, vectorcraft_geom::Point::new(w.bounds.x0, w.bounds.y0)), at(f, vectorcraft_geom::Point::new(w.bounds.x1, w.bounds.y1)));
        [a.0, a.1, b.0, b.1]
    };
    // A wedge and a redrawn one are the same wedge when they show the same series with the same bounds. Wedges are
    // compared this many times in all, labels included; past it, nothing is matched.
    let mut budget: usize = 1 << 20;
    let mut copied_wedges: HashMap<u32, Vec<(usize, [f64; 4])>> = HashMap::new();
    for (k, w) in now.iter().enumerate() {
        copied_wedges.entry(w.series).or_default().push((k, corners(is, w)));
    }
    // The wedge in the same place in paint order when its bounds are the same, else the one of the series whose bounds
    // are nearest, when they are the same to within 0.001 of the whole; each copied wedge is matched once.
    let off = |c: &[f64; 4], d: &[f64; 4]| c.iter().zip(d).map(|(p, q)| (p - q).abs()).fold(0.0, f64::max);
    let mut used = vec![false; now.len()];
    let mut fills = Vec::with_capacity(wedges.len());
    for (i, r) in wedges.iter().enumerate() {
        let c = corners(was, r);
        let k = match now.get(i) {
            Some(w) if w.series == r.series && off(&c, &corners(is, w)) <= 1e-7 => i,
            _ => {
                let group = copied_wedges.get(&r.series)?;
                if budget < group.len() {
                    return None;
                }
                budget -= group.len();
                let (k, d) = group.iter().map(|(k, d)| (*k, off(&c, d))).min_by(|a, b| a.1.total_cmp(&b.1))?;
                if d > 1e-3 {
                    return None;
                }
                k
            }
        };
        if std::mem::replace(used.get_mut(k)?, true) {
            return None;
        }
        fills.push(now.get(k)?.dark);
    }
    for (r, dark) in wedges.iter_mut().zip(fills) {
        r.dark = dark;
    }
    // Each label's words and origin, in the frame of its pie's wedges.
    let place = |labels: &[&Node], f| {
        labels
            .iter()
            .map(|t| match &t.kind {
                NodeKind::Text(text) => Some((text.plain_text(), at(f, text.xf * vectorcraft_geom::Point::ORIGIN))),
                _ => None,
            })
            .collect::<Option<Vec<_>>>()
    };
    let (Some(copied), Some(mut drawn)) = (place(&legend, is), place(&redrawn, was)) else { return None };
    // The series of each redrawn label, and its words without the mark.
    let mut series = Vec::with_capacity(drawn.len());
    for (words, _) in &mut drawn {
        let (said, s) = words.rsplit_once(MARK)?;
        let s: usize = s.parse().ok()?;
        let name = g.series.get(s)?;
        if *said != name.replace('|', "\n") {
            return None;
        }
        series.push((s, name.as_str()));
        words.truncate(said.len());
    }
    // Each label is the nearest redrawn label that says the same, and no two labels are the same redrawn one.
    let mut same: HashMap<&str, Vec<usize>> = HashMap::new();
    for (j, (words, _)) in drawn.iter().enumerate() {
        same.entry(words.as_str()).or_default().push(j);
    }
    let mut taken = vec![false; drawn.len()];
    let mut matched = Vec::with_capacity(copied.len());
    for (words, p) in &copied {
        let group = same.get(words.as_str())?;
        if budget < group.len() {
            return None;
        }
        budget -= group.len();
        let away = |j: &usize| drawn.get(*j).map_or(f64::INFINITY, |(_, q)| (p.0 - q.0).powi(2) + (p.1 - q.1).powi(2));
        let j = *group.iter().min_by(|a, b| away(a).total_cmp(&away(b)))?;
        if std::mem::replace(taken.get_mut(j)?, true) {
            return None;
        }
        matched.push(j);
    }
    let mut by_series: HashMap<u32, Vec<&Wedge>> = HashMap::new();
    for w in &wedges {
        by_series.entry(w.series).or_default().push(w);
    }
    // A redraw puts the middle of a label's lines (its series name split at `|` and line breaks, at the default leading)
    // 0.35 of the label size below a point inside its wedge, at the label's origin across. The label is on the nearest
    // wedge of its series to that point, the one holding it. When the wedges of the series are all dark or all light, the
    // place does not matter.
    let uniform: HashMap<u32, Option<bool>> =
        by_series.iter().map(|(s, ws)| (*s, ws.first().map(|w| w.dark).filter(|d| ws.iter().all(|w| w.dark == *d)))).collect();
    let ls = label_size(&g.label_styles, Role::Legend);
    let mut out = HashMap::new();
    for (t, j) in legend.iter().zip(matched) {
        let (Some(r), Some((s, name))) = (redrawn.get(j), series.get(j).copied()) else { return None };
        let s = u32::try_from(s).ok()?;
        if let Some(Some(dark)) = uniform.get(&s) {
            out.insert(t.id, *dark);
            continue;
        }
        let NodeKind::Text(text) = &r.kind else { return None };
        let middle = name.split(['|', '\n']).count().saturating_sub(1) as f64 * ls * 1.2 / 2.0;
        let point = text.xf * vectorcraft_geom::Point::new(0.0, middle - ls * 0.35);
        let named = by_series.get(&s)?;
        if budget < named.len() {
            return None;
        }
        budget -= named.len();
        // The nearest wedge, measured exactly only while its bounds are nearer than the nearest one found so far.
        let to_box = |b: vectorcraft_geom::Rect| {
            let (dx, dy) = ((b.x0 - point.x).max(point.x - b.x1).max(0.0), (b.y0 - point.y).max(point.y - b.y1).max(0.0));
            dx * dx + dy * dy
        };
        let away = |w: &Wedge| {
            if w.path.contains(point) { 0.0 } else { w.path.segments().map(|seg| seg.nearest(point, 1e-3).distance_sq).fold(f64::INFINITY, f64::min) }
        };
        let mut by_box: Vec<(f64, &Wedge)> = named.iter().map(|w| (to_box(w.bounds), *w)).collect();
        by_box.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut best: Option<(f64, bool)> = None;
        for (near, w) in by_box {
            if best.is_some_and(|b| near >= b.0) {
                break;
            }
            let d = away(w);
            if best.is_none_or(|b| d < b.0) {
                best = Some((d, w.dark));
            }
        }
        out.insert(t.id, best?.1);
    }
    Some(out)
}

/// The legend labels of graph `n` (a pie with Legends in Wedges, its labels where it drew them), each with whether a
/// redraw sets it on a dark wedge, by the fill each wedge shows now. A label is placed by the point a redraw centers it
/// on, which lies inside its wedge: the wedge holding it of the series the label names (the nearest one when none holds
/// it), or of any series when the label names none. When the wedges of the series a label names are all dark or all
/// light, the label's place does not matter.
fn legend_on_dark(n: &Node, g: &GraphSpec) -> HashMap<NodeId, bool> {
    wedge_contrast(&legend_labels(n), &pie_wedges(n), g)
}

/// The point a redraw centers legend label `text` of series `name` on, in a pie of spec `g` whose legend labels are
/// `ls` points: 0.35 of the label size above where a redraw (`label` in `labels.rs`) puts the middle of the label's
/// lines, at the label's origin across. The lines are those of the series name, split at `|` and line breaks, at the
/// default leading; with kept paragraph attributes, a redraw puts the middle of the first and last baselines there
/// instead, when each of those lines that holds characters has one baseline and the first and last hold characters.
fn label_anchor(text: &vectorcraft_doc::TextObject, name: &str, g: &GraphSpec, ls: f64) -> vectorcraft_geom::Point {
    let lines: Vec<&str> = name.split(['|', '\n']).collect();
    let ys: Vec<f64> = text.baselines().map(|(a, _)| a.y).collect();
    let set = lines.iter().filter(|l| !l.is_empty()).count();
    let ends_set = lines.first().is_some_and(|l| !l.is_empty()) && lines.last().is_some_and(|l| !l.is_empty());
    let middle = match (ys.first(), ys.last()) {
        (Some(first), Some(last)) if g.label_styles.legend_para.is_some() && ends_set && ys.len() == set => (first + last) / 2.0,
        _ => lines.len().saturating_sub(1) as f64 * ls * 1.2 / 2.0,
    };
    text.xf * vectorcraft_geom::Point::new(0.0, middle - ls * 0.35)
}

/// [`legend_on_dark`] for legend labels `legend` of a pie of spec `g` with wedges `wedges`.
fn wedge_contrast(legend: &[&Node], wedges: &[Wedge], g: &GraphSpec) -> HashMap<NodeId, bool> {
    use vectorcraft_geom::kurbo::ParamCurveNearest;
    let mut out = HashMap::new();
    if legend.is_empty() {
        return out;
    }
    let mut by_series: HashMap<u32, Vec<&Wedge>> = HashMap::new();
    for w in wedges {
        by_series.entry(w.series).or_default().push(w);
    }
    // For each label text, the wedges of the series that show it, and how many of them are dark.
    let named_wedges: HashMap<String, (Vec<&Wedge>, usize)> = series_by_label(g)
        .into_iter()
        .map(|(name, series)| {
            let named: Vec<&Wedge> = series.iter().filter_map(|s| by_series.get(s)).flatten().copied().collect();
            let darks = named.iter().filter(|w| w.dark).count();
            (name, (named, darks))
        })
        .collect();
    let none = (vec![], 0);
    // For each label text, the first series name that shows it (names that differ only in `|` against a line break of
    // their own show the same text, and nothing in the label tells them apart).
    let mut series_names: HashMap<String, &str> = HashMap::new();
    for name in &g.series {
        series_names.entry(name.replace('|', "\n")).or_insert(name.as_str());
    }
    let ls = label_size(&g.label_styles, Role::Legend);
    // The search for the nearest wedge looks at this many wedges in all; past it, a label takes the contrast most of the
    // wedges of its series show.
    let mut budget: usize = 1 << 20;
    for t in legend {
        let NodeKind::Text(text) = &t.kind else { continue };
        let id = t.id;
        let words = text.plain_text();
        let (named, darks) = named_wedges.get(&words).unwrap_or(&none);
        let darks = *darks;
        if (darks == 0 || darks == named.len())
            && let Some(w) = named.first()
        {
            out.insert(id, w.dark);
            continue;
        }
        let name = series_names.get(&words).map_or_else(|| words.replace('\n', "|"), |s| (*s).to_owned());
        let at = label_anchor(text, &name, g, ls);
        if named.is_empty() {
            let cost = wedges.len();
            let dark = budget >= cost && wedges.iter().any(|w| w.dark && w.bounds.contains(at) && w.path.contains(at));
            budget = budget.saturating_sub(cost);
            out.insert(id, dark);
            continue;
        }
        if budget < named.len() {
            out.insert(id, darks * 2 > named.len());
            continue;
        }
        budget -= named.len();
        // The nearest wedge, measured exactly only while its bounds are nearer than the nearest one found so far.
        let to_box = |r: vectorcraft_geom::Rect| {
            let (dx, dy) = ((r.x0 - at.x).max(at.x - r.x1).max(0.0), (r.y0 - at.y).max(at.y - r.y1).max(0.0));
            dx * dx + dy * dy
        };
        let away = |w: &Wedge| {
            if w.path.contains(at) { 0.0 } else { w.path.segments().map(|s| s.nearest(at, 1e-3).distance_sq).fold(f64::INFINITY, f64::min) }
        };
        let mut by_box: Vec<(f64, &Wedge)> = named.iter().map(|w| (to_box(w.bounds), *w)).collect();
        by_box.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut best = (f64::INFINITY, darks * 2 > named.len());
        for (near, w) in by_box {
            if near >= best.0 {
                break;
            }
            let d = away(w);
            if d < best.0 {
                best = (d, w.dark);
            }
        }
        out.insert(id, best.1);
    }
    out
}

/// Keep the anti-aliasing method `method` given to graph labels `ids` for every label of their kind, as
/// [`capture_label_styles`] keeps a formatting change. Returns whether a kept method changed.
pub(crate) fn capture_label_anti_alias(doc: &mut Document, ids: &[NodeId], method: vectorcraft_doc::text::TypeAntiAlias) -> bool {
    let labels = graph_labels(doc);
    let mut kinds: Vec<(NodeId, Role)> = ids.iter().filter_map(|id| labels.get(id).copied()).collect();
    kinds.sort_by_key(|(g, r)| (g.0, *r as u8));
    kinds.dedup();
    let mut changed = false;
    for (graph, role) in kinds {
        let Some(spec) = doc.node_mut(graph).and_then(|n| n.extras_if_any().and_then(|e| e.graph.as_deref_mut())) else { continue };
        let slot = match role {
            Role::Value => &mut spec.label_styles.values_anti_alias,
            Role::Category => &mut spec.label_styles.categories_anti_alias,
            Role::Legend => &mut spec.label_styles.legend_anti_alias,
        };
        changed |= *slot != Some(method);
        *slot = Some(method);
    }
    changed
}

/// Keep a formatting change made to graph labels `ids` (Character or Appearance, on the whole graph or on labels
/// picked with Group Selection) for every label of their kind: `change` is applied to the kind's kept style (or
/// the default one), so the labels look the same when the graph is drawn again. Called inside the same edit, so
/// they undo together. A kind that keeps no style and that `change` leaves at the default style keeps none, unless
/// `change` sets the legend fill. Returns whether a kept style changed.
pub(crate) fn capture_label_styles(doc: &mut Document, ids: &[NodeId], change: &dyn Fn(&mut CharStyle)) -> bool {
    let mut changed = false;
    for (graph, role) in label_kinds(doc, ids) {
        changed |= store_label_style(doc, graph, role, change, true);
    }
    changed
}

/// [`capture_label_styles`], except that a kind that keeps no style keeps the default style when `change` leaves it
/// unchanged, for a command that gives character attributes (a size equal to the labels' own, for example).
pub(crate) fn capture_label_styles_given(doc: &mut Document, ids: &[NodeId], change: &dyn Fn(&mut CharStyle)) -> bool {
    let mut changed = false;
    for (graph, role) in label_kinds(doc, ids) {
        changed |= store_label_style(doc, graph, role, change, false);
    }
    changed
}

/// Apply `change` to the style graph `graph` keeps for its labels of kind `role` (or the default one). A `change` that
/// sets the fill of legend labels marks it as set on them, so a black one stays black on dark pie wedges. Returns whether
/// the kept style or that mark changed. With `unless_default`, a kind that keeps no style and that `change` leaves at
/// the default style keeps none, unless `change` sets the legend fill.
fn store_label_style(doc: &mut Document, graph: NodeId, role: Role, change: &dyn Fn(&mut CharStyle), unless_default: bool) -> bool {
    let Some(spec) = doc.node_mut(graph).and_then(|n| n.extras_if_any().and_then(|e| e.graph.as_deref_mut())) else { return false };
    let was = label_style(&spec.label_styles, role);
    let mut style = was.clone();
    // The kept fill can already be the one `change` sets (black, the default): `change` sets the fill when it gives one
    // of two different fills something else.
    let sets_fill = role == Role::Legend
        && [Paint::None, Paint::solid(Color::BLACK)].into_iter().any(|fill| {
            let mut probe = CharStyle { fill: fill.clone(), ..style.clone() };
            change(&mut probe);
            probe.fill != fill
        });
    change(&mut style);
    let styles = &mut spec.label_styles;
    let slot = match role {
        Role::Value => &mut styles.values,
        Role::Category => &mut styles.categories,
        Role::Legend => &mut styles.legend,
    };
    if unless_default && slot.is_none() && style == was && !(sets_fill && !styles.legend_fill_set) {
        return false;
    }
    let mut changed = slot.as_ref() != Some(&style);
    *slot = Some(style);
    if sets_fill && !styles.legend_fill_set {
        styles.legend_fill_set = true;
        changed = true;
    }
    changed
}

/// Keep a paragraph change made to all of the text of graph labels `ids` (Paragraph, on the whole graph or on labels
/// picked with Group Selection) for every label of their kind, as [`capture_label_styles`] does for character
/// formats. Returns whether kept attributes changed, or the error `change` returns for the kept attributes.
pub(crate) fn capture_label_paragraphs(
    doc: &mut Document,
    ids: &[NodeId],
    change: &dyn Fn(&mut vectorcraft_doc::ParaStyle) -> Result<()>,
) -> Result<bool> {
    let mut changed = false;
    for (graph, role) in label_kinds(doc, ids) {
        let Some(spec) = doc.node_mut(graph).and_then(|n| n.extras_if_any().and_then(|e| e.graph.as_deref_mut())) else { continue };
        let mut para = label_para(&spec.label_styles, role);
        change(&mut para)?;
        let slot = match role {
            Role::Value => &mut spec.label_styles.values_para,
            Role::Category => &mut spec.label_styles.categories_para,
            Role::Legend => &mut spec.label_styles.legend_para,
        };
        changed |= slot.as_ref() != Some(&para);
        *slot = Some(para);
    }
    Ok(changed)
}

/// The object rows graph label `n` shows ([`vectorcraft_doc::LabelRows`]): the paint, weight, opacity and blend mode of
/// its topmost visible fill and stroke row, its object opacity and blend mode, and where those rows paint. A placed
/// gradient is kept without its placement, so it fits each label it lands on. `None`: no visible row, opaque and Normal.
pub(crate) fn label_rows(n: &Node) -> Option<vectorcraft_doc::LabelRows> {
    use vectorcraft_doc::AppearanceItem;
    let items = &n.appearance.items;
    let top = |fill: bool| items.iter().rposition(|it| it.is_fill() == fill && it.visible());
    let opacity = |o: f32| (o != 1.0).then_some(o);
    let blend = |b: BlendMode| (b != BlendMode::Normal).then_some(b);
    let unplaced = crate::cmd::gradient::unplaced;
    let mut rows = vectorcraft_doc::SeriesPaint { opacity: opacity(n.opacity), blend: blend(n.blend), ..Default::default() };
    let (fill, stroke) = (top(true), top(false));
    if let Some(f) = fill.and_then(|i| items.get(i)) {
        (rows.fill, rows.fill_opacity, rows.fill_blend) = (Some(unplaced(f.paint())), opacity(f.opacity()), blend(f.blend()));
    }
    if let Some(AppearanceItem::Stroke(st)) = stroke.and_then(|i| items.get(i)) {
        (rows.stroke, rows.stroke_width, rows.stroke_opacity, rows.stroke_blend) =
            (Some(unplaced(&st.paint)), Some(st.width), opacity(st.opacity), blend(st.blend));
    }
    let contents = n.appearance.contents_at();
    let below = [fill, stroke].into_iter().flatten().filter(|i| *i < contents).fold(0u8, |k, _| k + 1);
    let rows = vectorcraft_doc::LabelRows {
        paint: rows,
        below: (below > 0).then_some(below),
        stroke_under: matches!((fill, stroke), (Some(f), Some(s)) if s < f),
    };
    (rows != vectorcraft_doc::LabelRows::default()).then_some(rows)
}

/// The graph and kind of graph line `id`: a path directly in a graph's Axes group. Looked up through its parents, with no
/// pass over the document.
fn graph_line(doc: &Document, id: NodeId) -> Option<(NodeId, Line)> {
    let line = doc.node(id)?;
    if !matches!(line.kind, NodeKind::Path { .. }) {
        return None;
    }
    let path = doc.ancestry(id)?;
    let [.., graph, axes, _] = path.as_slice() else { return None };
    let spec = doc.node(*graph)?.extras().graph.as_deref()?;
    Some((*graph, line_in(spec, doc.node(*axes)?, line)?))
}

/// The kind of graph line `line` is when `axes` is the Axes group of a graph drawn from `spec` and `line` a path directly
/// in it. A radar graph's rings are its closed paths; its spokes, open, are axis lines.
fn line_in(spec: &GraphSpec, axes: &Node, line: &Node) -> Option<Line> {
    if axes.name.as_deref() != Some("Axes") || axes.extras().series_index.is_some() || !matches!(axes.kind, NodeKind::Group { .. }) {
        return None;
    }
    let NodeKind::Path { path, .. } = &line.kind else { return None };
    Some(if spec.kind == GraphKind::Radar && path.is_closed() { Line::Ring } else { Line::Axis })
}

/// The graph lines among `ids`, each with its graph and kind, from the back to the front and each once: a later line's
/// capture replaces an earlier one's, so the frontmost line of a kind sets what the kind keeps.
fn lines_of(doc: &Document, ids: &[NodeId]) -> Vec<(NodeId, NodeId, Line)> {
    let mut out: Vec<(NodeId, NodeId, Line)> = ids.iter().filter_map(|id| graph_line(doc, *id).map(|(g, line)| (*id, g, line))).collect();
    out.sort_by_cached_key(|(id, ..)| doc.index_path(*id));
    out.dedup_by_key(|(id, ..)| *id);
    out
}

/// Which of the object rows ([`label_rows`]) a graph label or line shows its kind keeps after an edit.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Keep {
    /// All of them.
    All,
    /// Only the object opacity and blend mode; the fill and stroke rows stay as the kind keeps them.
    Object,
    /// The fill and stroke rows; the object opacity and blend mode stay as the kind keeps them.
    Rows,
    /// The fill row with `fill`, the stroke row with `stroke`, and the object opacity and blend mode with `object`; the
    /// others stay as the kind keeps them. Each row keeps its place relative to the characters; a row taken from the
    /// label or line paints above a kept row on the same side of them, unless the label or line shows it below its own row
    /// of that kind.
    Kinds { fill: bool, stroke: bool, object: bool },
}

/// The kinds (true: fill) of the rows in `rows` from the bottom up, each with whether it paints below the characters.
fn row_places(rows: &vectorcraft_doc::LabelRows) -> Vec<(bool, bool)> {
    let order = if rows.stroke_under { [false, true] } else { [true, false] };
    let has = |fill: bool| if fill { rows.paint.fill.is_some() } else { rows.paint.stroke.is_some() };
    let below = usize::from(rows.below.unwrap_or(0));
    order.into_iter().filter(|k| has(*k)).enumerate().map(|(i, k)| (k, i < below)).collect()
}

/// Where the rows paint when the row of kind `taken` (true: fill) comes from `shown` and the row of the other kind from
/// `kept`, each keeping its place relative to the characters: `below` and `stroke_under` of [`vectorcraft_doc::LabelRows`].
fn merged_places(kept: &vectorcraft_doc::LabelRows, shown: &vectorcraft_doc::LabelRows, taken: bool) -> (Option<u8>, bool) {
    let in_shown = row_places(shown);
    let t = in_shown.iter().copied().find(|p| p.0 == taken);
    let o = row_places(kept).into_iter().find(|p| p.0 != taken);
    let order = match (t, o) {
        (Some(t), Some(o)) => {
            // Rows on different sides of the characters: the one below them is lower. On the same side: as the label or
            // line shows them, else the taken row on top.
            let at = |kind: bool| in_shown.iter().position(|p| p.0 == kind);
            let o_first = if t.1 != o.1 {
                o.1
            } else {
                match (at(!taken), at(taken)) {
                    (Some(o_at), Some(t_at)) => o_at < t_at,
                    _ => true,
                }
            };
            if o_first { vec![o, t] } else { vec![t, o] }
        }
        (t, o) => t.into_iter().chain(o).collect(),
    };
    let below = order.iter().filter(|p| p.1).fold(0u8, |k, _| k + 1);
    ((below > 0).then_some(below), order.len() == 2 && !order[0].0)
}

/// `shown` with the parts of the rows `kept` keeps that `keep` leaves as they are.
fn keep_rows(kept: vectorcraft_doc::LabelRows, shown: vectorcraft_doc::LabelRows, keep: Keep) -> vectorcraft_doc::LabelRows {
    match keep {
        Keep::All => shown,
        Keep::Object => {
            let mut kept = kept;
            (kept.paint.opacity, kept.paint.blend) = (shown.paint.opacity, shown.paint.blend);
            kept
        }
        Keep::Rows => {
            let mut shown = shown;
            (shown.paint.opacity, shown.paint.blend) = (kept.paint.opacity, kept.paint.blend);
            shown
        }
        Keep::Kinds { fill, stroke, object } => {
            let places = match (fill, stroke) {
                (true, true) => (shown.below, shown.stroke_under),
                (false, false) => (kept.below, kept.stroke_under),
                (taken, _) => merged_places(&kept, &shown, taken),
            };
            let mut rows = shown;
            (rows.below, rows.stroke_under) = places;
            let (r, k) = (&mut rows.paint, kept.paint);
            if !object {
                (r.opacity, r.blend) = (k.opacity, k.blend);
            }
            if !fill {
                (r.fill, r.fill_opacity, r.fill_blend, r.fill_unit_box) = (k.fill, k.fill_opacity, k.fill_blend, k.fill_unit_box);
            }
            if !stroke {
                (r.stroke, r.stroke_width, r.stroke_opacity, r.stroke_blend, r.stroke_unit_box) =
                    (k.stroke, k.stroke_width, k.stroke_opacity, k.stroke_blend, k.stroke_unit_box);
            }
            rows
        }
    }
}

/// What lines of kind `line` keep (`slot`) after line `n` shows its rows ([`label_rows`]), of which `keep` says what is
/// taken from the line: `None` when that is how the graph draws them with no kept rows.
fn line_rows_after(slot: Option<&vectorcraft_doc::LabelRows>, line: Line, n: &Node, keep: Keep) -> Option<vectorcraft_doc::LabelRows> {
    let shown = label_rows(n).unwrap_or_default();
    let rows = keep_rows(slot.cloned().unwrap_or_else(|| line.default_rows()), shown, keep);
    (rows != line.default_rows()).then_some(rows)
}

/// Keep the object rows ([`label_rows`]) of the graph lines among `ids` for every line of their kind (axis lines and tick
/// marks, or a radar graph's rings), the part of them `keep` says, even when a line already showed them. Returns whether
/// kept rows changed.
fn store_line_rows(doc: &mut Document, ids: &[NodeId], keep: Keep) -> bool {
    let mut changed = false;
    for (id, graph, line) in lines_of(doc, ids) {
        let Some(spec) = doc.node(graph).and_then(|n| n.extras().graph.as_deref()) else { continue };
        let Some(n) = doc.node(id) else { continue };
        let kept = line_rows_after(kept_line_rows(&spec.label_styles, line), line, n, keep);
        let Some(spec) = doc.node_mut(graph).and_then(|n| n.extras_if_any().and_then(|e| e.graph.as_deref_mut())) else { continue };
        let slot = line_slot(&mut spec.label_styles, line);
        changed |= *slot != kept;
        *slot = kept;
    }
    changed
}

/// Keep the object rows of graph lines `ids` for every line of their kind, as [`capture_label_rows`] keeps a label's,
/// for an edit that wrote their fill or stroke. Lines of a kind that the edit reached differently take the frontmost
/// one's rows. Called inside the same edit, so they undo together. Returns whether kept rows changed.
pub(crate) fn capture_line_rows(doc: &mut Document, ids: &[NodeId]) -> bool {
    store_line_rows(doc, ids, Keep::All)
}

/// [`capture_line_rows`] for an edit that wrote the fill and stroke rows of graph lines `ids` and left their object
/// opacity and blend mode alone: those stay as the kind keeps them, also when a line still shows other ones until the
/// graph is drawn again.
pub(crate) fn capture_line_fill_stroke_rows(doc: &mut Document, ids: &[NodeId]) -> bool {
    store_line_rows(doc, ids, Keep::Rows)
}

/// Keep, for every line of their kind, the attributes of the object rows ([`label_rows`]) that an edit changed on the
/// graph lines among `ids` (`before`: the document before the edit). The attributes the edit left alone stay as the kind
/// keeps them, also when a line still shows other ones until the graph is drawn again. Lines of a kind that the edit
/// changed differently take the frontmost one's attributes. Called inside the same edit, so they undo together. Returns
/// whether kept rows changed.
pub(crate) fn capture_line_changes(before: &Document, doc: &mut Document, ids: &[NodeId]) -> bool {
    let mut changed = false;
    for (id, graph, line) in lines_of(doc, ids) {
        let old = before.node(id).and_then(label_rows).unwrap_or_default();
        let new = doc.node(id).and_then(label_rows).unwrap_or_default();
        if old == new {
            continue;
        }
        let Some(spec) = doc.node_mut(graph).and_then(|n| n.extras_if_any().and_then(|e| e.graph.as_deref_mut())) else { continue };
        let slot = line_slot(&mut spec.label_styles, line);
        let mut rows = slot.clone().unwrap_or_else(|| line.default_rows());
        merge_row_edits(&mut rows, &old, &new);
        let kept = (rows != line.default_rows()).then_some(rows);
        changed |= *slot != kept;
        *slot = kept;
    }
    changed
}

/// Keep, for every line of their kind, the paint an edit wrote to row `at` of graph line `id` for each `(id, at)` in
/// `written` when that row is the line's topmost visible fill or stroke, also when the line already showed that paint:
/// the paint replaces the one its kind keeps from another line. The other attributes stay as the kind keeps them. Lines
/// of a kind take the frontmost one's paint. Called inside the same edit, so they undo together. Returns whether kept
/// rows changed.
pub(crate) fn capture_line_paint_writes(doc: &mut Document, written: &[(NodeId, usize)]) -> bool {
    let paint = RowWrites { paint: true, ..RowWrites::NONE };
    capture_line_row_writes(doc, written, paint, paint)
}

/// Which attributes of a fill or stroke row an edit wrote, for [`capture_line_row_writes`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct RowWrites {
    /// The paint.
    pub paint: bool,
    /// The weight (a stroke row only).
    pub weight: bool,
    /// The row's opacity.
    pub opacity: bool,
    /// The row's blend mode.
    pub blend: bool,
}

impl RowWrites {
    /// No attribute.
    pub const NONE: RowWrites = RowWrites { paint: false, weight: false, opacity: false, blend: false };
}

/// [`capture_line_paint_writes`] for the attributes `fill` says of a written fill row and `stroke` says of a written
/// stroke row: each is kept for every line of the kind as the line shows it, also when the line already showed it, in
/// place of what its kind keeps from another line.
pub(crate) fn capture_line_row_writes(doc: &mut Document, written: &[(NodeId, usize)], fill: RowWrites, stroke: RowWrites) -> bool {
    let ids: Vec<NodeId> = written.iter().map(|(id, _)| *id).collect();
    let mut changed = false;
    for (id, graph, line) in lines_of(doc, &ids) {
        let Some(n) = doc.node(id) else { continue };
        let items = &n.appearance.items;
        let wrote = |kind: bool| items.iter().rposition(|it| it.is_fill() == kind && it.visible()).is_some_and(|at| written.contains(&(id, at)));
        let on_fill = if wrote(true) { fill } else { RowWrites::NONE };
        let on_stroke = if wrote(false) { stroke } else { RowWrites::NONE };
        if on_fill == RowWrites::NONE && on_stroke == RowWrites::NONE {
            continue;
        }
        let shown = label_rows(n).unwrap_or_default().paint;
        let Some(spec) = doc.node_mut(graph).and_then(|n| n.extras_if_any().and_then(|e| e.graph.as_deref_mut())) else { continue };
        let slot = line_slot(&mut spec.label_styles, line);
        let mut rows = slot.clone().unwrap_or_else(|| line.default_rows());
        let k = &mut rows.paint;
        if on_fill.paint {
            k.fill = shown.fill;
        }
        if on_fill.opacity {
            k.fill_opacity = shown.fill_opacity;
        }
        if on_fill.blend {
            k.fill_blend = shown.fill_blend;
        }
        if on_stroke.paint {
            k.stroke = shown.stroke;
        }
        if on_stroke.weight {
            k.stroke_width = shown.stroke_width;
        }
        if on_stroke.opacity {
            k.stroke_opacity = shown.stroke_opacity;
        }
        if on_stroke.blend {
            k.stroke_blend = shown.stroke_blend;
        }
        let kept = (rows != line.default_rows()).then_some(rows);
        changed |= *slot != kept;
        *slot = kept;
    }
    changed
}

/// Is `id` a graph line: a path directly in a graph's Axes group?
pub(crate) fn is_graph_line(doc: &Document, id: NodeId) -> bool {
    graph_line(doc, id).is_some()
}

/// Set each attribute of `rows` that differs between `old` and `new` as `new` has it.
fn merge_row_edits(rows: &mut vectorcraft_doc::LabelRows, old: &vectorcraft_doc::LabelRows, new: &vectorcraft_doc::LabelRows) {
    fn take<T: PartialEq + Clone>(kept: &mut T, old: &T, new: &T) {
        if old != new {
            *kept = new.clone();
        }
    }
    let (k, o, n) = (&mut rows.paint, &old.paint, &new.paint);
    take(&mut k.fill, &o.fill, &n.fill);
    take(&mut k.stroke, &o.stroke, &n.stroke);
    take(&mut k.stroke_width, &o.stroke_width, &n.stroke_width);
    take(&mut k.fill_opacity, &o.fill_opacity, &n.fill_opacity);
    take(&mut k.stroke_opacity, &o.stroke_opacity, &n.stroke_opacity);
    take(&mut k.fill_blend, &o.fill_blend, &n.fill_blend);
    take(&mut k.stroke_blend, &o.stroke_blend, &n.stroke_blend);
    take(&mut k.opacity, &o.opacity, &n.opacity);
    take(&mut k.blend, &o.blend, &n.blend);
    take(&mut rows.below, &old.below, &new.below);
    take(&mut rows.stroke_under, &old.stroke_under, &new.stroke_under);
}

/// Would [`store_line_rows`] with [`Keep::Object`] change what a graph keeps for the lines among `ids`? Reads the
/// document without copying it.
fn line_object_opacity_differs(doc: &Document, ids: &[NodeId]) -> bool {
    let mut kept: Vec<(NodeId, Line, Option<vectorcraft_doc::LabelRows>)> = vec![];
    for (id, graph, line) in lines_of(doc, ids) {
        let (Some(spec), Some(n)) = (doc.node(graph).and_then(|n| n.extras().graph.as_deref()), doc.node(id)) else { continue };
        let slot = match kept.iter().position(|(g, l, _)| *g == graph && *l == line) {
            Some(i) => kept.swap_remove(i).2,
            None => kept_line_rows(&spec.label_styles, line).cloned(),
        };
        kept.push((graph, line, line_rows_after(slot.as_ref(), line, n, Keep::Object)));
    }
    kept.into_iter().any(|(graph, line, rows)| {
        doc.node(graph).and_then(|n| n.extras().graph.as_deref()).is_some_and(|spec| kept_line_rows(&spec.label_styles, line) != rows.as_ref())
    })
}

/// Keep the object rows ([`label_rows`]) of graph labels `ids` for every label of their kind, as
/// [`capture_label_styles`] keeps a character format, even when a label already showed them: the last label of a kind
/// in the order of `ids` sets what that kind keeps. Objects other than type are left out before the graph's labels are
/// looked up. The graph lines among `ids` keep their rows for every line of their kind too ([`capture_line_rows`]).
/// Called inside the same edit, so they undo together. Returns whether kept rows changed.
pub(crate) fn capture_label_rows(doc: &mut Document, ids: &[NodeId]) -> bool {
    store_label_rows(doc, ids, Keep::All)
}

/// Keep the object opacity and blend mode of graph labels and graph lines `ids` for every label or line of their kind, as
/// [`capture_label_rows`] keeps them, and leave the kept fill and stroke rows as they are. Returns whether what the graph
/// keeps changed.
pub(crate) fn capture_label_object_opacity(doc: &mut Document, ids: &[NodeId]) -> bool {
    store_label_rows(doc, ids, Keep::Object)
}

/// Keep, for every label or line of their kind, the fill row (with `fill`) and the stroke row (with `stroke`) that graph
/// labels and graph lines `ids` show, and with `object` their object opacity and blend mode, as [`capture_label_rows`]
/// keeps them, for an edit that wrote only those: a row of the other kind stays as the kind keeps it, also when a label or
/// line still shows its own one until the graph is drawn again. Returns whether what the graph keeps changed.
pub(crate) fn capture_label_kind_rows(doc: &mut Document, ids: &[NodeId], (fill, stroke): (bool, bool), object: bool) -> bool {
    store_label_rows(doc, ids, Keep::Kinds { fill, stroke, object })
}

/// [`capture_label_kind_rows`] for the graph lines among `ids` alone.
pub(crate) fn capture_line_kind_rows(doc: &mut Document, ids: &[NodeId], (fill, stroke): (bool, bool), object: bool) -> bool {
    store_line_rows(doc, ids, Keep::Kinds { fill, stroke, object })
}

/// Would [`capture_label_object_opacity`] change what a graph keeps for labels `ids`? Reads the document without copying
/// it.
pub(crate) fn label_object_opacity_differs(doc: &Document, ids: &[NodeId]) -> bool {
    line_object_opacity_differs(doc, ids)
        || label_rows_of(doc, ids).into_iter().any(|(graph, role, rows)| {
            let Some(spec) = doc.node(graph).and_then(|n| n.extras().graph.as_deref()) else { return false };
            let slot = match role {
                Role::Value => &spec.label_styles.values_rows,
                Role::Category => &spec.label_styles.categories_rows,
                Role::Legend => &spec.label_styles.legend_rows,
            };
            *slot != kept_label_rows(slot, rows, Keep::Object)
        })
}

/// [`capture_label_rows`], [`capture_label_object_opacity`] or [`capture_label_kind_rows`], as `keep` says.
fn store_label_rows(doc: &mut Document, ids: &[NodeId], keep: Keep) -> bool {
    let mut changed = store_line_rows(doc, ids, keep);
    for (graph, role, rows) in label_rows_of(doc, ids) {
        let Some(spec) = doc.node_mut(graph).and_then(|n| n.extras_if_any().and_then(|e| e.graph.as_deref_mut())) else { continue };
        let slot = match role {
            Role::Value => &mut spec.label_styles.values_rows,
            Role::Category => &mut spec.label_styles.categories_rows,
            Role::Legend => &mut spec.label_styles.legend_rows,
        };
        let kept = kept_label_rows(slot, rows, keep);
        changed |= *slot != kept;
        *slot = kept;
    }
    changed
}

/// What a kind keeps (`slot`) after a label of it shows `rows`, of which `keep` says what is taken from the label.
fn kept_label_rows(
    slot: &Option<vectorcraft_doc::LabelRows>,
    rows: Option<vectorcraft_doc::LabelRows>,
    keep: Keep,
) -> Option<vectorcraft_doc::LabelRows> {
    if keep == Keep::All {
        return rows;
    }
    let kept = keep_rows(slot.clone().unwrap_or_default(), rows.unwrap_or_default(), keep);
    (kept != vectorcraft_doc::LabelRows::default()).then_some(kept)
}

/// The graph, kind and rows ([`label_rows`]) of the last label of each kind among `ids`.
fn label_rows_of(doc: &Document, ids: &[NodeId]) -> Vec<(NodeId, Role, Option<vectorcraft_doc::LabelRows>)> {
    // Graph labels are type: without type among `ids`, the graph's labels are not looked up.
    if !ids.iter().any(|id| doc.node(*id).is_some_and(|n| matches!(n.kind, NodeKind::Text(_)))) {
        return vec![];
    }
    let labels = graph_labels(doc);
    let mut picked: Vec<(NodeId, Role, Option<vectorcraft_doc::LabelRows>)> = vec![];
    for (&(g, role), n) in ids.iter().rev().filter_map(|id| Some((labels.get(id)?, doc.node(*id)?))) {
        if !picked.iter().any(|(pg, pr, _)| *pg == g && *pr == role) {
            picked.push((g, role, label_rows(n)));
        }
    }
    picked
}

/// Keep what an edit gave all of the text of graph labels `ids` for every label of their kind: `change`, when given (the
/// character attributes the edit applied to label `id`), on the kind's kept style, as [`capture_label_styles`] keeps a
/// character format, and `para`, when given, as the kind's paragraph attributes ([`capture_label_paragraphs`]).
/// Attributes the edit did not apply stay as the kind keeps them. Returns whether a kept style or set of attributes
/// changed.
pub(crate) fn capture_label_text(
    doc: &mut Document,
    ids: &[NodeId],
    change: Option<LabelTextChange>,
    para: Option<&vectorcraft_doc::ParaStyle>,
) -> Result<bool> {
    if (change.is_none() && para.is_none()) || !ids.iter().any(|id| doc.node(*id).is_some_and(|n| matches!(n.kind, NodeKind::Text(_)))) {
        return Ok(false);
    }
    // The last label of each kind sets what the kind keeps.
    let labels = graph_labels(doc);
    let mut last: Vec<((NodeId, Role), NodeId)> = vec![];
    for id in ids.iter().rev() {
        if let Some(kind) = labels.get(id).filter(|k| !last.iter().any(|(l, _)| l == *k)) {
            last.push((*kind, *id));
        }
    }
    let mut changed = false;
    for id in last.iter().rev().map(|(_, id)| id) {
        if let Some(change) = change {
            changed |= capture_label_styles(doc, &[*id], &|st| change(*id, st));
        }
        if let Some(para) = para {
            changed |= capture_label_paragraphs(doc, &[*id], &|p| {
                *p = para.clone();
                Ok(())
            })?;
        }
    }
    Ok(changed)
}

/// The character fill and stroke attributes a kind of graph label keeps when a recolor, or a stroke or gradient edit
/// aimed at the characters, changes them on its labels ([`capture_label_changes`]). Each entry copies one attribute
/// from the first style to the second and returns whether they differed.
const RUN_PAINT: [fn(&CharStyle, &mut CharStyle) -> bool; 8] = [
    |a, b| keep(&a.fill, &mut b.fill),
    |a, b| keep(&a.stroke, &mut b.stroke),
    |a, b| keep(&a.stroke_width, &mut b.stroke_width),
    |a, b| keep(&a.stroke_cap, &mut b.stroke_cap),
    |a, b| keep(&a.stroke_join, &mut b.stroke_join),
    |a, b| keep(&a.stroke_miter_limit, &mut b.stroke_miter_limit),
    |a, b| keep(&a.stroke_dash, &mut b.stroke_dash),
    |a, b| keep(&a.stroke_align, &mut b.stroke_align),
];

/// Copy `from` to `to`. Returns whether they differed.
fn keep<T: Clone + PartialEq>(from: &T, to: &mut T) -> bool {
    let differs = from != to;
    if differs {
        to.clone_from(from);
    }
    differs
}

/// The [`RUN_PAINT`] attributes an edit changed on the characters of graph label `now` (`was`: the label before the
/// edit), as the label's style and the indexes of the changed entries. An attribute counts only when every character
/// of the label shows it alike, and the fill only when `fill` is set. A placed gradient is kept without its placement,
/// so it fits each label it lands on. The stroke weight and dashes are kept as the label shows them: the graph draws its
/// labels again without the label's own scale.
fn run_paint_change(was: &Node, now: &Node, fill: bool) -> Option<(CharStyle, Vec<usize>)> {
    let (NodeKind::Text(was), NodeKind::Text(now)) = (&was.kind, &now.kind) else { return None };
    let first_now = &now.runs.first()?.style;
    // Changed: some character showed something else before the edit, whichever run it is in.
    let changed: Vec<usize> = RUN_PAINT
        .iter()
        .enumerate()
        .filter(|(i, copy)| {
            (fill || *i != 0)
                && now.runs.iter().all(|r| !copy(first_now, &mut r.style.clone()))
                && was.runs.iter().any(|r| copy(first_now, &mut r.style.clone()))
        })
        .map(|(i, _)| i)
        .collect();
    if changed.is_empty() {
        return None;
    }
    let unplaced = crate::cmd::gradient::unplaced;
    let mut style = first_now.clone();
    (style.fill, style.stroke) = (unplaced(&style.fill), unplaced(&style.stroke));
    // Character strokes are drawn in text space, scaled by the type's transform.
    if let Some(k) = vectorcraft_doc::Scaling::factor(now.xf).filter(|k| k.is_finite() && *k > 1e-12) {
        let mut st = style.stroke_layer();
        crate::inspect::scale_layer(&mut st, k);
        style.set_stroke_layer(&st);
    }
    Some((style, changed))
}

/// Does graph `g` pick the fill of its legend labels itself, white on a dark wedge and black on the others: a pie with
/// Legends in Wedges whose legend labels keep no fill of their own ([`GraphSpec::picks_legend_fill`])?
pub(super) fn picks_legend_fill(g: &GraphSpec) -> bool {
    g.picks_legend_fill()
}

/// The graph labels and graph lines among `roots` and inside them, for an edit that reaches them without naming them.
pub(crate) fn labels_in(doc: &Document, roots: &[NodeId]) -> Vec<NodeId> {
    let mut texts = Vec::new();
    let mut lines = Vec::new();
    for id in roots {
        // A root that is a line, or the Axes group: found through its parents.
        if graph_line(doc, *id).is_some() {
            lines.push(*id);
        }
        let Some(root) = doc.node(*id) else { continue };
        if let Some(parent) = doc.parent_of(*id).and_then(|p| doc.node(p))
            && let Some(spec) = parent.extras().graph.as_deref()
        {
            lines.extend(root.children().into_iter().flatten().filter(|c| line_in(spec, root, c).is_some()).map(|c| c.id));
        }
        root.walk(&mut |n| {
            if matches!(n.kind, NodeKind::Text(_)) {
                texts.push(n.id);
            }
            if let Some(spec) = n.extras().graph.as_deref() {
                for axes in n.children().into_iter().flatten() {
                    lines.extend(axes.children().into_iter().flatten().filter(|c| line_in(spec, axes, c).is_some()).map(|c| c.id));
                }
            }
        });
    }
    // Graph labels are type: without type among `roots`, the graph's labels are not looked up.
    if !texts.is_empty() {
        let labels = graph_labels(doc);
        texts.retain(|id| labels.contains_key(id));
    }
    texts.extend(lines.into_iter().filter(|id| doc.node(*id).is_some_and(|n| matches!(n.kind, NodeKind::Path { .. }))));
    // Nested roots (a graph and one of its labels) reach a label twice.
    texts.sort_unstable();
    texts.dedup();
    texts
}

/// Keep for every label of their kind what an edit changed on graph labels `ids` (`before`: the document before the
/// edit): with `rows`, the object rows ([`capture_label_rows`]) of the labels whose rows changed, and the character fill
/// and stroke attributes ([`RUN_PAINT`]) that changed on all of a label's characters, on the kind's kept style. The
/// frontmost label of a kind sets what that kind keeps, whatever order `ids` are in. On a graph that colors its legend
/// labels itself ([`picks_legend_fill`]), their fill is kept only when the legend labels among `ids` end up with one
/// fill and the edit named them: with `named`, only a legend label among `named` (an edit that reached the others
/// through the graph or a group leaves them the graph's black or white); without it, every label in `ids`. The graph
/// lines among `ids` whose rows changed keep what changed for every line of their kind ([`capture_line_changes`]), with or without
/// `rows`. Called inside the same edit, so they undo together. Returns whether what a graph keeps changed.
pub(crate) fn capture_label_changes(before: &Document, doc: &mut Document, ids: &[NodeId], rows: bool, named: Option<&[NodeId]>) -> bool {
    let lines = capture_line_changes(before, doc, ids);
    if !ids.iter().any(|id| doc.node(*id).is_some_and(|n| matches!(n.kind, NodeKind::Text(_)))) {
        return lines;
    }
    let labels = graph_labels(doc);
    let mut picked: Vec<NodeId> = ids.iter().copied().filter(|id| labels.contains_key(id)).collect();
    // A later label's capture replaces an earlier one's: take them in paint order, each once.
    picked.sort_by_cached_key(|id| doc.index_path(*id));
    picked.dedup();
    let picks_fill = |graph: NodeId| doc.node(graph).and_then(|n| n.extras().graph.as_deref()).is_some_and(picks_legend_fill);
    // On a pie that colors its legend labels black or white by their wedge, a recolor of the whole pie flips them both
    // ways: their fill is kept only when every legend label the edit reached ends up with the same one.
    let mut wedge_fills: HashMap<NodeId, Vec<Paint>> = HashMap::new();
    for id in &picked {
        let (Some(&(graph, Role::Legend)), Some(NodeKind::Text(t))) = (labels.get(id), doc.node(*id).map(|n| &n.kind)) else { continue };
        if picks_fill(graph) {
            wedge_fills.entry(graph).or_default().extend(t.runs.iter().map(|r| r.style.fill.clone()));
        }
    }
    let one_fill = |graph: NodeId| wedge_fills.get(&graph).is_none_or(|f| f.windows(2).all(|w| w[0] == w[1]));
    let mut changed_rows = Vec::new();
    let mut characters = Vec::new();
    for (id, kind) in picked.iter().filter_map(|id| Some((*id, *labels.get(id)?))) {
        let (Some(was), Some(now)) = (before.node(id), doc.node(id)) else { continue };
        if rows && label_rows(was) != label_rows(now) {
            changed_rows.push(id);
        }
        let aimed = !wedge_fills.contains_key(&kind.0) || named.is_none_or(|n| n.contains(&id));
        let fill = kind.1 != Role::Legend || (one_fill(kind.0) && aimed);
        characters.extend(run_paint_change(was, now, fill).map(|change| (kind, change)));
    }
    let mut changed = capture_label_rows(doc, &changed_rows) | lines;
    for ((graph, role), (style, entries)) in &characters {
        changed |= store_label_style(
            doc,
            *graph,
            *role,
            &|st| {
                for i in entries {
                    if let Some(copy) = RUN_PAINT.get(*i) {
                        copy(style, st);
                    }
                }
            },
            true,
        );
    }
    changed
}

/// The graphs and kinds of label among `ids`, each once.
fn label_kinds(doc: &Document, ids: &[NodeId]) -> Vec<(NodeId, Role)> {
    if ids.is_empty() {
        return vec![];
    }
    let labels = graph_labels(doc);
    let mut kinds: Vec<(NodeId, Role)> = ids.iter().filter_map(|id| labels.get(id).copied()).collect();
    kinds.sort_by_key(|(g, r)| (g.0, *r as u8));
    kinds.dedup();
    kinds
}

/// The series a command on graph `id` (of `count` series) applies to: `seriesIndexes`, else, with no `id` param, a
/// selection made only of this graph's series (Group Selection). Empty: the whole graph.
pub(super) fn picked_series(s: &Session, p: &Value, id: NodeId, count: usize, cmd: &str) -> Result<Vec<usize>> {
    Ok(match p.get("seriesIndexes") {
        Some(v) => {
            let a = v.as_array().filter(|a| !a.is_empty()).ok_or_else(|| bad(cmd, "`seriesIndexes` is a non-empty list of series indexes"))?;
            a.iter()
                .map(|i| i.as_u64().and_then(|i| usize::try_from(i).ok()).filter(|i| *i < count).ok_or_else(|| bad(cmd, format!("no series {i}"))))
                .collect::<Result<_>>()?
        }
        None if p.get("id").is_none() => {
            let st = s.doc()?;
            let members = series_members(&st.doc);
            let picked: Option<Vec<usize>> = st.selection.objects.iter().map(|n| members.get(n).filter(|r| r.graph == id).map(|r| r.index)).collect();
            let mut out = picked.unwrap_or_default();
            out.sort_unstable();
            out.dedup();
            out
        }
        None => vec![],
    })
}

/// Select the series groups `indexes` of graph `id` (they are new nodes after a regeneration).
pub(super) fn reselect_series(d: &Document, sel: &mut vectorcraft_doc::Selection, id: NodeId, indexes: &[u32]) {
    let groups: Vec<NodeId> = d
        .node(id)
        .and_then(Node::children)
        .into_iter()
        .flatten()
        .filter(|c| c.extras().series_index.is_some_and(|i| indexes.contains(&i)))
        .map(|c| c.id)
        .collect();
    sel.set(groups);
}
