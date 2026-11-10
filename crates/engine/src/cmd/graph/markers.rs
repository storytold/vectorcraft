use super::designs::design_names;
use super::edit::{regenerate, spec_of, target};
use super::series::{picked_series, reselect_series};
use super::{Affine, Arc, Document, GraphSpec, Node, NodeKind, Rect, Result, Session, Value, bad, json};

/// Most objects a graph's marker designs may add (each data point and legend swatch draws a copy of its design).
pub(super) const MARKER_BUDGET: usize = 200_000;

/// Each series' marker design art, when the document has the design.
pub(super) fn marker_designs(d: &Document, g: &GraphSpec) -> Vec<Option<Arc<Node>>> {
    let nser = g.rows.iter().map(Vec::len).max().unwrap_or(0).clamp(1, vectorcraft_doc::MAX_GRAPH_SERIES);
    (0..nser)
        .map(|s| {
            let name = g.series_markers.get(s)?.as_deref()?;
            d.graph_designs.iter().find(|x| x.name == name).map(|x| x.art.clone())
        })
        .collect()
}

/// How many objects the marker designs `designs` add to graph `g`: a copy per data point and legend swatch.
pub(super) fn marker_cost(g: &GraphSpec, designs: &[Option<Arc<Node>>]) -> usize {
    let points = g.rows.len().min(vectorcraft_doc::MAX_GRAPH_CATEGORIES) + 1;
    designs.iter().flatten().map(|a| a.count().saturating_mul(points)).fold(0, usize::saturating_add)
}

/// The frame graph design `art` is drawn around: its backmost object's bounds (the whole art's when the design is a
/// single object). `None` for art with no area.
pub(super) fn design_frame(art: &Node) -> Option<Rect> {
    let usable = |b: &Rect| b.width() > 1e-9 && b.height() > 1e-9 && b.width().is_finite() && b.height().is_finite();
    let group = matches!(art.kind, NodeKind::Group { .. });
    let backmost = art.children().filter(|_| group).and_then(|c| c.first()).and_then(|c| c.geometric_bounds()).filter(usable);
    backmost.or_else(|| art.geometric_bounds().filter(usable))
}

/// Graph design `art` scaled into `cell` (Object › Graph › Marker): its backmost object, the rectangle a design is
/// drawn around, fills the cell (the whole art does when the design is a single object), strokes scaling with it.
/// Fresh ids. `None` for art with no area.
pub(super) fn fit_design(d: &mut Document, art: &Node, cell: Rect) -> Option<Arc<Node>> {
    let a = fit_affine(design_frame(art)?, cell);
    // Strokes, effects and pattern tiles scale with the art.
    let art = crate::cmd::place::transformed(art.clone(), a);
    Some(Arc::new(d.reid(&art)))
}

/// The map that puts a design's frame `frame` on `cell` ([`fit_design`]).
pub(super) fn fit_affine(frame: Rect, cell: Rect) -> Affine {
    Affine::translate(cell.origin().to_vec2())
        * Affine::scale_non_uniform(cell.width() / frame.width(), cell.height() / frame.height())
        * Affine::translate(-frame.origin().to_vec2())
}

pub(super) fn set_marker(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "graph.marker";
    let id = target(s, p, C)?;
    let mut spec = spec_of(s, id)?;
    let count = spec.rows.iter().map(Vec::len).max().unwrap_or(0).min(vectorcraft_doc::MAX_GRAPH_SERIES);
    let picked = picked_series(s, p, id, count, C)?;
    let names = design_names(&s.doc()?.doc);
    let Some(v) = p.get("design") else {
        // The design the picked series (all of them with none picked) share, or null.
        let all: Vec<usize> = if picked.is_empty() { (0..count).collect() } else { picked };
        // A design deleted since doesn't count: those series draw the default marker.
        let used: Vec<Option<&String>> =
            all.iter().map(|i| spec.series_markers.get(*i).and_then(Option::as_ref).filter(|n| names.contains(n))).collect();
        let shared = used.first().copied().flatten().filter(|d| used.iter().all(|x| *x == Some(*d)));
        return Ok(json!({ "designs": names, "design": shared }));
    };
    let design = match v {
        Value::Null => None,
        Value::String(n) if names.contains(n) => Some(n.clone()),
        Value::String(n) => return Err(bad(C, format!("no design named `{n}`"))),
        _ => return Err(bad(C, "`design` is a design name, or null for the default marker")),
    };
    let reselect: Vec<u32> = if p.get("seriesIndexes").is_none() { picked.iter().filter_map(|i| u32::try_from(*i).ok()).collect() } else { vec![] };
    let targets: Vec<usize> = if picked.is_empty() { (0..count).collect() } else { picked };
    spec.series_markers.truncate(count);
    spec.series_markers.resize(count, None);
    for i in targets {
        if let Some(m) = spec.series_markers.get_mut(i) {
            *m = design.clone();
        }
    }
    while spec.series_markers.last().is_some_and(Option::is_none) {
        spec.series_markers.pop();
    }
    if marker_cost(&spec, &marker_designs(&s.doc()?.doc, &spec)) > MARKER_BUDGET {
        return Err(bad(C, "the design is too big to draw at every data point of this graph"));
    }
    s.edit("Graph Marker", |d, sel| {
        regenerate(d, id, spec)?;
        if !reselect.is_empty() {
            reselect_series(d, sel, id, &reselect);
        }
        Ok(())
    })?;
    Ok(json!({ "id": id.0 }))
}
