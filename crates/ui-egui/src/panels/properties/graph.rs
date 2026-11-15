//! The Properties panel's Graph section, for one selected graph or series of one graph picked with
//! Group Selection: Graph Type… and Graph Data… open the dialogs of Object › Graph › Type… and
//! Data… ([Format pie graphs](https://helpx.adobe.com/illustrator/desktop/automate-visualize-data/visualize-data/format-pie-graphs.html)
//! opens Graph Type from the Properties panel). Column… shows when every series it would change
//! draws columns or bars, and Marker… when every one draws lines, scatter points or radar lines;
//! both run the same route as their Object › Graph items.

use egui::Ui;
use serde_json::json;
use vectorcraft_doc::{Document, GraphKind, MAX_GRAPH_SERIES, NodeId};

use crate::VectorcraftApp;
use crate::widgets::{self, divider, section_header};

/// The Graph section's optional buttons for the selection.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Shown {
    /// Column… takes the series it would change.
    pub(crate) column: bool,
    /// Marker… takes the series it would change.
    pub(crate) marker: bool,
}

/// What the Graph section shows for the selection; `None` unless everything selected is one graph
/// or lies inside the same graph.
pub(crate) fn shown(app: &VectorcraftApp) -> Option<Shown> {
    let st = app.session.active()?;
    let doc = &st.doc;
    let mut graph = None;
    for id in &st.selection.objects {
        let g = graph_of(doc, *id)?;
        if graph.is_some_and(|x| x != g) {
            return None;
        }
        graph = Some(g);
    }
    let graph = graph?;
    let spec = doc.node(graph)?.extras().graph.as_deref()?;
    let count = spec.rows.iter().map(Vec::len).max().unwrap_or(0).min(MAX_GRAPH_SERIES);
    // The series Column… and Marker… change: those picked with Group Selection when the selection is
    // only series, else every series (as `graph.column` and `graph.marker` pick them).
    let picked: Option<Vec<usize>> = st.selection.objects.iter().map(|id| series_of(doc, graph, *id).filter(|i| *i < count)).collect();
    let series = picked.unwrap_or_else(|| (0..count).collect());
    let kinds: Vec<GraphKind> = series.iter().map(|i| spec.series_kind(*i)).collect();
    let all = |ks: &[GraphKind]| !kinds.is_empty() && kinds.iter().all(|k| ks.contains(k));
    Some(Shown {
        column: all(&[GraphKind::Column, GraphKind::StackedColumn, GraphKind::Bar, GraphKind::StackedBar]),
        marker: all(&[GraphKind::Line, GraphKind::Scatter, GraphKind::Radar]),
    })
}

/// The graph `id` is or lies in.
fn graph_of(doc: &Document, id: NodeId) -> Option<NodeId> {
    let mut cur = Some(id);
    while let Some(c) = cur {
        if doc.node(c).is_some_and(|n| n.extras().graph.is_some()) {
            return Some(c);
        }
        cur = doc.parent_of(c);
    }
    None
}

/// The index of the series of `graph` that `id` is or lies in; `None` for the graph itself, its
/// axes and its legend.
fn series_of(doc: &Document, graph: NodeId, id: NodeId) -> Option<usize> {
    let mut cur = id;
    loop {
        let parent = doc.parent_of(cur)?;
        if parent == graph {
            return doc.node(cur)?.extras().series_index.and_then(|i| usize::try_from(i).ok());
        }
        cur = parent;
    }
}

/// The Graph section for the selection, with the divider after it; nothing without a graph.
pub(crate) fn section(app: &mut VectorcraftApp, ui: &mut Ui) {
    let Some(s) = shown(app) else { return };
    section_header(ui, tl!("Graph"));
    let mut buttons = vec![(tl!("Graph Type…"), "graph.setType"), (tl!("Graph Data…"), "graph.setData")];
    if s.column {
        buttons.push((tl!("Column…"), "graph.column"));
    }
    if s.marker {
        buttons.push((tl!("Marker…"), "graph.marker"));
    }
    let full = ui.available_width();
    let w = (full - 6.0) / 2.0;
    for pair in buttons.chunks(2) {
        // Two buttons share a row while both labels fit in half of it; a longer translated label
        // puts each button of the pair on a row of its own.
        if pair.iter().all(|(label, _)| widgets::flat_button_fit_width(ui, label, 0.0) <= w) {
            ui.horizontal(|ui| {
                for (label, id) in pair {
                    if widgets::flat_button(ui, label, w).clicked() {
                        crate::menus::invoke(app, id, json!({}));
                    }
                }
            });
        } else {
            for (label, id) in pair {
                if widgets::flat_button_fit(ui, label, full).clicked() {
                    crate::menus::invoke(app, id, json!({}));
                }
            }
        }
    }
    divider(ui);
}
