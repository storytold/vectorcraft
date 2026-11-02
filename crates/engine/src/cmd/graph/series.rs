use super::labels::{Role, label_style};
use super::{CharStyle, Color, Document, GraphKind, GraphSpec, HashMap, Node, NodeId, NodeKind, Paint, Result, Session, Value, bad};

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

/// Copy the paint just applied to generated series art into that series' fill or stroke. Called
/// inside the same document edit, so the visible paint and future regeneration undo together.
/// Gradients, patterns and swatch links are kept as [`Paint`], not reduced to a solid colour.
/// Returns whether a series' captured fill or stroke changed.
pub(crate) fn capture_series_paints(doc: &mut Document, ids: &[NodeId], fill: bool) -> bool {
    let members = series_members(doc);
    if members.is_empty() {
        return false;
    }
    let captures: Vec<_> = ids
        .iter()
        .filter_map(|id| {
            // Only a series' own marks set its paint, not the art of a marker design.
            let r = members.get(id).filter(|r| doc.parent_of(*id) == Some(r.group))?;
            let node = doc.node(*id)?;
            let paint = node.appearance.paint_at(None, fill)?.clone();
            let width = (!fill).then(|| node.appearance.stroke_width());
            Some((r.graph, r.index, paint, width))
        })
        .collect();
    let mut changed = false;
    for (graph, index, paint, width) in captures {
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

/// Keep a formatting change made to graph labels `ids` (Character or Appearance, on the whole graph or on labels
/// picked with Group Selection) for every label of their kind: `change` is applied to the kind's kept style (or
/// the default one), so the labels look the same when the graph is drawn again. Called inside the same edit, so
/// they undo together. Returns whether a kept style changed.
pub(crate) fn capture_label_styles(doc: &mut Document, ids: &[NodeId], change: &dyn Fn(&mut CharStyle)) -> bool {
    if ids.is_empty() {
        return false;
    }
    let labels = graph_labels(doc);
    if labels.is_empty() {
        return false;
    }
    let mut kinds: Vec<(NodeId, Role)> = ids.iter().filter_map(|id| labels.get(id).copied()).collect();
    kinds.sort_by_key(|(g, r)| (g.0, *r as u8));
    kinds.dedup();
    let mut changed = false;
    for (graph, role) in kinds {
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
