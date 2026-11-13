use vectorcraft_color::BlendMode;

use super::axis::radar_axis;
use super::labels::{Role, filled_lines_on, label_para, label_style};
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

/// Every node in a graph's series groups (the groups too), found in one pass over the document.
/// Axes, the legend and groups whose index is past the graph's series (the index comes from the
/// file) are left out.
fn series_members(doc: &Document) -> HashMap<NodeId, SeriesRef> {
    let mut out = HashMap::new();
    doc.walk(|n| {
        let Some(spec) = n.extras().graph.as_deref() else { return };
        let count = spec.rows.iter().map(Vec::len).max().unwrap_or(0).clamp(1, vectorcraft_doc::MAX_GRAPH_SERIES);
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

/// Store each capture (graph, series index, fill or stroke, paint, stroke weight) in its series. Returns whether a
/// series' captured fill or stroke changed.
fn store_series_paints(doc: &mut Document, captures: Vec<(NodeId, usize, bool, Paint, Option<f64>)>) -> bool {
    let mut changed = false;
    for (graph, index, fill, paint, width) in captures {
        let Some(spec) = doc.node_mut(graph).and_then(|n| n.extras_if_any().and_then(|e| e.graph.as_deref_mut())) else { continue };
        let was = spec.series_paints.clone();
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
        changed |= spec.series_paints != was;
    }
    changed
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
    let mut changed = false;
    for (graph, index, fill, kept, blend) in captures {
        let Some(spec) = doc.node_mut(graph).and_then(|n| n.extras_if_any().and_then(|e| e.graph.as_deref_mut())) else { continue };
        // A fully opaque Normal row on a series the graph keeps nothing for leaves the graph as it is.
        if kept.is_none() && blend.is_none() && spec.series_paints.len() <= index {
            continue;
        }
        let was = spec.series_paints.clone();
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
        changed |= spec.series_paints != was;
    }
    changed
}

/// Copy the object opacity and blend mode of generated series art just edited (`edited`) into that series' object opacity
/// and blend mode. Called inside the same document edit, so both undo together. Returns whether a series' kept object
/// opacity or blend mode changed.
pub(crate) fn capture_series_object_opacity(doc: &mut Document, edited: &[NodeId]) -> bool {
    let mut changed = false;
    for (graph, index, opacity, blend) in object_opacity_captures(doc, edited) {
        let Some(spec) = doc.node_mut(graph).and_then(|n| n.extras_if_any().and_then(|e| e.graph.as_deref_mut())) else { continue };
        // A fully opaque Normal object on a series the graph keeps nothing for leaves the graph as it is.
        if opacity.is_none() && blend.is_none() && spec.series_paints.len() <= index {
            continue;
        }
        let was = spec.series_paints.clone();
        // `index` is below the graph's series count (`series_members`).
        if spec.series_paints.len() <= index {
            spec.series_paints.resize(index + 1, vectorcraft_doc::SeriesPaint::default());
        }
        let Some(slot) = spec.series_paints.get_mut(index) else { continue };
        (slot.opacity, slot.blend) = (opacity, blend);
        changed |= spec.series_paints != was;
    }
    changed
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
        let Some(spec) = doc.node_mut(graph).and_then(|n| n.extras_if_any().and_then(|e| e.graph.as_deref_mut())) else { continue };
        let mut style = label_style(&spec.label_styles, role);
        change(&mut style);
        let slot = match role {
            Role::Value => &mut spec.label_styles.values,
            Role::Category => &mut spec.label_styles.categories,
            Role::Legend => &mut spec.label_styles.legend,
        };
        changed |= slot.as_ref() != Some(&style);
        *slot = Some(style);
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
