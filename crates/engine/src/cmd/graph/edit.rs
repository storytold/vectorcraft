use super::axis::{MAX_TICK_MARKS, count_given, one_line};
use super::data::apply_data;
use super::generate::{children_bounds, generate};
use super::series::{picked_series, reselect_series};
use super::{
    Affine, Document, EngineError, GraphKind, GraphSpec, Node, NodeId, Rect, Result, Session, TickLength, Value, ValueAxisSide, bad, f64_req, json,
    named_id, number_or, switch_or, text_param,
};

/// An axis bound `p` gives for `key`: `None` when left out, `Some(None)` for null (calculated from the data), or the
/// number. Any other value, such as `"10"`, is an error.
fn bound_param(p: &Value, key: &str, cmd: &str) -> Result<Option<Option<f64>>> {
    match p.get(key) {
        None => Ok(None),
        Some(Value::Null) => Ok(Some(None)),
        Some(v) => v.as_f64().map(|x| Some(Some(x))).ok_or_else(|| bad(cmd, format!("`{key}` must be a number or null, not {v}"))),
    }
}

/// Regenerate graph `id` from `spec` (adjusting the plot rect for any move/scale since the last
/// generation, which is recorded in the spec's `placed` bounds).
pub(super) fn regenerate(d: &mut Document, id: NodeId, mut spec: GraphSpec) -> Result<()> {
    let n = d.node(id).ok_or(EngineError::NoNode(id))?;
    if let (Some(placed), Some(now)) = (n.extras().graph.as_ref().and_then(|g| g.placed), n.children().and_then(|c| children_bounds(c)))
        && placed.width() > 1e-9
        && placed.height() > 1e-9
        && (placed.x0 - now.x0).abs() + (placed.y0 - now.y0).abs() + (placed.x1 - now.x1).abs() + (placed.y1 - now.y1).abs() > 1e-6
    {
        let a = Affine::translate(now.origin().to_vec2())
            * Affine::scale_non_uniform(now.width() / placed.width(), now.height() / placed.height())
            * Affine::translate(-placed.origin().to_vec2());
        spec.rect = a.transform_rect_bbox(spec.rect);
    }
    let children = generate(d, &spec);
    spec.placed = children_bounds(&children);
    let n = d.node_mut(id).ok_or(EngineError::NoNode(id))?;
    if let Some(ch) = n.children_mut() {
        *ch = children;
    }
    n.extras_mut().graph = Some(Box::new(spec));
    Ok(())
}

pub(super) fn create(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "graph.create";
    let kind = text_param(p, "type", C)?
        .map(|t| GraphKind::parse(t).ok_or_else(|| bad(C, format!("unknown graph type `{t}`"))))
        .transpose()?
        .unwrap_or_default();
    let (x, y) = (f64_req(p, "x", C)?, f64_req(p, "y", C)?);
    let (w, h) = (number_or(p, "width", 200.0, C)?.abs().max(1.0), number_or(p, "height", 150.0, C)?.abs().max(1.0));
    let mut spec = GraphSpec { kind, rect: Rect::new(x, y, x + w, y + h), ..GraphSpec::default() };
    if !apply_data(&mut spec, p, C)? {
        // Starter data so the new graph shows something (edit it with Object → Graph → Data…).
        spec.series = vec!["Series 1".into(), "Series 2".into()];
        spec.categories = vec!["A".into(), "B".into(), "C".into(), "D".into()];
        spec.rows = vec![vec![3.0, 2.0], vec![5.0, 4.0], vec![4.0, 6.0], vec![7.0, 5.0]];
    }
    let parent = s.doc()?.target_parent()?;
    let id = s.edit("Graph", |d, sel| {
        let gid = d.alloc_id();
        let mut g = Node::group(gid, vec![]);
        g.name = Some(format!("{} Graph", kind.label()));
        d.insert(parent, usize::MAX, g)?;
        regenerate(d, gid, spec)?;
        sel.set([gid]);
        Ok(gid)
    })?;
    Ok(json!({ "id": id.0 }))
}

/// The graph `id` names (or the graph containing it), else the selected graph (or the graph
/// containing the selection). Refused while an object of a dynamic symbol instance's art is
/// selected.
pub(super) fn target(s: &Session, p: &Value, cmd: &str) -> Result<NodeId> {
    let st = s.doc()?;
    let start = match named_id(p)? {
        Some(id) => vec![id],
        None => {
            crate::cmd::edit::refuse_symbol_part(s)?;
            st.selection.objects.clone()
        }
    };
    for id in start {
        let mut cur = Some(id);
        while let Some(c) = cur {
            if st.doc.node(c).is_some_and(|n| n.extras().graph.is_some()) {
                return Ok(c);
            }
            cur = st.doc.parent_of(c);
        }
    }
    Err(bad(cmd, "select a graph"))
}

pub(super) fn spec_of(s: &Session, id: NodeId) -> Result<GraphSpec> {
    s.doc()?.doc.node(id).and_then(|n| n.extras().graph.as_deref().cloned()).ok_or(EngineError::NoNode(id))
}

fn query_type_options(spec: &GraphSpec, count: usize, picked: &[usize]) -> Value {
    // With series picked, the type and value axis they share, so the dialog's OK keeps them.
    let kinds: Vec<GraphKind> = picked.iter().map(|i| spec.series_kind(*i)).collect();
    let kind = kinds.first().copied().filter(|k| kinds.iter().all(|x| x == k)).unwrap_or(spec.kind);
    // Series on both axes give no value axis (null), so OK leaves each where it is.
    let sides: Vec<bool> = picked.iter().map(|i| spec.right_series.binary_search(i).is_ok()).collect();
    let value_axis = match sides.first() {
        None => Some(spec.value_axis.id()),
        Some(r) if sides.iter().all(|x| x == r) => Some(if *r { "right" } else { "left" }),
        Some(_) => None,
    };
    let mut fields = json!({
        "type": kind.id(), "columnWidth": spec.column_width, "clusterWidth": spec.cluster_width, "legend": spec.legend,
        "markPoints": spec.mark_points, "connectPoints": spec.connect_points, "edgeToEdge": spec.edge_to_edge, "ticks": spec.ticks,
        "axisMin": spec.axis_min, "axisMax": spec.axis_max, "valueAxis": value_axis, "separateScales": spec.separate_scales,
        "rightTicks": spec.right_ticks, "rightAxisMin": spec.right_axis_min, "rightAxisMax": spec.right_axis_max,
        "tickLength": spec.tick_length.id(), "tickMarks": spec.tick_marks.clamp(1, MAX_TICK_MARKS), "rightTickLength": spec.right_tick_length.id(),
        "rightTickMarks": spec.right_tick_marks.clamp(1, MAX_TICK_MARKS), "categoryTickLength": spec.category_tick_length.id(),
        "categoryTickMarks": spec.category_tick_marks.clamp(1, MAX_TICK_MARKS), "ticksBetweenLabels": spec.ticks_between_labels,
        "prefix": one_line(&spec.prefix), "suffix": one_line(&spec.suffix), "rightPrefix": one_line(&spec.right_prefix),
        "rightSuffix": one_line(&spec.right_suffix), "dropShadow": spec.drop_shadow, "legendAcrossTop": spec.legend_across_top,
        "firstRowInFront": spec.first_row_in_front, "firstColumnInFront": spec.first_column_in_front, "filledLines": spec.filled_lines,
    });
    // Only the options this graph's types use: overlap order for columns and bars, filled lines for lines.
    let kinds: Vec<GraphKind> = (0..count.max(1)).map(|i| spec.series_kind(i)).collect();
    let has = |ks: &[GraphKind]| kinds.iter().any(|k| ks.contains(k));
    if let Some(o) = fields.as_object_mut() {
        if !has(&[GraphKind::Column, GraphKind::StackedColumn, GraphKind::Bar, GraphKind::StackedBar]) {
            o.remove("firstRowInFront");
            o.remove("firstColumnInFront");
        }
        if !has(&[GraphKind::Line, GraphKind::Scatter, GraphKind::Radar]) {
            o.remove("filledLines");
        }
    }
    // Pie graphs: Legend, Position and Sort (their Legend stands for the legend checkbox).
    if let Some(o) = fields.as_object_mut().filter(|_| spec.kind == GraphKind::Pie) {
        o.remove("legend");
        o.insert(
            "pieLegend".into(),
            json!(if !spec.legend {
                "none"
            } else if spec.pie_legend_in_wedges {
                "wedges"
            } else {
                "standard"
            }),
        );
        o.insert("piePosition".into(), json!(spec.pie_position.id()));
        o.insert("pieSort".into(), json!(spec.pie_sort.id()));
    }
    fields
}

pub(super) fn set_type(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "graph.setType";
    let id = target(s, p, C)?;
    let mut spec = spec_of(s, id)?;
    // The series to retype: `seriesIndexes`, else, with no `id`, a selection made only of this graph's series (Group
    // Selection). Anything else selected, the graph itself for one, is the whole graph.
    let count = spec.rows.iter().map(Vec::len).max().unwrap_or(0).min(vectorcraft_doc::MAX_GRAPH_SERIES);
    // A file's list of right-axis series, kept to series the graph has, at most once each.
    spec.right_series.retain(|i| *i < count);
    spec.right_series.sort_unstable();
    spec.right_series.dedup();
    let (new_type, new_axis) = (text_param(p, "type", C)?, text_param(p, "valueAxis", C)?);
    if p.get("seriesIndexes").is_some() && new_type.is_none() && new_axis.is_none() {
        return Err(bad(C, "`seriesIndexes` needs a `type` or a `valueAxis`"));
    }
    let picked = picked_series(s, p, id, count, C)?;
    let keys = [
        "type",
        "seriesIndexes",
        "columnWidth",
        "clusterWidth",
        "legend",
        "markPoints",
        "connectPoints",
        "edgeToEdge",
        "ticks",
        "axisMin",
        "axisMax",
        "valueAxis",
        "separateScales",
        "rightTicks",
        "rightAxisMin",
        "rightAxisMax",
        "tickLength",
        "tickMarks",
        "rightTickLength",
        "rightTickMarks",
        "categoryTickLength",
        "categoryTickMarks",
        "ticksBetweenLabels",
        "prefix",
        "suffix",
        "rightPrefix",
        "rightSuffix",
        "dropShadow",
        "legendAcrossTop",
        "firstRowInFront",
        "firstColumnInFront",
        "filledLines",
        "pieLegend",
        "piePosition",
        "pieSort",
    ];
    if !keys.iter().any(|k| p.get(*k).is_some()) {
        return Ok(query_type_options(&spec, count, &picked));
    }
    // Series picked with Group Selection stay selected through the regeneration (their groups are new nodes).
    let reselect: Vec<u32> = if p.get("seriesIndexes").is_none() { picked.iter().filter_map(|i| u32::try_from(*i).ok()).collect() } else { vec![] };
    if let Some(t) = new_type {
        let kind = GraphKind::parse(t).ok_or_else(|| bad(C, format!("unknown graph type `{t}`")))?;
        if picked.is_empty() {
            spec.kind = kind;
        } else {
            if !kind.combines_with(spec.kind) {
                return Err(bad(C, format!("a {} series can't go in a {} graph", kind.label(), spec.kind.label())));
            }
            spec.series_kinds.resize(count, None);
            for &i in &picked {
                if let Some(k) = spec.series_kinds.get_mut(i) {
                    *k = (kind != spec.kind).then_some(kind);
                }
            }
            while spec.series_kinds.last().is_some_and(Option::is_none) {
                spec.series_kinds.pop();
            }
        }
    }
    if let Some(v) = new_axis {
        let side = ValueAxisSide::parse(v).ok_or_else(|| bad(C, format!("unknown value axis `{v}` (left, right or both)")))?;
        if picked.is_empty() {
            spec.value_axis = side;
        } else {
            // A series goes on one axis.
            if side == ValueAxisSide::Both {
                return Err(bad(C, "a series goes on the left or the right value axis"));
            }
            spec.right_series.retain(|i| *i < count && !picked.contains(i));
            if side == ValueAxisSide::Right {
                spec.right_series.extend(&picked);
                spec.right_series.sort_unstable();
            }
        }
    }
    spec.separate_scales = switch_or(p, "separateScales", spec.separate_scales, C)?;
    for (key, slot) in [
        ("tickLength", &mut spec.tick_length),
        ("rightTickLength", &mut spec.right_tick_length),
        ("categoryTickLength", &mut spec.category_tick_length),
    ] {
        if let Some(v) = text_param(p, key, C)? {
            *slot = TickLength::parse(v).ok_or_else(|| bad(C, format!("unknown {key} `{v}` (none, short or full)")))?;
        }
    }
    for (key, slot) in
        [("tickMarks", &mut spec.tick_marks), ("rightTickMarks", &mut spec.right_tick_marks), ("categoryTickMarks", &mut spec.category_tick_marks)]
    {
        *slot = count_given(p, key, MAX_TICK_MARKS as u64, C)?.map_or(*slot, |n| n.max(1));
    }
    spec.ticks_between_labels = switch_or(p, "ticksBetweenLabels", spec.ticks_between_labels, C)?;
    if let Some(v) = text_param(p, "piePosition", C)? {
        spec.pie_position =
            vectorcraft_doc::PiePosition::parse(v).ok_or_else(|| bad(C, format!("unknown piePosition `{v}` (even, ratio or stacked)")))?;
    }
    if let Some(v) = text_param(p, "pieSort", C)? {
        spec.pie_sort = vectorcraft_doc::PieSort::parse(v).ok_or_else(|| bad(C, format!("unknown pieSort `{v}` (none, all or first)")))?;
    }
    for (key, slot) in
        [("prefix", &mut spec.prefix), ("suffix", &mut spec.suffix), ("rightPrefix", &mut spec.right_prefix), ("rightSuffix", &mut spec.right_suffix)]
    {
        if let Some(v) = text_param(p, key, C)? {
            // A label, not a paragraph: one line, at most MAX_AFFIX characters.
            *slot = one_line(v);
        }
    }
    spec.right_ticks = count_given(p, "rightTicks", 100, C)?.unwrap_or(spec.right_ticks);
    for (key, slot) in [
        ("rightAxisMin", &mut spec.right_axis_min),
        ("rightAxisMax", &mut spec.right_axis_max),
        ("axisMin", &mut spec.axis_min),
        ("axisMax", &mut spec.axis_max),
    ] {
        if let Some(v) = bound_param(p, key, C)? {
            *slot = v;
        }
    }
    spec.column_width = number_or(p, "columnWidth", spec.column_width, C)?.clamp(1.0, 1000.0);
    spec.cluster_width = number_or(p, "clusterWidth", spec.cluster_width, C)?.clamp(1.0, 1000.0);
    spec.legend = switch_or(p, "legend", spec.legend, C)?;
    spec.drop_shadow = switch_or(p, "dropShadow", spec.drop_shadow, C)?;
    spec.first_row_in_front = switch_or(p, "firstRowInFront", spec.first_row_in_front, C)?;
    spec.first_column_in_front = switch_or(p, "firstColumnInFront", spec.first_column_in_front, C)?;
    spec.filled_lines = switch_or(p, "filledLines", spec.filled_lines, C)?;
    spec.legend_across_top = switch_or(p, "legendAcrossTop", spec.legend_across_top, C)?;
    // After `legend`, which the Graph Type dialog sends too.
    if let Some(v) = text_param(p, "pieLegend", C)? {
        (spec.legend, spec.pie_legend_in_wedges) = match v.to_ascii_lowercase().as_str() {
            "none" => (false, false),
            "standard" => (true, false),
            "wedges" => (true, true),
            _ => return Err(bad(C, format!("unknown pieLegend `{v}` (none, standard or wedges)"))),
        };
    }
    spec.mark_points = switch_or(p, "markPoints", spec.mark_points, C)?;
    spec.connect_points = switch_or(p, "connectPoints", spec.connect_points, C)?;
    spec.edge_to_edge = switch_or(p, "edgeToEdge", spec.edge_to_edge, C)?;
    spec.ticks = count_given(p, "ticks", 100, C)?.unwrap_or(spec.ticks);
    let label = format!("{} Graph", spec.kind.label());
    s.edit("Graph Type", |d, sel| {
        regenerate(d, id, spec)?;
        if !reselect.is_empty() {
            reselect_series(d, sel, id, &reselect);
        }
        if let Some(n) = d.node_mut(id)
            && n.name.as_deref().is_some_and(|nm| nm.ends_with(" Graph"))
        {
            n.name = Some(label);
        }
        Ok(())
    })?;
    Ok(json!({ "id": id.0 }))
}
