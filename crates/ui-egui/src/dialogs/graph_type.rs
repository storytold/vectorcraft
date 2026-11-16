//! Object › Graph › Type…: Graph Type, for the selected graph or for the series picked in it with
//! Group Selection.
//!
//! A menu at the top shows one page at a time. Graph Options has the Type buttons, the Value Axis
//! side with Separate Scales, the Style checkboxes and the options of the type picked: Column or
//! Bar Width and Cluster Width, the data point and line checkboxes, or a pie graph's Legend,
//! Position and Sort. A page per axis follows for the axes the type has: Value Axis (Left Axis and
//! Right Axis while the value axis is on both sides) with Tick Values, Tick Marks and Add Labels,
//! and Category Axis with Tick Marks. OK runs `graph.setType`, and with nothing changed it closes
//! without a step of history.
//!
//! Fields: the options `graph.setType {}` returns under the command's names for them, with
//! `legend`, `pieLegend`, `piePosition`, `pieSort`, `firstRowInFront`, `firstColumnInFront`,
//! `filledLines` and `lineWidth` held for every type, and `type` or `valueAxis` null while the picked series
//! differ in it (OK then leaves each series as it is); `overrideValues` and `rightOverrideValues` (Override
//! Calculated Values of the left or only value axis and of the right one: while one is on, OK
//! sends `axisMin`, `axisMax` and `ticks`, or `rightAxisMin`, `rightAxisMax` and `rightTicks`); and
//! `__page`, the page shown (`options`, `valueAxis`, `rightAxis` or `categoryAxis`).

use std::ops::RangeInclusive;

use serde_json::{Map, Value, json};
use vectorcraft_doc::GraphKind;

use super::{DialogSpec, form, run_and_close};
use crate::state::Dialog;
use crate::theme::Tokens;
use crate::{VectorcraftApp, widgets};

/// The dialog kind of Graph Type.
pub const KIND: &str = "graphType";

/// The field that holds the page shown.
pub(super) const PAGE: &str = "__page";

const COMMAND: &str = "graph.setType";

/// The document and the graph the dialog opened on, the series picked with Group Selection, the graph's own type and
/// value axis side, each series' own type, the series on the right axis, the tick values each value axis is calculated with for the
/// options the dialog shows and the options and document revision they were calculated for, and the parameters OK would
/// have sent as the dialog opened.
const DOC: &str = "__doc";
const GRAPH_ID: &str = "__graphId";
const SERIES: &str = "__series";
const GRAPH: &str = "__graph";
const GRAPH_AXIS: &str = "__graphAxis";
const SERIES_KINDS: &str = "__seriesKinds";
const RIGHT_SERIES: &str = "__rightSeries";
const CALCULATED: &str = "__calculated";
const CALCULATED_FOR: &str = "__calculatedFor";
const OPENED: &str = "__opened";

const LABEL_W: f32 = 170.0;
const FIELD_W: f32 = 160.0;
const NUMBER_W: f32 = 72.0;
const TYPE_BUTTON: f32 = 34.0;

/// The widths `graph.setType` takes for columns, bars and clusters, in percent.
const WIDTHS: RangeInclusive<f64> = 1.0..=1000.0;
/// The divisions it takes for a value axis, and the tick marks it takes per division.
const DIVISIONS: RangeInclusive<f64> = 1.0..=100.0;
const TICK_MARKS: RangeInclusive<f64> = 1.0..=20.0;
/// The Line Width it takes for Draw Filled Lines, in points.
const LINE_WIDTHS: RangeInclusive<f64> = vectorcraft_doc::MIN_FILLED_LINE_WIDTH..=vectorcraft_doc::MAX_FILLED_LINE_WIDTH;
/// The characters it keeps of a prefix or suffix.
const AFFIX_CHARS: usize = 64;
/// The divisions it draws an overridden value axis in while the axis has no count of its own.
const DEFAULT_DIVISIONS: usize = 5;

/// The types whose value axis goes on a side, those whose right axis takes a scale of its own, and
/// those with a category axis.
const SIDED: [GraphKind; 5] = [GraphKind::Column, GraphKind::StackedColumn, GraphKind::Line, GraphKind::Area, GraphKind::Scatter];
const SCALED: [GraphKind; 4] = [GraphKind::Column, GraphKind::StackedColumn, GraphKind::Line, GraphKind::Area];
const CATEGORIES: [GraphKind; 6] =
    [GraphKind::Column, GraphKind::StackedColumn, GraphKind::Bar, GraphKind::StackedBar, GraphKind::Line, GraphKind::Area];
/// The types drawn as columns or bars, and those drawn as data points and lines.
const COLUMNS: [GraphKind; 4] = [GraphKind::Column, GraphKind::StackedColumn, GraphKind::Bar, GraphKind::StackedBar];
const LINES: [GraphKind; 3] = [GraphKind::Line, GraphKind::Scatter, GraphKind::Radar];

/// Value Axis for the whole graph and for picked series (a series goes on one axis), Tick Marks ›
/// Length, and a pie graph's Legend, Position and Sort, each as (value, label).
const SIDES: [(&str, &str); 3] = [("left", "On Left Side"), ("right", "On Right Side"), ("both", "On Both Sides")];
const SERIES_SIDES: [(&str, &str); 2] = [("left", "On Left Side"), ("right", "On Right Side")];
const TICK_LENGTHS: [(&str, &str); 3] = [("none", "None"), ("short", "Short"), ("full", "Full Width")];
const PIE_LEGENDS: [(&str, &str); 3] = [("none", "No Legend"), ("standard", "Standard Legend"), ("wedges", "Legends in Wedges")];
const PIE_POSITIONS: [(&str, &str); 3] = [("ratio", "Ratio"), ("even", "Even"), ("stacked", "Stacked")];
const PIE_SORTS: [(&str, &str); 3] = [("all", "All"), ("first", "First"), ("none", "None")];

/// The options OK always sends. `valueAxis` is left out while the picked series are on both axes,
/// and the legend and the tick values follow the type and Override Calculated Values.
const SENT: [&str; 25] = [
    "type",
    "valueAxis",
    "separateScales",
    "dropShadow",
    "legendAcrossTop",
    "firstRowInFront",
    "firstColumnInFront",
    "columnWidth",
    "clusterWidth",
    "markPoints",
    "connectPoints",
    "edgeToEdge",
    "filledLines",
    "lineWidth",
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
];

/// The fields of one value axis: its place in the calculated tick values, Override Calculated
/// Values as the dialog shows it and as it opened, the tick values, the tick marks and the labels.
struct Axis {
    index: usize,
    overridden: &'static str,
    opened_overridden: &'static str,
    min: &'static str,
    max: &'static str,
    ticks: &'static str,
    length: &'static str,
    marks: &'static str,
    prefix: &'static str,
    suffix: &'static str,
}

const LEFT: Axis = Axis {
    index: 0,
    overridden: "overrideValues",
    opened_overridden: "__override",
    min: "axisMin",
    max: "axisMax",
    ticks: "ticks",
    length: "tickLength",
    marks: "tickMarks",
    prefix: "prefix",
    suffix: "suffix",
};

const RIGHT: Axis = Axis {
    index: 1,
    overridden: "rightOverrideValues",
    opened_overridden: "__rightOverride",
    min: "rightAxisMin",
    max: "rightAxisMax",
    ticks: "rightTicks",
    length: "rightTickLength",
    marks: "rightTickMarks",
    prefix: "rightPrefix",
    suffix: "rightSuffix",
};

pub(super) const SPEC: DialogSpec = DialogSpec {
    heading: |_| tl!("Graph Type").into(),
    body,
    confirm,
    ok_enabled: Some(|_, d| problem(d).is_none()),
    min_width: 440.0,
    ..DialogSpec::FORM
};

/// Open Graph Type on the options of the selected graph, or of the series picked in it with Group
/// Selection. An error without a selected graph.
pub fn open(app: &mut VectorcraftApp) -> Result<Value, String> {
    let queried = app.session.execute(COMMAND, &json!({})).map_err(|e| e.to_string())?;
    let subject = vectorcraft_engine::graph_type_subject(&app.session).map_err(|e| e.to_string())?;
    let st = app.session.active().ok_or("no document open")?;
    let uid = st.uid;
    let spec = st.doc.node(subject.id).and_then(|n| n.extras().graph.as_deref()).ok_or("select a graph")?;
    let mut fields = queried.as_object().cloned().unwrap_or_default();
    // The options the query leaves out for the graph's types: the dialog shows each one once the
    // type picked uses it.
    let pie_legend = match (spec.legend, spec.pie_legend_in_wedges) {
        (false, _) => "none",
        (true, true) => "wedges",
        (true, false) => "standard",
    };
    for (key, value) in [
        ("firstRowInFront", json!(spec.first_row_in_front)),
        ("firstColumnInFront", json!(spec.first_column_in_front)),
        ("filledLines", json!(spec.filled_lines)),
        ("lineWidth", json!(spec.filled_line_width())),
        ("legend", json!(spec.legend)),
        ("pieLegend", json!(pie_legend)),
        ("piePosition", json!(spec.pie_position.id())),
        ("pieSort", json!(spec.pie_sort.id())),
    ] {
        fields.entry(key).or_insert(value);
    }
    // Picked series of different types show no type, and OK leaves each one's type as it is.
    let mut picked_kinds = subject.series.iter().map(|i| spec.series_kind(*i));
    if picked_kinds.next().is_some_and(|first| picked_kinds.any(|kind| kind != first)) {
        fields.insert("type".into(), Value::Null);
    }
    let count = spec.rows.iter().map(Vec::len).max().unwrap_or(0).min(vectorcraft_doc::MAX_GRAPH_SERIES);
    let series_kinds: Vec<Option<&str>> = (0..count).map(|i| spec.series_kinds.get(i).copied().flatten().map(GraphKind::id)).collect();
    let calculated: Vec<Value> = subject.calculated.iter().map(|(lo, hi, divisions)| json!([lo, hi, divisions])).collect();
    for (axis, min, max) in [(&LEFT, spec.axis_min, spec.axis_max), (&RIGHT, spec.right_axis_min, spec.right_axis_max)] {
        let overridden = json!(min.is_some() && max.is_some());
        fields.insert(axis.overridden.into(), overridden.clone());
        fields.insert(axis.opened_overridden.into(), overridden);
    }
    fields.insert(DOC.into(), json!(uid));
    fields.insert(GRAPH_ID.into(), json!(subject.id.0));
    fields.insert(SERIES.into(), json!(subject.series));
    fields.insert(GRAPH.into(), json!(spec.kind.id()));
    fields.insert(GRAPH_AXIS.into(), json!(spec.value_axis.id()));
    fields.insert(SERIES_KINDS.into(), json!(series_kinds));
    fields.insert(RIGHT_SERIES.into(), json!(spec.right_series.iter().filter(|i| **i < count).collect::<Vec<_>>()));
    fields.insert(CALCULATED.into(), json!(calculated));
    fields.insert(PAGE.into(), json!("options"));
    let mut d = Dialog { kind: KIND.into(), fields };
    let key = calculated_for(app, &d);
    d.fields.insert(CALCULATED_FOR.into(), key);
    settle(&mut d);
    let opened = params(&d);
    d.fields.insert(OPENED.into(), opened);
    app.ui.dialog = Some(d);
    Ok(json!({ "dialog": KIND }))
}

/// The series the type and the value axis go to: those picked with Group Selection. None: the
/// whole graph.
fn picked(d: &Dialog) -> Vec<usize> {
    let indexes = d.fields.get(SERIES).and_then(Value::as_array);
    indexes.map(|a| a.iter().filter_map(|v| v.as_u64().and_then(|i| usize::try_from(i).ok())).collect()).unwrap_or_default()
}

/// The type the Type buttons show.
fn chosen(d: &Dialog) -> Option<GraphKind> {
    d.fields.get("type").and_then(Value::as_str).and_then(GraphKind::parse)
}

/// The graph's own type as OK leaves it: the type picked, or with series picked the type the graph
/// has.
fn graph_kind(d: &Dialog) -> GraphKind {
    let own = d.fields.get(GRAPH).and_then(Value::as_str).and_then(GraphKind::parse).unwrap_or_default();
    if picked(d).is_empty() { chosen(d).unwrap_or(own) } else { own }
}

/// The type each series draws as once OK is pressed: the type picked for the picked series (their
/// own while they differ and none is picked), its own type where that combines with the graph's,
/// else the graph's.
fn series_kinds(d: &Dialog) -> Vec<GraphKind> {
    let (graph, picked, chosen) = (graph_kind(d), picked(d), chosen(d));
    let own: Vec<Option<GraphKind>> = d
        .fields
        .get(SERIES_KINDS)
        .and_then(Value::as_array)
        .map(|a| a.iter().map(|v| v.as_str().and_then(GraphKind::parse)).collect())
        .unwrap_or_default();
    (0..own.len().max(1))
        .map(|i| {
            let kept = own.get(i).copied().flatten();
            let kind = if picked.contains(&i) { chosen.or(kept) } else { kept };
            kind.filter(|k| k.combines_with(graph)).unwrap_or(graph)
        })
        .collect()
}

/// Whether a series draws as one of `kinds` once OK is pressed.
fn draws(d: &Dialog, kinds: &[GraphKind]) -> bool {
    series_kinds(d).iter().any(|k| kinds.contains(k))
}

/// The side or sides the graph's value axis is on once OK is pressed. With series picked, Value
/// Axis is theirs and the graph keeps its own.
fn graph_axis(d: &Dialog) -> String {
    d.str(if picked(d).is_empty() { "valueAxis" } else { GRAPH_AXIS })
}

/// Whether the graph draws a value axis on both sides.
fn both_sides(d: &Dialog) -> bool {
    SIDED.contains(&graph_kind(d)) && graph_axis(d) == "both"
}

/// Whether the right value axis has a scale of its own: Separate Scales with the value axis on
/// both sides of a type that takes one.
fn separate(d: &Dialog) -> bool {
    d.bool("separateScales") && both_sides(d) && SCALED.contains(&graph_kind(d))
}

/// Whether the graph draws a legend beside or above the plot.
fn legend_shown(d: &Dialog) -> bool {
    if graph_kind(d) == GraphKind::Pie { d.str("pieLegend") == "standard" } else { d.bool("legend") }
}

/// The pages the menu lists for the type picked, as (id, name): Graph Options, a page per value
/// axis side (none for a pie graph) and Category Axis for the types that have one.
fn pages(d: &Dialog) -> Vec<(&'static str, &'static str)> {
    let kind = graph_kind(d);
    let mut pages = vec![("options", tl!("Graph Options"))];
    if kind != GraphKind::Pie {
        if both_sides(d) {
            pages.extend([("valueAxis", tl!("Left Axis")), ("rightAxis", tl!("Right Axis"))]);
        } else {
            pages.push(("valueAxis", tl!("Value Axis")));
        }
    }
    if CATEGORIES.contains(&kind) {
        pages.push(("categoryAxis", tl!("Category Axis")));
    }
    pages
}

/// The least value, the greatest and the divisions `axis` is drawn with while its tick values are
/// calculated.
fn calculated(d: &Dialog, axis: &Axis) -> (f64, f64, usize) {
    let of = |i: usize| d.fields.get(CALCULATED)?.get(axis.index)?.get(i)?.as_f64();
    let divisions = of(2).filter(|n| *n >= 1.0).map_or(DEFAULT_DIVISIONS, |n| n.min(*DIVISIONS.end()) as usize);
    (of(0).unwrap_or(0.0), of(1).unwrap_or(1.0), divisions)
}

/// The series on the right value axis once OK is pressed: the graph's, with the picked series moved
/// to the side picked for them.
fn right_series(d: &Dialog) -> Vec<usize> {
    let mut right: Vec<usize> = d
        .fields
        .get(RIGHT_SERIES)
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|v| v.as_u64().and_then(|i| usize::try_from(i).ok())).collect())
        .unwrap_or_default();
    let picked = picked(d);
    match d.fields.get("valueAxis").and_then(Value::as_str) {
        Some("right") => right.extend(picked.into_iter().filter(|i| !right.contains(i)).collect::<Vec<_>>()),
        Some("left") => right.retain(|i| !picked.contains(i)),
        _ => {}
    }
    right
}

/// Whether the right axis' own scale (`right`) or the left one draws a value axis: the left scale
/// draws both sides without Separate Scales, and with it each scale needs a series on its side (the
/// other side's scale draws both sides when one side has none).
fn scale_used(d: &Dialog, right: bool) -> bool {
    if !separate(d) {
        return !right;
    }
    let on_right = right_series(d);
    let count = series_kinds(d).len();
    (0..count).any(|i| on_right.contains(&i) == right)
}

/// The options the calculated tick values depend on, as the dialog shows them: the graph's type,
/// each series' type, the value axis side, Separate Scales, and the side picked series go on.
fn pending(d: &Dialog) -> Value {
    let kinds: Vec<&str> = series_kinds(d).into_iter().map(GraphKind::id).collect();
    let side = d.fields.get("valueAxis").cloned().unwrap_or(Value::Null);
    json!([graph_kind(d).id(), kinds, graph_axis(d), d.bool("separateScales"), side])
}

/// Whether the active document is the one the dialog opened on.
fn on_its_document(app: &VectorcraftApp, d: &Dialog) -> bool {
    app.session.active().is_some_and(|st| d.fields.get(DOC).and_then(Value::as_u64) == Some(st.uid))
}

/// What the calculated tick values were calculated for: the options they depend on and the
/// revision of the active document, which an edit of the graph's data changes.
fn calculated_for(app: &VectorcraftApp, d: &Dialog) -> Value {
    json!([pending(d), app.session.active().map(|st| st.revision)])
}

/// Calculate the tick values again when the options they depend on or the document changed since
/// they were last calculated: the graph with the type, the value axis side and Separate Scales the
/// dialog shows, and the picked series on the type and side picked for them. Nothing while another
/// document is active.
fn recalculate(app: &VectorcraftApp, d: &mut Dialog) {
    if !on_its_document(app, d) {
        return;
    }
    let key = calculated_for(app, d);
    if d.fields.get(CALCULATED_FOR) == Some(&key) {
        return;
    }
    let id = d.fields.get(GRAPH_ID).and_then(Value::as_u64).map(vectorcraft_doc::NodeId);
    let Some(spec) = id.and_then(|id| app.session.active()?.doc.node(id)?.extras().graph.as_deref().cloned()) else {
        return;
    };
    let mut spec = spec;
    let graph = graph_kind(d);
    spec.series_kinds = series_kinds(d).into_iter().map(|kind| (kind != graph).then_some(kind)).collect();
    spec.kind = graph;
    if let Some(side) = vectorcraft_doc::ValueAxisSide::parse(&graph_axis(d)) {
        spec.value_axis = side;
    }
    spec.separate_scales = d.bool("separateScales");
    spec.right_series = right_series(d);
    let calculated: Vec<Value> =
        vectorcraft_engine::graph_calculated_axes(&spec).iter().map(|(lo, hi, divisions)| json!([lo, hi, divisions])).collect();
    let before = d.fields.insert(CALCULATED.into(), json!(calculated));
    d.fields.insert(CALCULATED_FOR.into(), key);
    // Each tick value of an overridden axis that still holds the value calculated before takes the
    // new one, unless it is the value the graph had when the dialog opened.
    for axis in [&LEFT, &RIGHT] {
        if !d.bool(axis.overridden) {
            continue;
        }
        let was = |i: usize| before.as_ref()?.get(axis.index)?.get(i)?.as_f64();
        let saved = |key: &str| d.fields.get(OPENED)?.get(key)?.as_f64().filter(|_| d.bool(axis.opened_overridden));
        let (lo, hi, divisions) = self::calculated(d, axis);
        let replaced: Vec<(&str, Value)> = [(axis.min, json!(lo)), (axis.max, json!(hi)), (axis.ticks, json!(divisions))]
            .into_iter()
            .enumerate()
            .filter(|(i, (key, _))| number(d, key).is_some_and(|v| Some(v) == was(*i) && Some(v) != saved(key)))
            .map(|(_, field)| field)
            .collect();
        for (key, value) in replaced {
            d.fields.insert(key.into(), value);
        }
    }
}

/// Field `key` as a number. `None` when it is missing, null or not a number.
fn number(d: &Dialog, key: &str) -> Option<f64> {
    d.fields.get(key).and_then(Value::as_f64)
}

/// The least and greatest value `axis` shows: its own, else the calculated ones.
fn bounds(d: &Dialog, axis: &Axis) -> (f64, f64) {
    let (lo, hi, _) = calculated(d, axis);
    (number(d, axis.min).unwrap_or(lo), number(d, axis.max).unwrap_or(hi))
}

/// The divisions `axis` shows: its own count, else the count it is drawn in.
fn divisions(d: &Dialog, axis: &Axis) -> usize {
    let own = number(d, axis.ticks).filter(|n| *n >= 1.0).map(|n| n.min(*DIVISIONS.end()) as usize);
    let both = number(d, axis.min).is_some() && number(d, axis.max).is_some();
    own.unwrap_or(if both { DEFAULT_DIVISIONS } else { calculated(d, axis).2 })
}

/// Give each axis whose Override Calculated Values is on the tick values it lacks: the calculated
/// ones, which the fields then show and OK sends.
fn settle(d: &mut Dialog) {
    for axis in [&LEFT, &RIGHT] {
        if !d.bool(axis.overridden) {
            continue;
        }
        let ((lo, hi), count) = (bounds(d, axis), divisions(d, axis));
        // A value that is not a number stays, and `graph.setType` refuses it.
        let missing = |d: &Dialog, key: &str| d.fields.get(key).is_none_or(Value::is_null);
        for (key, value) in [(axis.min, json!(lo)), (axis.max, json!(hi))] {
            if missing(d, key) {
                d.fields.insert(key.into(), value);
            }
        }
        if missing(d, axis.ticks) || number(d, axis.ticks).is_some_and(|n| n < 1.0) {
            d.fields.insert(axis.ticks.into(), json!(count));
        }
    }
}

/// Why OK can't apply the dialog: an overridden value axis whose maximum is not more than its
/// minimum, which `graph.setType` would draw with calculated values instead.
fn problem(d: &Dialog) -> Option<&'static str> {
    let out_of_order = |axis: &Axis| {
        let (lo, hi) = bounds(d, axis);
        d.bool(axis.overridden) && !(hi > lo && (hi - lo).is_finite())
    };
    let refused = graph_kind(d) != GraphKind::Pie && [&LEFT, &RIGHT].into_iter().any(|axis| scale_used(d, axis.index == 1) && out_of_order(axis));
    refused.then(|| tl!("Maximum must be more than Minimum."))
}

/// `graph.setType`'s parameters for the dialog as it is. The type and the value axis go to the
/// series picked with Group Selection, as the command reads the selection.
fn params(d: &Dialog) -> Value {
    let mut d = d.clone();
    settle(&mut d);
    let mut p = Map::new();
    let send = |p: &mut Map<String, Value>, key: &str| {
        if let Some(v) = d.fields.get(key).filter(|v| !v.is_null()) {
            p.insert(key.into(), v.clone());
        }
    };
    for key in SENT {
        send(&mut p, key);
    }
    // A pie graph's Legend menu stands for the legend checkbox.
    let legend: &[&str] = if graph_kind(&d) == GraphKind::Pie { &["pieLegend", "piePosition", "pieSort"] } else { &["legend"] };
    for key in legend {
        send(&mut p, key);
    }
    for axis in [&LEFT, &RIGHT] {
        if d.bool(axis.overridden) {
            for key in [axis.min, axis.max, axis.ticks] {
                send(&mut p, key);
            }
        } else if d.bool(axis.opened_overridden) {
            // Turned off: the tick values are calculated again.
            p.extend([(axis.min.into(), Value::Null), (axis.max.into(), Value::Null), (axis.ticks.into(), json!(0))]);
        }
    }
    Value::Object(p)
}

/// Whether two sets of parameters hold the same values, a count being the same as the number a
/// field gives for it.
fn same(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x.as_f64() == y.as_f64(),
        (Value::Object(x), Value::Object(y)) => x.len() == y.len() && x.iter().all(|(k, v)| y.get(k).is_some_and(|w| same(v, w))),
        _ => a == b,
    }
}

fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    // Another document is active: its graphs have ids of their own, so OK closes without a change.
    if !on_its_document(app, d) {
        app.ui.dialog = None;
        return Ok(Value::Null);
    }
    let mut d = d.clone();
    recalculate(app, &mut d);
    if let Some(why) = problem(&d) {
        return Err(why.into());
    }
    let mut p = params(&d);
    // Nothing changed: no step of history.
    if d.fields.get(OPENED).is_some_and(|opened| same(opened, &p)) {
        app.ui.dialog = None;
        return Ok(Value::Null);
    }
    aim(app, &d, &mut p);
    run_and_close(app, COMMAND, p)
}

/// Point `p` at the graph and the series the dialog opened on when the selection no longer holds
/// them: `id`, and `seriesIndexes` while series are picked and a type or a value axis goes to them.
/// While the selection still holds them, the command reads it and keeps picked series selected.
fn aim(app: &VectorcraftApp, d: &Dialog, p: &mut Value) {
    let Some(id) = d.fields.get(GRAPH_ID).and_then(Value::as_u64) else {
        return;
    };
    let now = vectorcraft_engine::graph_type_subject(&app.session).ok();
    if now.is_some_and(|now| now.id.0 == id && now.series == picked(d)) {
        return;
    }
    let Some(p) = p.as_object_mut() else {
        return;
    };
    p.insert("id".into(), json!(id));
    let picked = picked(d);
    // Picked series of different types on both axes send neither, and the other options go to the
    // whole graph.
    if !picked.is_empty() && (p.contains_key("type") || p.contains_key("valueAxis")) {
        p.insert("seriesIndexes".into(), json!(picked));
    }
}

fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    recalculate(app, d);
    settle(d);
    let pages = pages(d);
    let (page, shown) = pages.iter().find(|(id, _)| *id == d.str(PAGE)).or(pages.first()).copied().unwrap_or(("options", ""));
    let names: Vec<&str> = pages.iter().map(|(_, name)| *name).collect();
    let pick = widgets::dropdown_names_named(ui, "graph-type-page", Some(tl!("Graph Type")), shown, &names, FIELD_W);
    if let Some((id, _)) = pick.and_then(|i| pages.get(i)) {
        d.fields.insert(PAGE.into(), json!(id));
    }
    ui.add_space(8.0);
    match page {
        "valueAxis" => value_page(ui, d, false),
        "rightAxis" => value_page(ui, d, true),
        "categoryAxis" => category_page(ui, d),
        _ => options_page(ui, d),
    }
    if let Some(why) = problem(d) {
        ui.add_space(6.0);
        ui.colored_label(Tokens::get(ui.ctx()).error, why);
    }
    false
}

/// `text` with the colon of a field label.
fn colon(text: &str) -> String {
    format!("{text}:")
}

/// A checkbox bound to `d.fields[key]`, dimmed while the type picked has no use for it.
fn check(ui: &mut egui::Ui, d: &mut Dialog, key: &str, label: &str, enabled: bool) {
    let on = d.bool(key);
    if widgets::check(ui, label, on, enabled) {
        d.fields.insert(key.into(), json!(!on));
    }
}

/// A dropdown row bound to the string `d.fields[key]`; `options` are (value, label). A value that
/// is not among them, such as the null of series on both value axes, shows a blank field.
fn choice(ui: &mut egui::Ui, d: &mut Dialog, key: &str, label: &str, options: &[(&str, &str)]) {
    widgets::label_row(ui, label, LABEL_W, |ui| {
        let current = d.fields.get(key).and_then(Value::as_str).unwrap_or_default();
        let shown = options.iter().find(|(value, _)| *value == current).map_or("", |(_, label)| *label);
        let labels: Vec<&str> = options.iter().map(|(_, label)| *label).collect();
        if let Some((value, _)) = widgets::dropdown(ui, ("graph-type", key), shown, &labels, FIELD_W).and_then(|i| options.get(i)) {
            d.fields.insert(key.into(), json!(value));
        }
    });
}

/// A number row bound to `d.fields[key]`, kept within `range` and rounded to `decimals` places.
fn ranged(ui: &mut egui::Ui, d: &mut Dialog, key: &str, label: &str, shown: f64, (range, suffix, decimals): (RangeInclusive<f64>, &str, usize)) {
    widgets::label_row(ui, label, LABEL_W, |ui| {
        if let Some(v) = widgets::range_field(ui, ("graph-type", key), shown, range, suffix, decimals, NUMBER_W) {
            d.fields.insert(key.into(), json!(v));
        }
    });
}

/// A text row bound to `d.fields[key]` that takes the characters `graph.setType` keeps.
fn text(ui: &mut egui::Ui, d: &mut Dialog, key: &str, label: &str) {
    widgets::label_row(ui, label, LABEL_W, |ui| {
        form::text_edit_limited(ui, d, key, FIELD_W, AFFIX_CHARS);
    });
}

fn options_page(ui: &mut egui::Ui, d: &mut Dialog) {
    widgets::subheader(ui, tl!("Type"));
    type_row(ui, d);
    ui.add_space(4.0);
    let kind = graph_kind(d);
    ui.add_enabled_ui(SIDED.contains(&kind), |ui| {
        let sides: &[(&str, &str)] = if picked(d).is_empty() { &SIDES } else { &SERIES_SIDES };
        choice(ui, d, "valueAxis", &colon(tl!("Value Axis")), sides);
    });
    let scaled = both_sides(d) && SCALED.contains(&kind);
    check(ui, d, "separateScales", tl!("Separate Scales"), scaled);
    ui.add_space(8.0);
    widgets::subheader(ui, tl!("Style"));
    let columns = draws(d, &COLUMNS);
    egui::Grid::new("graph-type-style").num_columns(2).spacing([24.0, 6.0]).show(ui, |ui| {
        check(ui, d, "dropShadow", tl!("Add Drop Shadow"), true);
        check(ui, d, "firstRowInFront", tl!("First Row in Front"), columns);
        ui.end_row();
        check(ui, d, "legendAcrossTop", tl!("Add Legend Across Top"), legend_shown(d));
        check(ui, d, "firstColumnInFront", tl!("First Column in Front"), columns);
        ui.end_row();
        // A pie graph has its Legend menu among its options.
        if kind != GraphKind::Pie {
            check(ui, d, "legend", tl!("Legend"), true);
            ui.end_row();
        }
    });
    type_options(ui, d, kind);
}

/// The Options of the type picked: widths for columns and bars, data points and lines for line,
/// scatter and radar series, and Legend, Position and Sort for a pie graph. An area graph has none.
fn type_options(ui: &mut egui::Ui, d: &mut Dialog, kind: GraphKind) {
    let (columns, lines, pie) = (draws(d, &COLUMNS), draws(d, &LINES), kind == GraphKind::Pie);
    if !(columns || lines || pie) {
        return;
    }
    ui.add_space(8.0);
    widgets::subheader(ui, tl!("Options"));
    if columns {
        let width = if matches!(kind, GraphKind::Bar | GraphKind::StackedBar) { tl!("Bar Width") } else { tl!("Column Width") };
        for (key, label) in [("columnWidth", width), ("clusterWidth", tl!("Cluster Width"))] {
            let shown = number(d, key).unwrap_or(100.0);
            ranged(ui, d, key, &colon(label), shown, (WIDTHS, "%", 2));
        }
    }
    if lines {
        // Edge-to-Edge Lines is for line series, and Draw Filled Lines for connected data points.
        let (line, connected) = (draws(d, &[GraphKind::Line]), d.bool("connectPoints"));
        egui::Grid::new("graph-type-lines").num_columns(2).spacing([24.0, 6.0]).show(ui, |ui| {
            check(ui, d, "markPoints", tl!("Mark Data Points"), true);
            check(ui, d, "edgeToEdge", tl!("Edge-to-Edge Lines"), line);
            ui.end_row();
            check(ui, d, "connectPoints", tl!("Connect Data Points"), true);
            check(ui, d, "filledLines", tl!("Draw Filled Lines"), connected);
            ui.end_row();
        });
        // Line Width sets how wide Draw Filled Lines draws the lines.
        let shown = number(d, "lineWidth").unwrap_or(vectorcraft_doc::DEFAULT_FILLED_LINE_WIDTH);
        ui.add_enabled_ui(connected && d.bool("filledLines"), |ui| {
            ranged(ui, d, "lineWidth", &colon(tl!("Line Width")), shown, (LINE_WIDTHS, " pt", 2));
        });
    }
    if pie {
        choice(ui, d, "pieLegend", &colon(tl!("Legend")), &PIE_LEGENDS);
        choice(ui, d, "piePosition", &colon(tl!("Position")), &PIE_POSITIONS);
        choice(ui, d, "pieSort", &colon(tl!("Sort")), &PIE_SORTS);
    }
}

/// The page of a value axis: the tick values of the scale it shows, and the tick marks and labels
/// of its side. An only value axis on the right side shows the left scale with the right side's
/// tick marks and labels, as the graph draws it.
fn value_page(ui: &mut egui::Ui, d: &mut Dialog, right: bool) {
    let scale = if right { &RIGHT } else { &LEFT };
    let side = if right || (SIDED.contains(&graph_kind(d)) && graph_axis(d) == "right") { &RIGHT } else { &LEFT };
    widgets::subheader(ui, tl!("Tick Values"));
    // Without Separate Scales the right axis shows the left axis' tick values, and with it a side
    // without series shows the other side's.
    let own_scale = scale_used(d, right);
    // A side without a scale of its own shows the tick values of the scale drawn on it, and with
    // Override Calculated Values off a scale shows the values it is calculated with.
    let drawn = if own_scale {
        scale
    } else if right {
        &LEFT
    } else {
        &RIGHT
    };
    check(ui, d, drawn.overridden, tl!("Override Calculated Values"), own_scale);
    ui.add_enabled_ui(own_scale && d.bool(scale.overridden), |ui| {
        let ((lo, hi), count) = if d.bool(drawn.overridden) {
            (bounds(d, drawn), divisions(d, drawn))
        } else {
            let (lo, hi, count) = calculated(d, drawn);
            ((lo, hi), count)
        };
        for (key, label, shown) in [(scale.min, tl!("Minimum:"), lo), (scale.max, tl!("Maximum:"), hi)] {
            widgets::label_row(ui, label, LABEL_W, |ui| {
                if let Some(v) = widgets::plain_field(ui, ("graph-type", key), shown, "", 4, NUMBER_W) {
                    d.fields.insert(key.into(), json!(v));
                }
            });
        }
        ranged(ui, d, scale.ticks, &colon(tl!("Divisions")), count as f64, (DIVISIONS, "", 0));
    });
    ui.add_space(8.0);
    tick_marks(ui, d, side.length, side.marks);
    ui.add_space(8.0);
    widgets::subheader(ui, tl!("Add Labels"));
    text(ui, d, side.prefix, tl!("Prefix:"));
    text(ui, d, side.suffix, &colon(tl!("Suffix")));
}

fn category_page(ui: &mut egui::Ui, d: &mut Dialog) {
    tick_marks(ui, d, "categoryTickLength", "categoryTickMarks");
    let drawn = d.str("categoryTickLength") != "none";
    check(ui, d, "ticksBetweenLabels", tl!("Draw Tick Marks Between Labels"), drawn);
}

/// Tick Marks: the length, and the tick marks per division while there are tick marks to draw.
fn tick_marks(ui: &mut egui::Ui, d: &mut Dialog, length: &str, marks: &str) {
    widgets::subheader(ui, tl!("Tick Marks"));
    choice(ui, d, length, &colon(tl!("Length")), &TICK_LENGTHS);
    let shown = number(d, marks).unwrap_or(1.0);
    ui.add_enabled_ui(d.str(length) != "none", |ui| {
        ranged(ui, d, marks, &colon(tl!("Tick Marks per Division")), shown, (TICK_MARKS, "", 0));
    });
}

/// The nine Type buttons. With series picked, the types that cannot go in the graph are dimmed.
fn type_row(ui: &mut egui::Ui, d: &mut Dialog) {
    let own = (!picked(d).is_empty()).then(|| graph_kind(d));
    let current = chosen(d);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        for kind in GraphKind::ALL {
            let enabled = own.is_none_or(|graph| kind.combines_with(graph));
            if type_button(ui, kind, current == Some(kind), enabled) {
                d.fields.insert("type".into(), json!(kind.id()));
            }
        }
    });
}

/// The name of a graph type, in the UI language.
fn kind_name(kind: GraphKind) -> &'static str {
    match kind {
        GraphKind::Column => tl!("Column Graph"),
        GraphKind::StackedColumn => tl!("Stacked Column Graph"),
        GraphKind::Bar => tl!("Bar Graph"),
        GraphKind::StackedBar => tl!("Stacked Bar Graph"),
        GraphKind::Line => tl!("Line Graph"),
        GraphKind::Area => tl!("Area Graph"),
        GraphKind::Scatter => tl!("Scatter Graph"),
        GraphKind::Pie => tl!("Pie Graph"),
        GraphKind::Radar => tl!("Radar Graph"),
    }
}

/// A Type button: the type's picture, pressed while it is the type picked. Whether it was clicked.
fn type_button(ui: &mut egui::Ui, kind: GraphKind, selected: bool, enabled: bool) -> bool {
    let t = Tokens::get(ui.ctx());
    let sense = if enabled { egui::Sense::click() } else { egui::Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(egui::Vec2::splat(TYPE_BUTTON), sense);
    let fill = if selected {
        t.tool_active
    } else if enabled && resp.hovered() {
        t.hover
    } else {
        egui::Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, egui::CornerRadius::same(5), fill);
    let ink = if !enabled {
        t.text_disabled
    } else if selected {
        t.text_strong
    } else {
        t.icon
    };
    paint_kind(ui.painter(), rect.shrink(7.0), kind, ink);
    let name = kind_name(kind);
    resp.widget_info(|| widgets::icon_button_info(enabled, selected, name));
    resp.on_hover_text(name).clicked()
}

/// Draw the picture of graph type `kind` in `r` with `ink`.
fn paint_kind(p: &egui::Painter, r: egui::Rect, kind: GraphKind, ink: egui::Color32) {
    let at = |x: f32, y: f32| egui::pos2(r.left() + x * r.width(), r.top() + y * r.height());
    let faint = ink.gamma_multiply(0.45);
    let block = |from: (f32, f32), to: (f32, f32), color: egui::Color32| {
        p.rect_filled(egui::Rect::from_two_pos(at(from.0, from.1), at(to.0, to.1)), 0.0, color);
    };
    let stroke = egui::Stroke::new(1.5, ink);
    // Three columns or bars: where each one starts across the picture, and how long it is.
    let marks = [(0.04, 0.55), (0.37, 0.95), (0.7, 0.75)];
    match kind {
        GraphKind::Column | GraphKind::StackedColumn => {
            for (x, len) in marks {
                block((x, 1.0 - len), (x + 0.26, 1.0), ink);
                if kind == GraphKind::StackedColumn {
                    block((x, 1.0 - len), (x + 0.26, 1.0 - len * 0.55), faint);
                }
            }
        }
        GraphKind::Bar | GraphKind::StackedBar => {
            for (y, len) in marks {
                block((0.0, y), (len, y + 0.26), ink);
                if kind == GraphKind::StackedBar {
                    block((len * 0.55, y), (len, y + 0.26), faint);
                }
            }
        }
        GraphKind::Line | GraphKind::Area | GraphKind::Scatter => paint_points(p, &at, kind, ink),
        GraphKind::Pie => {
            let (centre, radius) = (r.center(), r.width().min(r.height()) / 2.0);
            p.circle_stroke(centre, radius - 0.75, stroke);
            let arc = (0..=8).map(|i| {
                let a = -std::f32::consts::FRAC_PI_2 + std::f32::consts::FRAC_PI_2 * 1.3 * i as f32 / 8.0;
                centre + egui::vec2(a.cos(), a.sin()) * radius
            });
            let wedge: Vec<egui::Pos2> = std::iter::once(centre).chain(arc).collect();
            p.add(egui::Shape::convex_polygon(wedge, ink, egui::Stroke::NONE));
        }
        GraphKind::Radar => {
            let (centre, radius) = (r.center(), r.width().min(r.height()) / 2.0);
            let ring = |reach: [f32; 5]| -> Vec<egui::Pos2> {
                let corner = |(i, len): (usize, &f32)| {
                    let a = -std::f32::consts::FRAC_PI_2 + std::f32::consts::TAU * i as f32 / 5.0;
                    centre + egui::vec2(a.cos(), a.sin()) * radius * *len
                };
                reach.iter().enumerate().map(corner).collect()
            };
            p.add(egui::Shape::closed_line(ring([1.0; 5]), egui::Stroke::new(1.0, faint)));
            p.add(egui::Shape::closed_line(ring([0.85, 0.5, 0.75, 0.45, 0.7]), stroke));
        }
    }
}

/// The pictures of the line, area and scatter types: four data points joined by a line, filled
/// down to the baseline, or on their own with a second row of points.
fn paint_points(p: &egui::Painter, at: &dyn Fn(f32, f32) -> egui::Pos2, kind: GraphKind, ink: egui::Color32) {
    let points = [(0.05, 0.7), (0.35, 0.3), (0.65, 0.5), (0.95, 0.1)];
    match kind {
        GraphKind::Area => {
            // A band under each segment: the area under the line is not convex as a whole.
            for pair in points.windows(2) {
                if let [(x0, y0), (x1, y1)] = *pair {
                    let band = vec![at(x0, 1.0), at(x0, y0), at(x1, y1), at(x1, 1.0)];
                    p.add(egui::Shape::convex_polygon(band, ink, egui::Stroke::NONE));
                }
            }
        }
        GraphKind::Scatter => {
            for (x, y) in points.into_iter().chain([(0.2, 0.25), (0.5, 0.85), (0.8, 0.6)]) {
                p.circle_filled(at(x, y), 1.8, ink);
            }
        }
        _ => {
            p.add(egui::Shape::line(points.iter().map(|(x, y)| at(*x, *y)).collect(), egui::Stroke::new(1.5, ink)));
            for (x, y) in points {
                p.circle_filled(at(x, y), 2.0, ink);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use egui::accesskit::Role;
    use egui_kittest::kittest::{NodeT, Queryable};
    use vectorcraft_engine::Session;

    use super::*;

    type Harness = egui_kittest::Harness<'static, VectorcraftApp>;

    /// A 400 by 400 document with a selected graph of type `kind`: series a and b over Q1 (1, 2)
    /// and Q2 (3, 4).
    fn app_with_graph(kind: &str) -> VectorcraftApp {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 400, "height": 400})).unwrap();
        app.run("graph.create", json!({"type": kind, "x": 0, "y": 0, "width": 200, "height": 150, "csv": ",a,b\nQ1,1,2\nQ2,3,4"})).unwrap();
        app
    }

    /// The open dialog drawn in a kittest harness.
    fn harness(app: VectorcraftApp) -> Harness {
        let mut fonts = false;
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1100.0, 900.0)).build_ui_state(
            move |ui, app: &mut VectorcraftApp| {
                if !std::mem::replace(&mut fonts, true) {
                    crate::theme::install_fonts(ui.ctx());
                    return;
                }
                crate::dialogs::show(app, ui.ctx());
            },
            app,
        );
        h.run_steps(5);
        h
    }

    fn query(app: &mut VectorcraftApp) -> Value {
        app.session.execute(COMMAND, &json!({})).unwrap()
    }

    fn set(app: &mut VectorcraftApp, key: &str, value: Value) {
        app.ui.dialog.as_mut().unwrap().fields.insert(key.into(), value);
    }

    fn steps(app: &VectorcraftApp) -> usize {
        app.session.active().unwrap().history.undo.len()
    }

    /// Clicks the Type button named `name`.
    fn pick_type(h: &mut Harness, name: &str) {
        h.get_by_role_and_label(Role::Button, name).click();
        h.run_steps(3);
    }

    /// Object › Graph › Type… opens the dialog on the selected graph's options, on its Graph
    /// Options page, and passes kittest's check that every input has a name on each page.
    #[test]
    fn the_graph_menu_opens_graph_type_on_the_selected_graph() {
        let mut app = app_with_graph("column");
        crate::menus::invoke(&mut app, COMMAND, json!({}));
        let d = app.ui.dialog.as_ref().unwrap();
        assert_eq!((d.kind.as_str(), d.str("type"), d.str("valueAxis"), d.str(PAGE)), (KIND, "column".into(), "left".into(), "options".into()));
        assert!(picked(d).is_empty() && !d.bool("overrideValues") && !d.bool("rightOverrideValues"));
        for key in ["legend", "pieLegend", "piePosition", "pieSort", "firstRowInFront", "firstColumnInFront", "filledLines"] {
            assert!(d.fields.contains_key(key), "{key} is held for every type");
        }
        set(&mut app, "valueAxis", json!("both"));
        let mut h = harness(app);
        let _ = h.get_by_role_and_label(Role::ComboBox, "Graph Type");
        let _ = h.get_by_role_and_label(Role::Button, "Pie Graph");
        // The right axis shows the left axis' tick values until Separate Scales gives it its own.
        for (page, own_scale) in [("valueAxis", true), ("rightAxis", false)] {
            set(h.state_mut(), PAGE, json!(page));
            h.run_steps(3);
            h.check_accessibility();
            let overridden = h.get_by_role_and_label(Role::CheckBox, "Override Calculated Values");
            assert_eq!(overridden.accesskit_node().is_disabled(), !own_scale, "{page}");
            let _ = h.get_by_role_and_label(Role::TextInput, "Prefix:");
        }
        set(h.state_mut(), PAGE, json!("categoryAxis"));
        h.run_steps(3);
        h.check_accessibility();
        assert!(h.get_by_role_and_label(Role::CheckBox, "Draw Tick Marks Between Labels").accesskit_node().is_disabled(), "no tick marks to place");
        // Without a selected graph the menu item opens no dialog.
        let app = h.state_mut();
        app.ui.dialog = None;
        app.run("select.set", json!({"ids": []})).unwrap();
        assert!(open(app).is_err());
        assert!(app.ui.dialog.is_none());
    }

    /// A Type button changes the graph's type and keeps its data.
    #[test]
    fn a_type_button_changes_the_graph_type_and_keeps_its_data() {
        let mut app = app_with_graph("column");
        let data = app.session.execute("graph.setData", &json!({})).unwrap();
        assert_eq!(data["rows"], json!([[1.0, 2.0], [3.0, 4.0]]));
        open(&mut app).unwrap();
        let mut h = harness(app);
        for kind in GraphKind::ALL {
            pick_type(&mut h, kind_name(kind));
            assert_eq!(h.state().ui.dialog.as_ref().unwrap().str("type"), kind.id());
        }
        pick_type(&mut h, "Stacked Bar Graph");
        let app = h.state_mut();
        let before = steps(app);
        crate::dialogs::confirm(app).unwrap();
        assert!(app.ui.dialog.is_none());
        assert_eq!(query(app)["type"], "stackedBar");
        assert_eq!(app.session.execute("graph.setData", &json!({})).unwrap(), data);
        assert_eq!(steps(app), before + 1, "one step of history");
    }

    /// The menus list the values `graph.setType` takes, and Value Axis puts the value axis on the
    /// side picked.
    #[test]
    fn the_menus_list_the_values_the_command_takes() {
        let doc = vectorcraft_engine::find_command(COMMAND).unwrap().params;
        let lists: [(&str, &[(&str, &str)]); 4] =
            [("valueAxis", &SIDES), ("tickLength", &TICK_LENGTHS), ("pieLegend", &PIE_LEGENDS), ("pieSort", &PIE_SORTS)];
        for (key, options) in lists {
            let values: Vec<&str> = options.iter().map(|(value, _)| *value).collect();
            assert!(doc.contains(&format!("{key}?: {}", values.join("|"))), "{key}: {values:?}");
        }
        assert!(doc.contains("piePosition?: ratio|even|stacked"));
        let mut app = app_with_graph("column");
        open(&mut app).unwrap();
        let mut h = harness(app);
        assert!(h.get_by_role_and_label(Role::CheckBox, "Separate Scales").accesskit_node().is_disabled(), "one side: one scale");
        h.get_by_role_and_label(Role::ComboBox, "Value Axis:").click();
        h.run_steps(3);
        h.get_by_label("On Both Sides").click();
        h.run_steps(3);
        assert_eq!(h.state().ui.dialog.as_ref().unwrap().str("valueAxis"), "both");
        assert!(!h.get_by_role_and_label(Role::CheckBox, "Separate Scales").accesskit_node().is_disabled());
        let app = h.state_mut();
        crate::dialogs::confirm(app).unwrap();
        assert_eq!(query(app)["valueAxis"], "both");
    }

    /// The Options group shows the options of the type picked: widths for columns and bars, the
    /// data point and line checkboxes for line, scatter and radar graphs, the three menus for a
    /// pie graph, and none for an area graph.
    #[test]
    fn the_options_follow_the_type_picked() {
        let mut app = app_with_graph("column");
        open(&mut app).unwrap();
        let mut h = harness(app);
        let field = |h: &Harness, name: &str| h.query_by_role_and_label(Role::TextInput, name).is_some();
        let checkbox = |h: &Harness, name: &str| h.query_by_role_and_label(Role::CheckBox, name).is_some();
        let menu = |h: &Harness, name: &str| h.query_by_role_and_label(Role::ComboBox, name).is_some();
        let dimmed = |h: &Harness, name: &str| h.get_by_role_and_label(Role::CheckBox, name).accesskit_node().is_disabled();
        assert!(field(&h, "Column Width:") && field(&h, "Cluster Width:") && !checkbox(&h, "Mark Data Points") && !menu(&h, "Legend:"));
        assert!(!dimmed(&h, "First Row in Front") && !dimmed(&h, "First Column in Front") && checkbox(&h, "Legend"));
        pick_type(&mut h, "Bar Graph");
        assert!(field(&h, "Bar Width:") && !field(&h, "Column Width:") && field(&h, "Cluster Width:"));
        assert!(
            h.get_by_role_and_label(Role::ComboBox, "Value Axis:").accesskit_node().is_disabled(),
            "a bar graph's value axis stays along the bottom"
        );
        pick_type(&mut h, "Line Graph");
        assert!(!field(&h, "Cluster Width:") && dimmed(&h, "First Row in Front") && dimmed(&h, "First Column in Front"));
        for name in ["Mark Data Points", "Connect Data Points", "Edge-to-Edge Lines", "Draw Filled Lines"] {
            assert!(checkbox(&h, name) && !dimmed(&h, name), "{name}");
        }
        // Draw Filled Lines is for connected data points, and Edge-to-Edge Lines for line graphs.
        set(h.state_mut(), "connectPoints", json!(false));
        h.run_steps(3);
        assert!(dimmed(&h, "Draw Filled Lines"));
        for name in ["Scatter Graph", "Radar Graph"] {
            pick_type(&mut h, name);
            assert!(checkbox(&h, "Mark Data Points") && dimmed(&h, "Edge-to-Edge Lines"), "{name}");
        }
        pick_type(&mut h, "Pie Graph");
        assert!(menu(&h, "Legend:") && menu(&h, "Position:") && menu(&h, "Sort:") && !checkbox(&h, "Legend") && !checkbox(&h, "Mark Data Points"));
        assert!(h.get_by_role_and_label(Role::ComboBox, "Value Axis:").accesskit_node().is_disabled(), "a pie graph has no value axis");
        // Add Legend Across Top is for a legend outside the wedges.
        assert!(!dimmed(&h, "Add Legend Across Top"));
        set(h.state_mut(), "pieLegend", json!("wedges"));
        h.run_steps(3);
        assert!(dimmed(&h, "Add Legend Across Top"));
        pick_type(&mut h, "Area Graph");
        assert!(!field(&h, "Column Width:") && !checkbox(&h, "Mark Data Points") && !menu(&h, "Legend:") && h.query_by_label("Options").is_none());
    }

    /// Line Width next to Draw Filled Lines shows the graph's width, is available only while Draw
    /// Filled Lines and Connect Data Points are on, and OK gives the graph the width typed.
    #[test]
    fn line_width_goes_with_draw_filled_lines() {
        let mut app = app_with_graph("line");
        open(&mut app).unwrap();
        assert_eq!(number(app.ui.dialog.as_ref().unwrap(), "lineWidth"), Some(vectorcraft_doc::DEFAULT_FILLED_LINE_WIDTH));
        let mut h = harness(app);
        let dimmed = |h: &Harness| h.get_by_role_and_label(Role::TextInput, "Line Width:").accesskit_node().is_disabled();
        assert!(dimmed(&h), "Draw Filled Lines is off");
        h.get_by_role_and_label(Role::CheckBox, "Draw Filled Lines").click();
        h.run_steps(3);
        assert!(!dimmed(&h));
        h.check_accessibility();
        set(h.state_mut(), "connectPoints", json!(false));
        h.run_steps(3);
        assert!(dimmed(&h), "no lines to fill");
        set(h.state_mut(), "connectPoints", json!(true));
        set(h.state_mut(), "lineWidth", json!(6.5));
        let app = h.state_mut();
        let before = steps(app);
        crate::dialogs::confirm(app).unwrap();
        assert_eq!((query(app)["filledLines"].clone(), query(app)["lineWidth"].clone()), (json!(true), json!(6.5)));
        assert_eq!(steps(app), before + 1, "one step of history");
    }

    /// The menu at the top lists a page per axis of the type picked, and a second value axis page
    /// while the value axis is on both sides.
    #[test]
    fn the_menu_lists_a_page_per_axis_of_the_type() {
        let mut app = app_with_graph("column");
        open(&mut app).unwrap();
        let mut d = app.ui.dialog.clone().unwrap();
        let ids = |d: &Dialog| pages(d).iter().map(|(id, _)| *id).collect::<Vec<_>>();
        let names = |d: &Dialog| pages(d).iter().map(|(_, name)| *name).collect::<Vec<_>>();
        for kind in ["column", "stackedColumn", "bar", "stackedBar", "line", "area"] {
            d.fields.insert("type".into(), json!(kind));
            assert_eq!(ids(&d), ["options", "valueAxis", "categoryAxis"], "{kind}");
        }
        assert_eq!(names(&d), ["Graph Options", "Value Axis", "Category Axis"]);
        for kind in ["scatter", "radar"] {
            d.fields.insert("type".into(), json!(kind));
            assert_eq!(ids(&d), ["options", "valueAxis"], "{kind}");
        }
        d.fields.insert("type".into(), json!("pie"));
        assert_eq!(ids(&d), ["options"]);
        d.fields.insert("valueAxis".into(), json!("both"));
        for kind in ["column", "stackedColumn", "line", "area"] {
            d.fields.insert("type".into(), json!(kind));
            assert_eq!(ids(&d), ["options", "valueAxis", "rightAxis", "categoryAxis"], "{kind}");
            assert_eq!(names(&d), ["Graph Options", "Left Axis", "Right Axis", "Category Axis"], "{kind}");
        }
        d.fields.insert("type".into(), json!("scatter"));
        assert_eq!(ids(&d), ["options", "valueAxis", "rightAxis"]);
        // A bar or radar graph's value axis has no sides, and a pie graph has no value axis.
        for (kind, pages) in
            [("bar", vec!["options", "valueAxis", "categoryAxis"]), ("radar", vec!["options", "valueAxis"]), ("pie", vec!["options"])]
        {
            d.fields.insert("type".into(), json!(kind));
            assert_eq!(ids(&d), pages, "{kind}");
        }
        // A page the type picked lacks shows Graph Options.
        d.fields.insert(PAGE.into(), json!("rightAxis"));
        app.ui.dialog = Some(d);
        let h = harness(app);
        assert!(h.query_by_role_and_label(Role::ComboBox, "Legend:").is_some());
    }

    /// Override Calculated Values starts from the tick values the axis is drawn with, OK sends
    /// them, and turning it off again gives the axis its calculated values back. Left off, it
    /// leaves a count the graph has alone.
    #[test]
    fn override_calculated_values_starts_from_the_calculated_tick_values() {
        let mut app = app_with_graph("column");
        open(&mut app).unwrap();
        assert_eq!(calculated(app.ui.dialog.as_ref().unwrap(), &LEFT), (0.0, 4.0, 4));
        set(&mut app, PAGE, json!("valueAxis"));
        let mut h = harness(app);
        let dimmed = |h: &Harness, name: &str| h.get_by_role_and_label(Role::TextInput, name).accesskit_node().is_disabled();
        assert!(dimmed(&h, "Minimum:") && dimmed(&h, "Maximum:") && dimmed(&h, "Divisions:"));
        h.get_by_role_and_label(Role::CheckBox, "Override Calculated Values").click();
        h.run_steps(3);
        assert!(!dimmed(&h, "Minimum:") && !dimmed(&h, "Maximum:") && !dimmed(&h, "Divisions:"));
        let app = h.state_mut();
        let d = app.ui.dialog.as_ref().unwrap();
        assert_eq!((number(d, "axisMin"), number(d, "axisMax"), number(d, "ticks")), (Some(0.0), Some(4.0), Some(4.0)));
        let before = steps(app);
        crate::dialogs::confirm(app).unwrap();
        let q = query(app);
        assert_eq!((q["axisMin"].as_f64(), q["axisMax"].as_f64(), q["ticks"].as_u64()), (Some(0.0), Some(4.0), Some(4)));
        assert_eq!(steps(app), before + 1);
        // It opens on, with the graph's values, and off it sends the calculated values back.
        open(app).unwrap();
        assert!(app.ui.dialog.as_ref().unwrap().bool("overrideValues"));
        set(app, "axisMax", json!(8.0));
        crate::dialogs::confirm(app).unwrap();
        assert_eq!(query(app)["axisMax"], 8.0);
        open(app).unwrap();
        set(app, "overrideValues", json!(false));
        crate::dialogs::confirm(app).unwrap();
        let q = query(app);
        assert_eq!((q["axisMin"].clone(), q["axisMax"].clone(), q["ticks"].clone()), (Value::Null, Value::Null, json!(0)));
        // Left off, a count the graph has stays, whatever else OK changes.
        app.run(COMMAND, json!({"ticks": 7})).unwrap();
        open(app).unwrap();
        set(app, "dropShadow", json!(true));
        crate::dialogs::confirm(app).unwrap();
        let q = query(app);
        assert_eq!((q["ticks"].clone(), q["dropShadow"].clone(), q["axisMin"].clone()), (json!(7), json!(true), Value::Null));
    }

    /// An overridden value axis whose maximum is not more than its minimum is refused: the dialog
    /// stays open and the graph is not changed. The right axis counts while it has a scale of its
    /// own.
    #[test]
    fn an_overridden_axis_needs_a_maximum_above_its_minimum() {
        let mut app = app_with_graph("column");
        open(&mut app).unwrap();
        for (key, value) in [("overrideValues", json!(true)), ("axisMin", json!(10)), ("axisMax", json!(10))] {
            set(&mut app, key, value);
        }
        let before = steps(&app);
        assert_eq!(problem(app.ui.dialog.as_ref().unwrap()), Some("Maximum must be more than Minimum."));
        assert!(crate::dialogs::confirm(&mut app).is_err());
        assert!(app.ui.dialog.is_some(), "the dialog stays open");
        assert_eq!((steps(&app), query(&mut app)["axisMax"].clone()), (before, Value::Null));
        // A range too wide to divide is refused too.
        set(&mut app, "axisMin", json!(-1.7e308));
        set(&mut app, "axisMax", json!(1.7e308));
        assert!(problem(app.ui.dialog.as_ref().unwrap()).is_some());
        set(&mut app, "axisMin", json!(10));
        set(&mut app, "axisMax", json!(20));
        crate::dialogs::confirm(&mut app).unwrap();
        let q = query(&mut app);
        assert_eq!((q["axisMin"].as_f64(), q["axisMax"].as_f64()), (Some(10.0), Some(20.0)));
        // The right axis shows the left scale until Separate Scales gives it one.
        open(&mut app).unwrap();
        for (key, value) in [("rightOverrideValues", json!(true)), ("rightAxisMin", json!(5)), ("rightAxisMax", json!(1))] {
            set(&mut app, key, value);
        }
        assert_eq!(problem(app.ui.dialog.as_ref().unwrap()), None);
        set(&mut app, "valueAxis", json!("both"));
        set(&mut app, "separateScales", json!(true));
        // A scale without series on its side draws no axis of its own.
        assert_eq!(problem(app.ui.dialog.as_ref().unwrap()), None);
        set(&mut app, RIGHT_SERIES, json!([1]));
        assert!(problem(app.ui.dialog.as_ref().unwrap()).is_some());
        // A value that is not a number is not replaced by a calculated one: the command refuses it.
        open(&mut app).unwrap();
        set(&mut app, "axisMax", json!("ten"));
        settle(app.ui.dialog.as_mut().unwrap());
        assert_eq!(app.ui.dialog.as_ref().unwrap().fields["axisMax"], "ten");
        let before = steps(&app);
        assert!(crate::dialogs::confirm(&mut app).is_err());
        assert_eq!((steps(&app), query(&mut app)["axisMax"].as_f64()), (before, Some(20.0)));
        open(&mut app).unwrap();
        for (key, value) in [("rightOverrideValues", json!(true)), ("rightAxisMin", json!(5)), ("rightAxisMax", json!(1))] {
            set(&mut app, key, value);
        }
        set(&mut app, "valueAxis", json!("both"));
        set(&mut app, "separateScales", json!(true));
        set(&mut app, RIGHT_SERIES, json!([1]));
        // A pie graph has no value axis to refuse.
        set(&mut app, "type", json!("pie"));
        assert_eq!(problem(app.ui.dialog.as_ref().unwrap()), None);
    }

    /// OK with nothing changed closes the dialog without a step of history, on every page and
    /// for every type.
    #[test]
    fn ok_with_nothing_changed_adds_no_step_of_history() {
        for kind in GraphKind::ALL {
            let mut app = app_with_graph(kind.id());
            app.run(COMMAND, json!({"valueAxis": "both", "axisMin": 0, "axisMax": 10})).unwrap();
            let before = steps(&app);
            open(&mut app).unwrap();
            let mut h = harness(app);
            for page in ["valueAxis", "rightAxis", "categoryAxis", "options"] {
                set(h.state_mut(), PAGE, json!(page));
                h.run_steps(3);
            }
            let app = h.state_mut();
            assert_eq!(crate::dialogs::confirm(app), Ok(Value::Null), "{kind:?}");
            assert!(app.ui.dialog.is_none());
            assert_eq!(steps(app), before, "{kind:?}");
        }
    }

    /// With series picked by Group Selection, the type and the value axis go to those series: the
    /// types that cannot go in the graph are dimmed, Value Axis offers one side, and the other
    /// options still go to the whole graph.
    #[test]
    fn picked_series_take_the_type_and_the_value_axis() {
        let mut app = app_with_graph("column");
        let (graph, series) = {
            let st = app.session.active().unwrap();
            let graph = st.selection.objects[0];
            (graph, st.doc.node(graph).unwrap().children().unwrap().iter().find(|c| c.extras().series_index == Some(1)).unwrap().id)
        };
        app.run("select.set", json!({"ids": [series.0]})).unwrap();
        open(&mut app).unwrap();
        assert_eq!(picked(app.ui.dialog.as_ref().unwrap()), [1]);
        let mut h = harness(app);
        for name in ["Bar Graph", "Stacked Bar Graph", "Scatter Graph", "Pie Graph", "Radar Graph"] {
            assert!(h.get_by_role_and_label(Role::Button, name).accesskit_node().is_disabled(), "{name}");
            pick_type(&mut h, name);
            assert_eq!(h.state().ui.dialog.as_ref().unwrap().str("type"), "column", "{name}");
        }
        h.get_by_role_and_label(Role::ComboBox, "Value Axis:").click();
        h.run_steps(3);
        assert!(h.query_by_label("On Both Sides").is_none(), "a series goes on one axis");
        h.get_by_label("On Right Side").click();
        h.run_steps(3);
        pick_type(&mut h, "Line Graph");
        // The picked line series brings the line options beside the columns'.
        assert!(h.query_by_role_and_label(Role::CheckBox, "Mark Data Points").is_some());
        assert!(h.query_by_role_and_label(Role::TextInput, "Column Width:").is_some());
        let app = h.state_mut();
        set(app, "columnWidth", json!(50.0));
        crate::dialogs::confirm(app).unwrap();
        // The series stays picked, on its own type and axis.
        let q = query(app);
        assert_eq!((q["type"].clone(), q["valueAxis"].clone(), q["columnWidth"].clone()), (json!("line"), json!("right"), json!(50.0)));
        let whole = app.session.execute(COMMAND, &json!({"id": graph.0})).unwrap();
        assert_eq!((whole["type"].clone(), whole["valueAxis"].clone()), (json!("column"), json!("left")));
        // Series on both value axes open with no side, and OK leaves each where it is.
        let both: Vec<u64> = {
            let st = app.session.active().unwrap();
            st.doc.node(graph).unwrap().children().unwrap().iter().filter(|c| c.extras().series_index.is_some()).map(|c| c.id.0).collect()
        };
        app.run("select.set", json!({"ids": both})).unwrap();
        open(app).unwrap();
        let d = app.ui.dialog.as_ref().unwrap();
        assert_eq!((d.fields["valueAxis"].clone(), d.fields["type"].clone()), (Value::Null, Value::Null));
        assert_eq!(series_kinds(d), [GraphKind::Column, GraphKind::Line]);
        set(app, "dropShadow", json!(true));
        crate::dialogs::confirm(app).unwrap();
        let st = app.session.active().unwrap();
        let spec = st.doc.node(graph).unwrap().extras().graph.as_deref().unwrap();
        assert_eq!((spec.right_series.clone(), spec.drop_shadow), (vec![1], true));
        assert_eq!(spec.series_kinds, [None, Some(GraphKind::Line)], "each series keeps its type");
    }

    /// `ui.dialog.set` takes the page shown and refuses the fields that hold what OK acts on.
    #[test]
    fn the_page_is_a_control_and_the_subject_is_protected() {
        let mut app = app_with_graph("line");
        open(&mut app).unwrap();
        let d = app.ui.dialog.as_ref().unwrap();
        assert!(crate::dialogs::accepts_field(d, PAGE) && crate::dialogs::accepts_field(d, "overrideValues"));
        for field in
            [GRAPH_ID, SERIES, GRAPH, GRAPH_AXIS, SERIES_KINDS, RIGHT_SERIES, CALCULATED, CALCULATED_FOR, OPENED, "__override", "__rightOverride"]
        {
            assert!(crate::dialogs::protected_field(d, field), "{field}");
        }
        assert!(crate::dialogs::protected_field(d, DOC));
    }

    /// A value axis side without a scale of its own shows the tick values of the scale drawn on it,
    /// not bounds kept for it.
    #[test]
    fn a_side_without_its_own_scale_shows_the_scale_drawn_on_it() {
        let mut app = app_with_graph("column");
        app.run(COMMAND, json!({"valueAxis": "both", "rightAxisMin": 100, "rightAxisMax": 200})).unwrap();
        open(&mut app).unwrap();
        assert!(app.ui.dialog.as_ref().unwrap().bool("rightOverrideValues"));
        set(&mut app, PAGE, json!("rightAxis"));
        let mut h = harness(app);
        let shown = |h: &Harness, name: &str| {
            let node = h.get_by_role_and_label(Role::TextInput, name);
            node.accesskit_node().value().as_deref().and_then(|v| v.trim().parse::<f64>().ok())
        };
        assert_eq!((shown(&h, "Minimum:"), shown(&h, "Maximum:")), (Some(0.0), Some(4.0)), "the left scale, calculated");
        let overridden = h.get_by_role_and_label(Role::CheckBox, "Override Calculated Values");
        assert!(overridden.accesskit_node().is_disabled());
        // With Separate Scales and a series on the right axis, the side shows its own bounds.
        set(h.state_mut(), "separateScales", json!(true));
        set(h.state_mut(), RIGHT_SERIES, json!([1]));
        h.run_steps(3);
        assert_eq!((shown(&h, "Minimum:"), shown(&h, "Maximum:")), (Some(100.0), Some(200.0)));
    }

    /// OK changes the graph and the series the dialog opened on, even when the selection changed
    /// while it was open.
    #[test]
    fn ok_changes_the_graph_the_dialog_opened_on() {
        let mut app = app_with_graph("column");
        let first = app.session.active().unwrap().selection.objects[0];
        app.run("graph.create", json!({"type": "column", "x": 0, "y": 200, "width": 200, "height": 150, "csv": ",a,b\nQ1,1,2\nQ2,3,4"})).unwrap();
        let second = app.session.active().unwrap().selection.objects[0];
        app.run("select.set", json!({"ids": [first.0]})).unwrap();
        open(&mut app).unwrap();
        set(&mut app, "type", json!("line"));
        app.run("select.set", json!({"ids": [second.0]})).unwrap();
        crate::dialogs::confirm(&mut app).unwrap();
        let kind =
            |app: &mut VectorcraftApp, id: vectorcraft_doc::NodeId| app.session.execute(COMMAND, &json!({"id": id.0})).unwrap()["type"].clone();
        assert_eq!((kind(&mut app, first), kind(&mut app, second)), (json!("line"), json!("column")));
        // Picked series keep the type picked for them when the selection moves to the whole graph.
        let series = {
            let st = app.session.active().unwrap();
            st.doc.node(first).unwrap().children().unwrap().iter().find(|c| c.extras().series_index == Some(1)).unwrap().id
        };
        app.run("select.set", json!({"ids": [series.0]})).unwrap();
        open(&mut app).unwrap();
        set(&mut app, "type", json!("area"));
        app.run("select.set", json!({"ids": [first.0]})).unwrap();
        crate::dialogs::confirm(&mut app).unwrap();
        let st = app.session.active().unwrap();
        let spec = st.doc.node(first).unwrap().extras().graph.as_deref().unwrap();
        assert_eq!((spec.kind, spec.series_kind(0), spec.series_kind(1)), (GraphKind::Line, GraphKind::Line, GraphKind::Area));
    }

    /// Override Calculated Values starts from the tick values of the type picked in the dialog, not
    /// of the type the graph had when it opened.
    #[test]
    fn override_starts_from_the_tick_values_of_the_type_picked() {
        let mut app = app_with_graph("column");
        open(&mut app).unwrap();
        assert_eq!(calculated(app.ui.dialog.as_ref().unwrap(), &LEFT).1, 4.0);
        // Stacked, Q2 reaches 3 + 4 = 7.
        set(&mut app, "type", json!("stackedColumn"));
        set(&mut app, "overrideValues", json!(true));
        crate::dialogs::confirm(&mut app).unwrap();
        let q = query(&mut app);
        assert_eq!(q["type"], "stackedColumn");
        assert!(q["axisMax"].as_f64().is_some_and(|max| max >= 7.0), "{}", q["axisMax"]);
        let st = app.session.active().unwrap();
        let spec = st.doc.node(st.selection.objects[0]).unwrap().extras().graph.as_deref().unwrap();
        // The calculated values do not depend on the override the graph now has.
        let stacked = vectorcraft_engine::graph_calculated_axes(spec)[0];
        let sent = |q: &Value| (q["axisMin"].as_f64(), q["axisMax"].as_f64(), q["ticks"].as_u64());
        assert_eq!(sent(&q), (Some(stacked.0), Some(stacked.1), Some(stacked.2 as u64)));
        // Turned on first, the values filled for the column graph follow the type picked after it.
        let mut app = app_with_graph("column");
        open(&mut app).unwrap();
        set(&mut app, "overrideValues", json!(true));
        settle(app.ui.dialog.as_mut().unwrap());
        assert_eq!(number(app.ui.dialog.as_ref().unwrap(), "axisMax"), Some(4.0));
        set(&mut app, "type", json!("stackedColumn"));
        crate::dialogs::confirm(&mut app).unwrap();
        assert_eq!(sent(&query(&mut app)), (Some(stacked.0), Some(stacked.1), Some(stacked.2 as u64)));
        // Values typed in stay.
        open(&mut app).unwrap();
        set(&mut app, "axisMax", json!(50.0));
        set(&mut app, "type", json!("column"));
        crate::dialogs::confirm(&mut app).unwrap();
        assert_eq!(query(&mut app)["axisMax"], 50.0);
    }

    /// The limits the fields keep to are the ones `graph.setType` keeps to.
    #[test]
    fn the_fields_keep_to_the_command_s_limits() {
        let mut app = app_with_graph("column");
        let long = "x".repeat(AFFIX_CHARS + 6);
        app.run(COMMAND, json!({"tickMarks": 99, "columnWidth": 5000, "clusterWidth": 0, "prefix": long, "axisMin": 0, "axisMax": 1, "ticks": 500}))
            .unwrap();
        let q = query(&mut app);
        assert_eq!(q["tickMarks"].as_f64(), Some(*TICK_MARKS.end()));
        assert_eq!((q["columnWidth"].as_f64(), q["clusterWidth"].as_f64()), (Some(*WIDTHS.end()), Some(*WIDTHS.start())));
        assert_eq!(q["prefix"].as_str().map(str::len), Some(AFFIX_CHARS));
        assert_eq!(q["ticks"].as_f64(), Some(*DIVISIONS.end()));
    }

    /// With Override Calculated Values turned off, the dimmed fields show the calculated tick
    /// values OK gives the axis, not values typed while it was on.
    #[test]
    fn the_dimmed_fields_show_the_calculated_tick_values() {
        let mut app = app_with_graph("column");
        open(&mut app).unwrap();
        set(&mut app, PAGE, json!("valueAxis"));
        set(&mut app, "overrideValues", json!(true));
        set(&mut app, "axisMax", json!(50.0));
        set(&mut app, "ticks", json!(10));
        set(&mut app, "overrideValues", json!(false));
        let h = harness(app);
        let shown = |name: &str| {
            let node = h.get_by_role_and_label(Role::TextInput, name);
            node.accesskit_node().value().as_deref().and_then(|v| v.trim().parse::<f64>().ok())
        };
        assert_eq!((shown("Minimum:"), shown("Maximum:"), shown("Divisions:")), (Some(0.0), Some(4.0), Some(4.0)));
    }

    /// The tick values are calculated again when the graph's data changes while the dialog is open.
    #[test]
    fn a_data_change_calculates_the_tick_values_again() {
        let mut app = app_with_graph("column");
        open(&mut app).unwrap();
        assert_eq!(calculated(app.ui.dialog.as_ref().unwrap(), &LEFT).1, 4.0);
        app.session.execute("graph.setData", &json!({"csv": ",a,b\nQ1,1,2\nQ2,3,40"})).unwrap();
        let mut d = app.ui.dialog.clone().unwrap();
        recalculate(&app, &mut d);
        assert!(calculated(&d, &LEFT).1 >= 40.0, "{:?}", calculated(&d, &LEFT));
        // Turned on now, Override Calculated Values starts from the new values.
        d.fields.insert("overrideValues".into(), json!(true));
        app.ui.dialog = Some(d);
        crate::dialogs::confirm(&mut app).unwrap();
        assert!(query(&mut app)["axisMax"].as_f64().is_some_and(|max| max >= 40.0));
        // Tick values the graph has keep, even when they are the calculated ones.
        let mut app = app_with_graph("column");
        app.run(COMMAND, json!({"axisMin": 0, "axisMax": 4, "ticks": 4})).unwrap();
        open(&mut app).unwrap();
        app.session.execute("graph.setData", &json!({"csv": ",a,b\nQ1,1,2\nQ2,3,40"})).unwrap();
        let mut d = app.ui.dialog.clone().unwrap();
        recalculate(&app, &mut d);
        settle(&mut d);
        app.ui.dialog = Some(d);
        let before = steps(&app);
        assert_eq!(crate::dialogs::confirm(&mut app), Ok(Value::Null));
        assert_eq!((steps(&app), query(&mut app)["axisMax"].as_f64()), (before, Some(4.0)));
        // A value typed in stays, and the values filled beside it are calculated again.
        let mut app = app_with_graph("column");
        open(&mut app).unwrap();
        set(&mut app, "overrideValues", json!(true));
        settle(app.ui.dialog.as_mut().unwrap());
        set(&mut app, "axisMax", json!(50.0));
        app.session.execute("graph.setData", &json!({"csv": ",a,b\nQ1,-30,2\nQ2,3,4"})).unwrap();
        let mut d = app.ui.dialog.clone().unwrap();
        recalculate(&app, &mut d);
        let (lo, _, divisions) = calculated(&d, &LEFT);
        assert!(lo < 0.0, "{lo}");
        app.ui.dialog = Some(d);
        crate::dialogs::confirm(&mut app).unwrap();
        let q = query(&mut app);
        assert_eq!((q["axisMin"].as_f64(), q["axisMax"].as_f64(), q["ticks"].as_u64()), (Some(lo), Some(50.0), Some(divisions as u64)));
        // With both bounds typed in, the divisions filled are calculated again, not the count an
        // axis without one is drawn in.
        let mut app = app_with_graph("column");
        open(&mut app).unwrap();
        set(&mut app, "overrideValues", json!(true));
        settle(app.ui.dialog.as_mut().unwrap());
        set(&mut app, "axisMin", json!(-100.0));
        set(&mut app, "axisMax", json!(100.0));
        app.session.execute("graph.setData", &json!({"csv": ",a,b\nQ1,-30,2\nQ2,3,4"})).unwrap();
        let mut d = app.ui.dialog.clone().unwrap();
        recalculate(&app, &mut d);
        settle(&mut d);
        assert_eq!(number(&d, "ticks"), Some(calculated(&d, &LEFT).2 as f64));
        // Each value the graph had keeps, beside one typed in.
        let mut app = app_with_graph("column");
        app.run(COMMAND, json!({"axisMin": 0, "axisMax": 4, "ticks": 4})).unwrap();
        open(&mut app).unwrap();
        set(&mut app, "axisMax", json!(50.0));
        app.session.execute("graph.setData", &json!({"csv": ",a,b\nQ1,-30,2\nQ2,3,4"})).unwrap();
        let mut d = app.ui.dialog.clone().unwrap();
        recalculate(&app, &mut d);
        app.ui.dialog = Some(d);
        crate::dialogs::confirm(&mut app).unwrap();
        let q = query(&mut app);
        assert_eq!((q["axisMin"].as_f64(), q["axisMax"].as_f64(), q["ticks"].as_u64()), (Some(0.0), Some(50.0), Some(4)));
    }

    /// OK changes nothing while another document is active, even one with a graph of the same id.
    #[test]
    fn ok_changes_nothing_in_another_document() {
        let mut app = app_with_graph("column");
        let first = app.session.active().unwrap().selection.objects[0];
        open(&mut app).unwrap();
        set(&mut app, "type", json!("line"));
        app.run("file.new", json!({"width": 400, "height": 400})).unwrap();
        app.run("graph.create", json!({"type": "column", "x": 0, "y": 0, "width": 200, "height": 150, "csv": ",a,b\nQ1,1,2\nQ2,3,4"})).unwrap();
        let second = app.session.active().unwrap().selection.objects[0];
        assert_eq!(first, second, "the same id in both documents");
        let before = steps(&app);
        assert_eq!(crate::dialogs::confirm(&mut app), Ok(Value::Null));
        assert!(app.ui.dialog.is_none());
        assert_eq!(steps(&app), before);
        assert_eq!(app.session.execute(COMMAND, &json!({"id": second.0})).unwrap()["type"], "column");
    }
}
