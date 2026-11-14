use vectorcraft_color::BlendMode;

use super::axis::radar_axis;
use super::labels::{Line, Role, filled_lines_on, kept_line_rows, label_para, label_style, line_slot};
use super::{Arc, CharStyle, Color, Document, GraphKind, GraphSpec, HashMap, Node, NodeId, NodeKind, Paint, Result, Session, Value, bad};

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
        // The series' own marks: the art of a marker design inside it keeps the design's paint (painting it would
        // last only until the graph is drawn again), so edit the design to change it.
        match members.get(id) {
            Some(r) if r.group == *id => {
                if let Some(group) = doc.node(*id) {
                    out.extend(group.children().into_iter().flatten().filter(|m| matches!(m.kind, NodeKind::Path { .. })).map(|m| m.id));
                }
            }
            Some(r) if doc.parent_of(*id) != Some(r.group) => {}
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
    // A series group is a child of a graph with one of the graph's series indexes ([`series_members`]). Each id is
    // looked up through its parent, with no pass over the document: the panels call this while they draw.
    let is_series_group = |n: &Node| {
        let Some(index) = n.extras().series_index.and_then(|i| usize::try_from(i).ok()) else { return false };
        let graph = doc.parent_of(n.id).and_then(|parent| doc.node(parent)).and_then(|parent| parent.extras().graph.as_deref());
        graph.is_some_and(|spec| index < series_count(spec))
    };
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for id in ids {
        match doc.node(*id) {
            Some(group) if is_series_group(group) && stands(group) => {
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
        captures.push((r.graph, r.index, *fill, paint, (!*fill).then_some(width)));
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

/// Store each capture (graph, series index, fill or stroke, paint, stroke weight) in its series. Returns whether a
/// series' captured fill or stroke differs from what the graph stored before the first capture.
fn store_series_paints(doc: &mut Document, captures: Vec<(NodeId, usize, bool, Paint, Option<f64>)>) -> bool {
    let mut before = SeriesPaintsBefore::new();
    for (graph, index, fill, paint, width) in captures {
        let Some(spec) = doc.node_mut(graph).and_then(|n| n.extras_if_any().and_then(|e| e.graph.as_deref_mut())) else { continue };
        before.entry(graph).or_insert_with(|| spec.series_paints.clone());
        // `index` is below the graph's series count (`series_members`).
        if spec.series_paints.len() <= index {
            spec.series_paints.resize(index + 1, vectorcraft_doc::SeriesPaint::default());
        }
        let Some(slot) = spec.series_paints.get_mut(index) else { continue };
        if fill {
            slot.fill = Some(paint);
        } else {
            slot.stroke = Some(paint);
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

/// Copy the object opacity and blend mode of generated series art just edited (`edited`) into that series' object opacity
/// and blend mode. Called inside the same document edit, so both undo together. Returns whether a series' kept object
/// opacity or blend mode changed.
pub(crate) fn capture_series_object_opacity(doc: &mut Document, edited: &[NodeId]) -> bool {
    let mut before = SeriesPaintsBefore::new();
    for (graph, index, opacity, blend) in object_opacity_captures(doc, edited) {
        let Some(spec) = doc.node_mut(graph).and_then(|n| n.extras_if_any().and_then(|e| e.graph.as_deref_mut())) else { continue };
        // A fully opaque Normal object on a series the graph keeps nothing for leaves the graph as it is.
        if opacity.is_none() && blend.is_none() && spec.series_paints.len() <= index {
            continue;
        }
        before.entry(graph).or_insert_with(|| spec.series_paints.clone());
        // `index` is below the graph's series count (`series_members`).
        if spec.series_paints.len() <= index {
            spec.series_paints.resize(index + 1, vectorcraft_doc::SeriesPaint::default());
        }
        let Some(slot) = spec.series_paints.get_mut(index) else { continue };
        (slot.opacity, slot.blend) = (opacity, blend);
    }
    settle_series_paints(doc, before)
}

/// Would [`capture_series_object_opacity`] change what the graph stores for `edited`? Reads the document without copying
/// it.
pub(crate) fn series_object_opacity_differs(doc: &Document, edited: &[NodeId]) -> bool {
    let mut after: HashMap<(NodeId, usize), (Option<f32>, Option<BlendMode>)> = HashMap::new();
    let stored = |graph: NodeId, index: usize| {
        let spec = doc.node(graph).and_then(|n| n.extras().graph.as_deref());
        spec.and_then(|g| g.series_paints.get(index)).map_or((None, None), |slot| (slot.opacity, slot.blend))
    };
    for (graph, index, opacity, blend) in object_opacity_captures(doc, edited) {
        after.insert((graph, index), (opacity, blend));
    }
    after.into_iter().any(|((graph, index), pair)| stored(graph, index) != pair)
}

/// The series and object opacity and blend mode (None for 100% and Normal) of each of `edited` that is a path directly in a
/// series group, in order.
fn object_opacity_captures(doc: &Document, edited: &[NodeId]) -> Vec<(NodeId, usize, Option<f32>, Option<BlendMode>)> {
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
            // Only the paths the graph draws again, not the copy of a graph design.
            let r = members.get(id).filter(|r| doc.parent_of(*id) == Some(r.group))?;
            let node = doc.node(*id).filter(|n| matches!(n.kind, NodeKind::Path { .. }))?;
            Some((r.graph, r.index, (node.opacity < 1.0).then_some(node.opacity), (node.blend != BlendMode::Normal).then_some(node.blend)))
        })
        .collect()
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
        for c in n.children().into_iter().flatten() {
            match (&c.kind, c.name.as_deref(), c.extras().series_index) {
                (NodeKind::Text(_), _, _) if g.kind == GraphKind::Pie => {
                    out.insert(c.id, (n.id, Role::Category));
                }
                (NodeKind::Group { .. }, Some("Legend"), None) => {
                    for t in c.children().into_iter().flatten().filter(|t| matches!(t.kind, NodeKind::Text(_))) {
                        out.insert(t.id, (n.id, Role::Legend));
                    }
                }
                (NodeKind::Group { .. }, Some("Axes"), None) => {
                    let texts: Vec<NodeId> =
                        c.children().into_iter().flatten().filter(|t| matches!(t.kind, NodeKind::Text(_))).map(|t| t.id).collect();
                    let cats = match g.kind {
                        GraphKind::Scatter => 0,
                        GraphKind::Radar => texts.len(),
                        _ => g.categories.iter().take(g.rows.len().max(1)).filter(|c| !c.is_empty()).count().min(texts.len()),
                    };
                    for (i, t) in texts.iter().enumerate() {
                        out.insert(*t, (n.id, if i + cats >= texts.len() { Role::Category } else { Role::Value }));
                    }
                }
                _ => {}
            }
        }
    });
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
/// they undo together. Returns whether a kept style changed.
pub(crate) fn capture_label_styles(doc: &mut Document, ids: &[NodeId], change: &dyn Fn(&mut CharStyle)) -> bool {
    let mut changed = false;
    for (graph, role) in label_kinds(doc, ids) {
        changed |= store_label_style(doc, graph, role, change);
    }
    changed
}

/// Apply `change` to the style graph `graph` keeps for its labels of kind `role` (or the default one). Returns whether
/// the kept style changed.
fn store_label_style(doc: &mut Document, graph: NodeId, role: Role, change: &dyn Fn(&mut CharStyle)) -> bool {
    let Some(spec) = doc.node_mut(graph).and_then(|n| n.extras_if_any().and_then(|e| e.graph.as_deref_mut())) else { return false };
    let mut style = label_style(&spec.label_styles, role);
    change(&mut style);
    let slot = match role {
        Role::Value => &mut spec.label_styles.values,
        Role::Category => &mut spec.label_styles.categories,
        Role::Legend => &mut spec.label_styles.legend,
    };
    let changed = slot.as_ref() != Some(&style);
    *slot = Some(style);
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

/// What lines of kind `line` keep (`slot`) after line `n` shows its rows ([`label_rows`]), or with `object_only` what
/// `slot` keeps with the line's object opacity and blend mode: `None` when that is how the graph draws them with no
/// kept rows.
fn line_rows_after(slot: Option<&vectorcraft_doc::LabelRows>, line: Line, n: &Node, object_only: bool) -> Option<vectorcraft_doc::LabelRows> {
    let shown = label_rows(n).unwrap_or_default();
    let rows = if object_only {
        let mut kept = slot.cloned().unwrap_or_else(|| line.default_rows());
        (kept.paint.opacity, kept.paint.blend) = (shown.paint.opacity, shown.paint.blend);
        kept
    } else {
        shown
    };
    (rows != line.default_rows()).then_some(rows)
}

/// Keep the object rows ([`label_rows`]) of the graph lines among `ids` for every line of their kind (axis lines and tick
/// marks, or a radar graph's rings), with `object_only` only their object opacity and blend mode, even when a line
/// already showed them. Returns whether kept rows changed.
fn store_line_rows(doc: &mut Document, ids: &[NodeId], object_only: bool) -> bool {
    let mut changed = false;
    for (id, graph, line) in lines_of(doc, ids) {
        let Some(spec) = doc.node(graph).and_then(|n| n.extras().graph.as_deref()) else { continue };
        let Some(n) = doc.node(id) else { continue };
        let kept = line_rows_after(kept_line_rows(&spec.label_styles, line), line, n, object_only);
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
    store_line_rows(doc, ids, false)
}

/// Keep the object rows of the graph lines among `ids` whose rows an edit changed (`before`: the document before the
/// edit) for every line of their kind. Returns whether kept rows changed.
fn capture_line_changes(before: &Document, doc: &mut Document, ids: &[NodeId]) -> bool {
    let changed: Vec<NodeId> = ids
        .iter()
        .copied()
        .filter(|id| graph_line(doc, *id).is_some() && before.node(*id).map(label_rows) != doc.node(*id).map(label_rows))
        .collect();
    store_line_rows(doc, &changed, false)
}

/// Would [`store_line_rows`] with `object_only` change what a graph keeps for the lines among `ids`? Reads the document
/// without copying it.
fn line_object_opacity_differs(doc: &Document, ids: &[NodeId]) -> bool {
    let mut kept: Vec<(NodeId, Line, Option<vectorcraft_doc::LabelRows>)> = vec![];
    for (id, graph, line) in lines_of(doc, ids) {
        let (Some(spec), Some(n)) = (doc.node(graph).and_then(|n| n.extras().graph.as_deref()), doc.node(id)) else { continue };
        let slot = match kept.iter().position(|(g, l, _)| *g == graph && *l == line) {
            Some(i) => kept.swap_remove(i).2,
            None => kept_line_rows(&spec.label_styles, line).cloned(),
        };
        kept.push((graph, line, line_rows_after(slot.as_ref(), line, n, true)));
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
    store_label_rows(doc, ids, false)
}

/// Keep the object opacity and blend mode of graph labels and graph lines `ids` for every label or line of their kind, as
/// [`capture_label_rows`] keeps them, and leave the kept fill and stroke rows as they are. Returns whether what the graph
/// keeps changed.
pub(crate) fn capture_label_object_opacity(doc: &mut Document, ids: &[NodeId]) -> bool {
    store_label_rows(doc, ids, true)
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
            *slot != kept_label_rows(slot, rows, true)
        })
}

/// [`capture_label_rows`], or with `object_only` [`capture_label_object_opacity`].
fn store_label_rows(doc: &mut Document, ids: &[NodeId], object_only: bool) -> bool {
    let mut changed = store_line_rows(doc, ids, object_only);
    for (graph, role, rows) in label_rows_of(doc, ids) {
        let Some(spec) = doc.node_mut(graph).and_then(|n| n.extras_if_any().and_then(|e| e.graph.as_deref_mut())) else { continue };
        let slot = match role {
            Role::Value => &mut spec.label_styles.values_rows,
            Role::Category => &mut spec.label_styles.categories_rows,
            Role::Legend => &mut spec.label_styles.legend_rows,
        };
        let kept = kept_label_rows(slot, rows, object_only);
        changed |= *slot != kept;
        *slot = kept;
    }
    changed
}

/// What a kind keeps after a label of it shows `rows`: those rows, or with `object_only` what `slot` keeps with the
/// label's object opacity and blend mode.
fn kept_label_rows(
    slot: &Option<vectorcraft_doc::LabelRows>,
    rows: Option<vectorcraft_doc::LabelRows>,
    object_only: bool,
) -> Option<vectorcraft_doc::LabelRows> {
    if !object_only {
        return rows;
    }
    let mut kept = slot.clone().unwrap_or_default();
    let shown = rows.unwrap_or_default();
    (kept.paint.opacity, kept.paint.blend) = (shown.paint.opacity, shown.paint.blend);
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
/// Legends in Wedges whose legend labels keep no fill of their own?
pub(super) fn picks_legend_fill(g: &GraphSpec) -> bool {
    g.kind == GraphKind::Pie
        && g.legend
        && g.pie_legend_in_wedges
        && !g.label_styles.legend.as_ref().is_some_and(|st| st.fill != Paint::solid(Color::BLACK))
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
/// fill. The graph lines among `ids` whose rows changed keep them for every line of their kind ([`capture_line_rows`]),
/// with or without `rows`. Called inside the same edit, so they undo together. Returns whether what a graph keeps changed.
pub(crate) fn capture_label_changes(before: &Document, doc: &mut Document, ids: &[NodeId], rows: bool) -> bool {
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
        let fill = kind.1 != Role::Legend || one_fill(kind.0);
        characters.extend(run_paint_change(was, now, fill).map(|change| (kind, change)));
    }
    let mut changed = capture_label_rows(doc, &changed_rows) | lines;
    for ((graph, role), (style, entries)) in &characters {
        changed |= store_label_style(doc, *graph, *role, &|st| {
            for i in entries {
                if let Some(copy) = RUN_PAINT.get(*i) {
                    copy(style, st);
                }
            }
        });
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
