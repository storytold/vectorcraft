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
//! `page`, the page shown (`options`, `valueAxis`, `rightAxis` or `categoryAxis`).

use std::ops::RangeInclusive;

use craft::behavior::{ColumnWidth, Mixed};
use craft::theme::variant::Tone;
use craft::ui::choice::{Checkbox, Segmented};
use craft::ui::contract::{Bind, Entry, MemoProps, Props as _, Show as _, ShowContainer as _};
use craft::ui::dialog::{DialogId, DialogResult, DialogStack, FieldRefusal, HeldAnswer, HeldOk, StackDialog, dialog_id};
use craft::ui::form::Field;
use craft::ui::layout_components::Grid;
use craft::ui::notices::{Notice, NoticeLayout};
use craft::ui::patterns::form_dialogs::FormDialog;
use craft::ui::pickers::Select;
use craft::ui::primitives::{Glyph, IconSpec};
use craft::ui::runtime::env::Label;
use craft::ui::style::UiTokens;
use craft::ui::text::{NumberField, TextField};
use egui::{Context, Ui};
use serde_json::{Map, Value, json};
use vectorcraft_doc::GraphKind;

use crate::VectorcraftApp;

/// The dialog kind of Graph Type: its id on the host and the name automation reaches it by.
pub const KIND: &str = "graphType";

/// The field that holds the page shown.
const PAGE: &str = "page";

const COMMAND: &str = "graph.setType";

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

/// The fields of the legend, sent by type, and those of the value axes' tick values, sent while overridden.
const LEGEND: [&str; 4] = ["legend", "pieLegend", "piePosition", "pieSort"];
const TICK_VALUES: [&str; 8] = ["overrideValues", "rightOverrideValues", "axisMin", "axisMax", "ticks", "rightAxisMin", "rightAxisMax", "rightTicks"];

/// The fields of one value axis: its place in the calculated tick values, Override Calculated
/// Values, the tick values, the tick marks and the labels.
struct Axis {
    index: usize,
    overridden: &'static str,
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
    min: "rightAxisMin",
    max: "rightAxisMax",
    ticks: "rightTicks",
    length: "rightTickLength",
    marks: "rightTickMarks",
    prefix: "rightPrefix",
    suffix: "rightSuffix",
};

/// How a number row shows its number: the range it keeps to, its unit and its decimals.
struct Num {
    range: Option<RangeInclusive<f64>>,
    unit: Option<&'static str>,
    decimals: usize,
}

const PERCENT: Num = Num { range: Some(WIDTHS), unit: Some("%"), decimals: 2 };
const POINTS: Num = Num { range: Some(LINE_WIDTHS), unit: Some("pt"), decimals: 2 };
const BOUND: Num = Num { range: None, unit: None, decimals: 4 };
const DIVISION_COUNT: Num = Num { range: Some(DIVISIONS), unit: None, decimals: 0 };
const MARK_COUNT: Num = Num { range: Some(TICK_MARKS), unit: None, decimals: 0 };

/// The two columns of the checkbox grids.
const TWO: &[ColumnWidth] = &[ColumnWidth::fill(1.0); 2];

/// The dialog's values under the names `json_fields` and `set_json_field` use: the options of
/// `graph.setType`, Override Calculated Values of each value axis, and `page`.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct GraphTypeValues(Map<String, Value>);

impl GraphTypeValues {
    fn get(&self, key: &str) -> Option<&Value> {
        self.0.get(key)
    }

    fn bool(&self, key: &str) -> bool {
        self.get(key).and_then(Value::as_bool).unwrap_or(false)
    }

    fn str(&self, key: &str) -> &str {
        self.get(key).and_then(Value::as_str).unwrap_or_default()
    }

    fn set(&mut self, key: &str, value: Value) {
        self.0.insert(key.into(), value);
    }
}

/// What the dialog opened on and what it calculated, which no field reaches: the document and the
/// graph, the series picked with Group Selection, the graph's own type and value axis side, each
/// series' own type, the series on the right axis, the tick values each value axis is calculated
/// with and the options and document revision they were calculated for, the parameters OK would
/// have sent as the dialog opened, and whether each axis was overridden then.
#[derive(Clone)]
struct GraphSubject {
    doc: u64,
    reverts: u64,
    graph_id: u64,
    series: Vec<usize>,
    graph: GraphKind,
    graph_axis: String,
    series_kinds: Vec<Option<GraphKind>>,
    right_series: Vec<usize>,
    calculated: [(f64, f64, usize); 2],
    calculated_for: Value,
    opened: Value,
    opened_overridden: [bool; 2],
}

/// Graph Type on the dialog host. OK is held: the pump runs `graph.setType` and answers.
pub(crate) struct GraphTypeDialog {
    id: DialogId,
    values: GraphTypeValues,
    subject: GraphSubject,
    held: HeldAnswer<GraphTypeValues>,
}

impl StackDialog for GraphTypeDialog {
    type Result = ();

    fn id(&self) -> DialogId {
        self.id
    }

    fn key(&self) -> Option<&str> {
        Some(KIND)
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<()>> {
        let GraphTypeDialog { values, subject, held, .. } = self;
        let why = problem(values, subject);
        let mut dialog = FormDialog::new(tl!("Graph Type"), Bind::bound(values)).ok_enabled(why.is_none()).held(held);
        if let Some(why) = why {
            dialog = dialog.ok_reason(why);
        }
        let shown = dialog.show(ui, |ui, _, v| {
            body(ui, v, subject);
            true
        });
        match shown.closed {
            None => None,
            Some(_) if shown.accepted.is_some() => Some(DialogResult::Ok(())),
            Some(_) => Some(DialogResult::Cancel),
        }
    }

    /// OK as the button: files the values while they can be applied, and closes once the app accepted them.
    fn confirm(&mut self, _: &Context) -> Option<()> {
        if self.held.is_accepted() {
            return self.held.finish().map(drop);
        }
        if !self.held.is_waiting() && problem(&self.values, &self.subject).is_none() {
            self.held.file(self.values.clone());
        }
        None
    }

    fn held_state(&self) -> Option<HeldOk> {
        self.held.state()
    }

    fn json_fields(&self) -> Option<Value> {
        Some(Value::Object(self.values.0.clone()))
    }

    /// Sets a field as its control does: a value the control can't show is refused, and so is a
    /// field whose control is dimmed or hidden.
    fn set_json_field(&mut self, _: &Context, field: &str, value: Value) -> Result<(), String> {
        let (v, s) = (&mut self.values, &self.subject);
        if !(field == PAGE || SENT.contains(&field) || LEGEND.contains(&field) || TICK_VALUES.contains(&field)) {
            return Err(FieldRefusal::Unknown.message(field));
        }
        if !accepts(v, s, field, &value) {
            return Err(FieldRefusal::Rejects(&value).message(field));
        }
        let kind = value.as_str().and_then(GraphKind::parse);
        let combines = field != "type" || s.series.is_empty() || kind.is_some_and(|kind| kind.combines_with(s.graph));
        if !(active(v, s, field) && combines) {
            return Err(FieldRefusal::Inactive.message(field));
        }
        v.set(field, value);
        Ok(())
    }
}

/// Whether `field` can hold `value`: a page the type has, a value of a menu's list, a number in its
/// field's range, a flag, or a label within the characters the command keeps.
fn accepts(v: &GraphTypeValues, s: &GraphSubject, field: &str, value: &Value) -> bool {
    let listed = |list: &[(&str, &str)]| value.as_str().is_some_and(|value| list.iter().any(|(id, _)| *id == value));
    let within = |range: RangeInclusive<f64>| value.as_f64().is_some_and(|x| range.contains(&x));
    match field {
        PAGE => value.as_str().is_some_and(|page| pages(v, s).iter().any(|(id, _)| *id == page)),
        "type" => value.as_str().and_then(GraphKind::parse).is_some(),
        "valueAxis" => listed(if s.series.is_empty() { &SIDES } else { &SERIES_SIDES }),
        "tickLength" | "rightTickLength" | "categoryTickLength" => listed(&TICK_LENGTHS),
        "pieLegend" => listed(&PIE_LEGENDS),
        "piePosition" => listed(&PIE_POSITIONS),
        "pieSort" => listed(&PIE_SORTS),
        "columnWidth" | "clusterWidth" => within(WIDTHS),
        "lineWidth" => within(LINE_WIDTHS),
        "tickMarks" | "rightTickMarks" | "categoryTickMarks" => within(TICK_MARKS),
        "ticks" | "rightTicks" => within(DIVISIONS),
        "axisMin" | "axisMax" | "rightAxisMin" | "rightAxisMax" => value.as_f64().is_some_and(f64::is_finite),
        "prefix" | "suffix" | "rightPrefix" | "rightSuffix" => value.as_str().is_some_and(|text| text.chars().count() <= AFFIX_CHARS),
        _ => value.is_boolean(),
    }
}

/// Whether the control of `field` is shown and enabled, by the rules the pages draw them with.
fn active(v: &GraphTypeValues, s: &GraphSubject, field: &str) -> bool {
    let kind = graph_kind(v, s);
    let lines = draws(v, s, &LINES);
    let categories = CATEGORIES.contains(&kind);
    match field {
        PAGE | "type" | "dropShadow" => true,
        "valueAxis" => SIDED.contains(&kind),
        "separateScales" => both_sides(v, s) && SCALED.contains(&kind),
        "firstRowInFront" | "firstColumnInFront" | "columnWidth" | "clusterWidth" => draws(v, s, &COLUMNS),
        "legendAcrossTop" => legend_shown(v, s),
        "legend" => kind != GraphKind::Pie,
        "markPoints" | "connectPoints" => lines,
        "edgeToEdge" => lines && draws(v, s, &[GraphKind::Line]),
        "filledLines" => lines && v.bool("connectPoints"),
        "lineWidth" => lines && v.bool("connectPoints") && v.bool("filledLines"),
        "pieLegend" | "piePosition" | "pieSort" => kind == GraphKind::Pie,
        "categoryTickLength" => categories,
        "categoryTickMarks" | "ticksBetweenLabels" => categories && v.str("categoryTickLength") != "none",
        _ => axis_active(v, s, field),
    }
}

/// Whether a value axis page that is shown has the control of `field` enabled.
fn axis_active(v: &GraphTypeValues, s: &GraphSubject, field: &str) -> bool {
    let shown = pages(v, s);
    [(false, "valueAxis"), (true, "rightAxis")].into_iter().filter(|(_, page)| shown.iter().any(|(id, _)| id == page)).any(|(right, _)| {
        let (scale, side) = page_axes(v, s, right);
        let own = scale_used(v, s, right);
        (field == scale.overridden && own)
            || ([scale.min, scale.max, scale.ticks].contains(&field) && own && v.bool(scale.overridden))
            || [side.length, side.prefix, side.suffix].contains(&field)
            || (field == side.marks && v.str(side.length) != "none")
    })
}

/// Open Graph Type on the options of the selected graph, or of the series picked in it with Group
/// Selection. An error without a selected graph.
pub fn open(app: &mut VectorcraftApp) -> Result<Value, String> {
    let queried = app.session.execute(COMMAND, &json!({})).map_err(|e| e.to_string())?;
    let subject = vectorcraft_engine::graph_type_subject(&app.session).map_err(|e| e.to_string())?;
    let st = app.session.active().ok_or("no document open")?;
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
    let mut opened_overridden = [false; 2];
    for (axis, min, max) in [(&LEFT, spec.axis_min, spec.axis_max), (&RIGHT, spec.right_axis_min, spec.right_axis_max)] {
        opened_overridden[axis.index] = min.is_some() && max.is_some();
        fields.insert(axis.overridden.into(), json!(opened_overridden[axis.index]));
    }
    fields.insert(PAGE.into(), json!("options"));
    let mut s = GraphSubject {
        doc: st.uid,
        reverts: st.reverts,
        graph_id: subject.id.0,
        series: subject.series,
        graph: spec.kind,
        graph_axis: spec.value_axis.id().to_string(),
        series_kinds: (0..count).map(|i| spec.series_kinds.get(i).copied().flatten()).collect(),
        right_series: spec.right_series.iter().copied().filter(|i| *i < count).collect(),
        calculated: subject.calculated,
        calculated_for: Value::Null,
        opened: Value::Null,
        opened_overridden,
    };
    let mut v = GraphTypeValues(fields);
    s.calculated_for = calculated_for(app, &v, &s);
    settle(&mut v, &s);
    s.opened = params(&v, &s);
    let dialogs = app.dialogs().ok_or("no window")?;
    dialogs.with(|host| host.open(GraphTypeDialog { id: dialog_id(KIND), values: v, subject: s, held: HeldAnswer::new() }));
    Ok(json!({ "dialog": KIND }))
}

/// Runs once a frame before the dialog's pass: calculates the tick values again for the options
/// and the document as they are now, and answers a filed OK with `graph.setType`.
pub(crate) fn pump(app: &mut VectorcraftApp, _: &Context) {
    let Some(dialogs) = app.dialogs() else { return };
    let id = dialog_id(KIND);
    let filed = dialogs.with(|host| {
        let dialog = host.get_mut_by_id::<GraphTypeDialog>(id)?;
        recalculate(app, dialog);
        settle(&mut dialog.values, &dialog.subject);
        dialog.held.take().map(|_| verdict(app, &dialog.values, &dialog.subject))
    });
    let answer = match filed {
        None => return,
        Some(Err(why)) => Err(Label::from(why)),
        Some(Ok(None)) => Ok(()),
        Some(Ok(Some(p))) => app.run(COMMAND, p).map(drop).map_err(Label::from),
    };
    dialogs.with(|host| {
        if let Some(dialog) = host.get_mut_by_id::<GraphTypeDialog>(id) {
            dialog.held.answer(answer);
        }
    });
}

/// What OK does with the dialog as it is: nothing while another document is active (its graphs
/// have ids of their own) or with nothing changed, else `graph.setType` with these parameters.
/// `Err` while the dialog can't be applied.
fn verdict(app: &VectorcraftApp, v: &GraphTypeValues, s: &GraphSubject) -> Result<Option<Value>, &'static str> {
    let Some(st) = app.session.active().filter(|st| st.uid == s.doc) else {
        return Ok(None);
    };
    if st.reverts != s.reverts {
        return Err(tl!("The document was reverted. Reopen this dialog to continue."));
    }
    if let Some(why) = problem(v, s) {
        return Err(why);
    }
    let mut p = params(v, s);
    if same(&s.opened, &p) {
        return Ok(None);
    }
    aim(app, s, &mut p);
    Ok(Some(p))
}

/// The type the Type buttons show.
fn chosen(v: &GraphTypeValues) -> Option<GraphKind> {
    v.get("type").and_then(Value::as_str).and_then(GraphKind::parse)
}

/// The graph's own type as OK leaves it: the type picked, or with series picked the type the graph
/// has.
fn graph_kind(v: &GraphTypeValues, s: &GraphSubject) -> GraphKind {
    if s.series.is_empty() { chosen(v).unwrap_or(s.graph) } else { s.graph }
}

/// The type each series draws as once OK is pressed: the type picked for the picked series (their
/// own while they differ and none is picked), its own type where that combines with the graph's,
/// else the graph's.
fn series_kinds(v: &GraphTypeValues, s: &GraphSubject) -> Vec<GraphKind> {
    let (graph, chosen) = (graph_kind(v, s), chosen(v));
    (0..s.series_kinds.len().max(1))
        .map(|i| {
            let kept = s.series_kinds.get(i).copied().flatten();
            let kind = if s.series.contains(&i) { chosen.or(kept) } else { kept };
            kind.filter(|k| k.combines_with(graph)).unwrap_or(graph)
        })
        .collect()
}

/// Whether a series draws as one of `kinds` once OK is pressed.
fn draws(v: &GraphTypeValues, s: &GraphSubject, kinds: &[GraphKind]) -> bool {
    series_kinds(v, s).iter().any(|k| kinds.contains(k))
}

/// The side or sides the graph's value axis is on once OK is pressed. With series picked, Value
/// Axis is theirs and the graph keeps its own.
fn graph_axis<'a>(v: &'a GraphTypeValues, s: &'a GraphSubject) -> &'a str {
    if s.series.is_empty() { v.str("valueAxis") } else { &s.graph_axis }
}

/// Whether the graph draws a value axis on both sides.
fn both_sides(v: &GraphTypeValues, s: &GraphSubject) -> bool {
    SIDED.contains(&graph_kind(v, s)) && graph_axis(v, s) == "both"
}

/// Whether the right value axis has a scale of its own: Separate Scales with the value axis on
/// both sides of a type that takes one.
fn separate(v: &GraphTypeValues, s: &GraphSubject) -> bool {
    v.bool("separateScales") && both_sides(v, s) && SCALED.contains(&graph_kind(v, s))
}

/// Whether the graph draws a legend beside or above the plot.
fn legend_shown(v: &GraphTypeValues, s: &GraphSubject) -> bool {
    if graph_kind(v, s) == GraphKind::Pie { v.str("pieLegend") == "standard" } else { v.bool("legend") }
}

/// The pages the menu lists for the type picked, as (id, name): Graph Options, a page per value
/// axis side (none for a pie graph) and Category Axis for the types that have one.
fn pages(v: &GraphTypeValues, s: &GraphSubject) -> Vec<(&'static str, &'static str)> {
    let kind = graph_kind(v, s);
    let mut pages = vec![("options", tl!("Graph Options"))];
    if kind != GraphKind::Pie {
        if both_sides(v, s) {
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

/// The scale a value axis page shows the tick values of, and the side it shows the tick marks and
/// labels of. An only value axis on the right side shows the left scale with the right side's tick
/// marks and labels, as the graph draws it.
fn page_axes(v: &GraphTypeValues, s: &GraphSubject, right: bool) -> (&'static Axis, &'static Axis) {
    let scale = if right { &RIGHT } else { &LEFT };
    let side = if right || (SIDED.contains(&graph_kind(v, s)) && graph_axis(v, s) == "right") { &RIGHT } else { &LEFT };
    (scale, side)
}

/// The least value, the greatest and the divisions `axis` is drawn with while its tick values are
/// calculated.
fn calculated(s: &GraphSubject, axis: &Axis) -> (f64, f64, usize) {
    let (lo, hi, count) = s.calculated[axis.index];
    let finite = |x: f64, or: f64| if x.is_finite() { x } else { or };
    let divisions = if count >= 1 { count.min(*DIVISIONS.end() as usize) } else { DEFAULT_DIVISIONS };
    (finite(lo, 0.0), finite(hi, 1.0), divisions)
}

/// The series on the right value axis once OK is pressed: the graph's, with the picked series moved
/// to the side picked for them.
fn right_series(v: &GraphTypeValues, s: &GraphSubject) -> Vec<usize> {
    let mut right = s.right_series.clone();
    match v.get("valueAxis").and_then(Value::as_str) {
        Some("right") => right.extend(s.series.iter().copied().filter(|i| !s.right_series.contains(i))),
        Some("left") => right.retain(|i| !s.series.contains(i)),
        _ => {}
    }
    right
}

/// Whether the right axis' own scale (`right`) or the left one draws a value axis: the left scale
/// draws both sides without Separate Scales, and with it each scale needs a series on its side (the
/// other side's scale draws both sides when one side has none).
fn scale_used(v: &GraphTypeValues, s: &GraphSubject, right: bool) -> bool {
    if !separate(v, s) {
        return !right;
    }
    let on_right = right_series(v, s);
    let count = series_kinds(v, s).len();
    (0..count).any(|i| on_right.contains(&i) == right)
}

/// The options the calculated tick values depend on, as the dialog shows them: the graph's type,
/// each series' type, the value axis side, Separate Scales, and the side picked series go on.
fn pending(v: &GraphTypeValues, s: &GraphSubject) -> Value {
    let kinds: Vec<&str> = series_kinds(v, s).into_iter().map(GraphKind::id).collect();
    let side = v.get("valueAxis").cloned().unwrap_or(Value::Null);
    json!([graph_kind(v, s).id(), kinds, graph_axis(v, s), v.bool("separateScales"), side])
}

/// Whether the active document is the one the dialog opened on.
fn on_its_document(app: &VectorcraftApp, s: &GraphSubject) -> bool {
    app.session.active().is_some_and(|st| (st.uid, st.reverts) == (s.doc, s.reverts))
}

/// What the calculated tick values were calculated for: the options they depend on and the
/// revision of the active document, which an edit of the graph's data changes.
fn calculated_for(app: &VectorcraftApp, v: &GraphTypeValues, s: &GraphSubject) -> Value {
    json!([pending(v, s), app.session.active().map(|st| st.revision)])
}

/// Calculate the tick values again when the options they depend on or the document changed since
/// they were last calculated: the graph with the type, the value axis side and Separate Scales the
/// dialog shows, and the picked series on the type and side picked for them. Nothing while another
/// document is active.
fn recalculate(app: &VectorcraftApp, d: &mut GraphTypeDialog) {
    let GraphTypeDialog { values: v, subject: s, .. } = d;
    if !on_its_document(app, s) {
        return;
    }
    let key = calculated_for(app, v, s);
    if s.calculated_for == key {
        return;
    }
    let id = vectorcraft_doc::NodeId(s.graph_id);
    let Some(mut spec) = app.session.active().and_then(|st| st.doc.node(id)?.extras().graph.as_deref().cloned()) else {
        return;
    };
    let graph = graph_kind(v, s);
    spec.series_kinds = series_kinds(v, s).into_iter().map(|kind| (kind != graph).then_some(kind)).collect();
    spec.kind = graph;
    if let Some(side) = vectorcraft_doc::ValueAxisSide::parse(graph_axis(v, s)) {
        spec.value_axis = side;
    }
    spec.separate_scales = v.bool("separateScales");
    spec.right_series = right_series(v, s);
    let before = std::mem::replace(&mut s.calculated, vectorcraft_engine::graph_calculated_axes(&spec));
    s.calculated_for = key;
    // Each tick value of an overridden axis that still holds the value calculated before takes the
    // new one, unless it is the value the graph had when the dialog opened.
    for axis in [&LEFT, &RIGHT] {
        if !v.bool(axis.overridden) {
            continue;
        }
        let (lo, hi, count) = before[axis.index];
        let was = [lo, hi, count as f64];
        let saved = |key: &str| s.opened.get(key)?.as_f64().filter(|_| s.opened_overridden[axis.index]);
        let (lo, hi, divisions) = calculated(s, axis);
        let replaced: Vec<(&str, Value)> = [(axis.min, json!(lo)), (axis.max, json!(hi)), (axis.ticks, json!(divisions))]
            .into_iter()
            .zip(was)
            .filter(|((key, _), was)| number(v, key).is_some_and(|x| x == *was && Some(x) != saved(key)))
            .map(|(field, _)| field)
            .collect();
        for (key, value) in replaced {
            v.set(key, value);
        }
    }
}

/// Field `key` as a number. `None` when it is missing, null or not a number.
fn number(v: &GraphTypeValues, key: &str) -> Option<f64> {
    v.get(key).and_then(Value::as_f64)
}

/// The least and greatest value `axis` shows: its own, else the calculated ones.
fn bounds(v: &GraphTypeValues, s: &GraphSubject, axis: &Axis) -> (f64, f64) {
    let (lo, hi, _) = calculated(s, axis);
    (number(v, axis.min).unwrap_or(lo), number(v, axis.max).unwrap_or(hi))
}

/// The divisions `axis` shows: its own count, else the count it is drawn in.
fn divisions(v: &GraphTypeValues, s: &GraphSubject, axis: &Axis) -> usize {
    let own = number(v, axis.ticks).filter(|n| *n >= 1.0).map(|n| n.min(*DIVISIONS.end()) as usize);
    let both = number(v, axis.min).is_some() && number(v, axis.max).is_some();
    own.unwrap_or(if both { DEFAULT_DIVISIONS } else { calculated(s, axis).2 })
}

/// Give each axis whose Override Calculated Values is on the tick values it lacks: the calculated
/// ones, which the fields then show and OK sends.
fn settle(v: &mut GraphTypeValues, s: &GraphSubject) {
    for axis in [&LEFT, &RIGHT] {
        if !v.bool(axis.overridden) {
            continue;
        }
        let ((lo, hi), count) = (bounds(v, s, axis), divisions(v, s, axis));
        let missing = |v: &GraphTypeValues, key: &str| v.get(key).is_none_or(Value::is_null);
        for (key, value) in [(axis.min, json!(lo)), (axis.max, json!(hi))] {
            if missing(v, key) {
                v.set(key, value);
            }
        }
        if missing(v, axis.ticks) || number(v, axis.ticks).is_some_and(|n| n < 1.0) {
            v.set(axis.ticks, json!(count));
        }
    }
}

/// Why OK can't apply the dialog: an overridden value axis whose maximum is not more than its
/// minimum, which `graph.setType` would draw with calculated values instead.
fn problem(v: &GraphTypeValues, s: &GraphSubject) -> Option<&'static str> {
    let out_of_order = |axis: &Axis| {
        let (lo, hi) = bounds(v, s, axis);
        v.bool(axis.overridden) && !(hi > lo && (hi - lo).is_finite())
    };
    let refused =
        graph_kind(v, s) != GraphKind::Pie && [&LEFT, &RIGHT].into_iter().any(|axis| scale_used(v, s, axis.index == 1) && out_of_order(axis));
    refused.then(|| tl!("Maximum must be more than Minimum."))
}

/// `graph.setType`'s parameters for the dialog as it is. The type and the value axis go to the
/// series picked with Group Selection, as the command reads the selection.
fn params(v: &GraphTypeValues, s: &GraphSubject) -> Value {
    let mut v = v.clone();
    settle(&mut v, s);
    let mut p = Map::new();
    let send = |p: &mut Map<String, Value>, key: &str| {
        if let Some(value) = v.get(key).filter(|value| !value.is_null()) {
            p.insert(key.into(), value.clone());
        }
    };
    for key in SENT {
        send(&mut p, key);
    }
    // A pie graph's Legend menu stands for the legend checkbox.
    let legend: &[&str] = if graph_kind(&v, s) == GraphKind::Pie { &["pieLegend", "piePosition", "pieSort"] } else { &["legend"] };
    for key in legend {
        send(&mut p, key);
    }
    for axis in [&LEFT, &RIGHT] {
        if v.bool(axis.overridden) {
            for key in [axis.min, axis.max, axis.ticks] {
                send(&mut p, key);
            }
        } else if s.opened_overridden[axis.index] {
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

/// Point `p` at the graph and the series the dialog opened on when the selection no longer holds
/// them: `id`, and `seriesIndexes` while series are picked and a type or a value axis goes to them.
/// While the selection still holds them, the command reads it and keeps picked series selected.
fn aim(app: &VectorcraftApp, s: &GraphSubject, p: &mut Value) {
    let now = vectorcraft_engine::graph_type_subject(&app.session).ok();
    if now.is_some_and(|now| now.id.0 == s.graph_id && now.series == s.series) {
        return;
    }
    let Some(p) = p.as_object_mut() else {
        return;
    };
    p.insert("id".into(), json!(s.graph_id));
    // Picked series of different types on both axes send neither, and the other options go to the
    // whole graph.
    if !s.series.is_empty() && (p.contains_key("type") || p.contains_key("valueAxis")) {
        p.insert("seriesIndexes".into(), json!(s.series));
    }
}

/// The page menu, the page picked and the reason OK is off.
fn body(ui: &mut Ui, v: &mut GraphTypeValues, s: &GraphSubject) {
    let pages = pages(v, s);
    // A page the type picked lacks shows Graph Options.
    let page = pages.iter().map(|(id, _)| *id).find(|id| *id == v.str(PAGE)).unwrap_or("options");
    let entries: Vec<Entry<&'static str>> = pages.iter().map(|&(id, name)| Entry::new(id, name)).collect();
    if let Some(id) = Select::new(tl!("Graph Type"), Bind::given(page), &entries).show(ui).events.committed().copied() {
        v.set(PAGE, json!(id));
    }
    match page {
        "valueAxis" => value_page(ui, v, s, false),
        "rightAxis" => value_page(ui, v, s, true),
        "categoryAxis" => category_page(ui, v, s),
        _ => options_page(ui, v, s),
    }
    if let Some(why) = problem(v, s) {
        Notice::new(Tone::Bad, why).layout(NoticeLayout::Inline).show(ui);
    }
}

/// A checkbox over the flag `key`.
fn check(ui: &mut Ui, v: &mut GraphTypeValues, key: &str, label: &'static str, enabled: bool) {
    if let Some(on) = Checkbox::new(Bind::given(v.bool(key))).label(label).enabled(enabled).show(ui).events.committed().copied() {
        v.set(key, json!(on));
    }
}

/// A menu row over the string `key`; `options` are (value, label). A value that is not among
/// them, such as the null of series on both value axes, shows a blank field.
fn choice(ui: &mut Ui, v: &mut GraphTypeValues, key: &str, label: &'static str, options: &[(&'static str, &'static str)], enabled: bool) {
    let entries: Vec<Entry<&'static str>> = options.iter().map(|&(value, name)| Entry::new(value, tl!(name))).collect();
    let current = options.iter().map(|(value, _)| *value).find(|value| *value == v.str(key));
    let shown = current.map_or(Mixed::Mixed, Mixed::Same);
    let mut picked = None;
    Field::new(label)
        .show(ui, |ui, _| picked = Select::new(label, Bind::given(shown), &entries).enabled(enabled).show(ui).events.committed().copied());
    if let Some(value) = picked {
        v.set(key, json!(value));
    }
}

/// A number row over `key`, showing `shown`.
fn number_row(ui: &mut Ui, v: &mut GraphTypeValues, key: &str, label: &'static str, shown: f64, num: &Num, enabled: bool) {
    let mut edit = None;
    Field::new(label).show(ui, |ui, _| {
        let mut field = NumberField::new(label, Bind::given(Some(shown))).fixed_decimals(num.decimals).enabled(enabled);
        if let Some(range) = num.range.clone() {
            field = field.range(range);
        }
        if let Some(unit) = num.unit {
            field = field.unit(unit);
        }
        edit = Some(field.show(ui).edit);
    });
    if let Some(Some(x)) = edit.as_ref().and_then(|edit| edit.events.committed().copied()) {
        v.set(key, json!(x));
    }
}

/// A text row over `key` that takes the characters `graph.setType` keeps.
fn text_row(ui: &mut Ui, v: &mut GraphTypeValues, key: &str, label: &'static str) {
    let mut typed = None;
    Field::new(label).show(ui, |ui, _| {
        typed = TextField::new(label, Bind::given(v.str(key).to_owned())).limit(AFFIX_CHARS).show(ui).events.committed().cloned();
    });
    if let Some(text) = typed {
        v.set(key, json!(text));
    }
}

fn options_page(ui: &mut Ui, v: &mut GraphTypeValues, s: &GraphSubject) {
    let kind = graph_kind(v, s);
    crate::panels::section(tl!("Type")).show(ui, |ui, _| {
        type_row(ui, v, s);
        let sides: &[(&str, &str)] = if s.series.is_empty() { &SIDES } else { &SERIES_SIDES };
        let enabled = active(v, s, "valueAxis");
        choice(ui, v, "valueAxis", tl!("Value Axis"), sides, enabled);
        let enabled = active(v, s, "separateScales");
        check(ui, v, "separateScales", tl!("Separate Scales"), enabled);
    });
    crate::panels::section(tl!("Style")).show(ui, |ui, _| {
        Grid::new(TWO).show(ui, |ui, _| {
            let rows = [
                ("dropShadow", tl!("Add Drop Shadow")),
                ("firstRowInFront", tl!("First Row in Front")),
                ("legendAcrossTop", tl!("Add Legend Across Top")),
                ("firstColumnInFront", tl!("First Column in Front")),
                ("legend", tl!("Legend")),
            ];
            // A pie graph has its Legend menu among its options.
            for (key, label) in rows.into_iter().filter(|(key, _)| *key != "legend" || kind != GraphKind::Pie) {
                let enabled = active(v, s, key);
                check(ui, v, key, label, enabled);
            }
        });
    });
    type_options(ui, v, s, kind);
}

/// The Options of the type picked: widths for columns and bars, data points and lines for line,
/// scatter and radar series, and Legend, Position and Sort for a pie graph. An area graph has none.
fn type_options(ui: &mut Ui, v: &mut GraphTypeValues, s: &GraphSubject, kind: GraphKind) {
    let (columns, lines, pie) = (draws(v, s, &COLUMNS), draws(v, s, &LINES), kind == GraphKind::Pie);
    if !(columns || lines || pie) {
        return;
    }
    crate::panels::section(tl!("Options")).show(ui, |ui, _| {
        if columns {
            let width = if matches!(kind, GraphKind::Bar | GraphKind::StackedBar) { tl!("Bar Width") } else { tl!("Column Width") };
            for (key, label) in [("columnWidth", width), ("clusterWidth", tl!("Cluster Width"))] {
                let shown = number(v, key).unwrap_or(100.0);
                number_row(ui, v, key, label, shown, &PERCENT, true);
            }
        }
        if lines {
            // Edge-to-Edge Lines is for line series, and Draw Filled Lines for connected data points.
            Grid::new(TWO).show(ui, |ui, _| {
                let rows = [
                    ("markPoints", tl!("Mark Data Points")),
                    ("edgeToEdge", tl!("Edge-to-Edge Lines")),
                    ("connectPoints", tl!("Connect Data Points")),
                    ("filledLines", tl!("Draw Filled Lines")),
                ];
                for (key, label) in rows {
                    let enabled = active(v, s, key);
                    check(ui, v, key, label, enabled);
                }
            });
            // Line Width sets how wide Draw Filled Lines draws the lines.
            let shown = number(v, "lineWidth").unwrap_or(vectorcraft_doc::DEFAULT_FILLED_LINE_WIDTH);
            let enabled = active(v, s, "lineWidth");
            number_row(ui, v, "lineWidth", tl!("Line Width"), shown, &POINTS, enabled);
        }
        if pie {
            choice(ui, v, "pieLegend", tl!("Legend"), &PIE_LEGENDS, true);
            choice(ui, v, "piePosition", tl!("Position"), &PIE_POSITIONS, true);
            choice(ui, v, "pieSort", tl!("Sort"), &PIE_SORTS, true);
        }
    });
}

/// The page of a value axis: the tick values of the scale it shows, and the tick marks and labels
/// of its side.
fn value_page(ui: &mut Ui, v: &mut GraphTypeValues, s: &GraphSubject, right: bool) {
    let (scale, side) = page_axes(v, s, right);
    let own_scale = scale_used(v, s, right);
    crate::panels::section(tl!("Tick Values")).show(ui, |ui, _| {
        let (drawn, (lo, hi), count) = shown_scale(v, s, right);
        check(ui, v, drawn.overridden, tl!("Override Calculated Values"), own_scale);
        let enabled = own_scale && v.bool(scale.overridden);
        number_row(ui, v, scale.min, tl!("Minimum"), lo, &BOUND, enabled);
        number_row(ui, v, scale.max, tl!("Maximum"), hi, &BOUND, enabled);
        number_row(ui, v, scale.ticks, tl!("Divisions"), count as f64, &DIVISION_COUNT, enabled);
    });
    tick_marks(ui, v, side.length, side.marks);
    crate::panels::section(tl!("Add Labels")).show(ui, |ui, _| {
        text_row(ui, v, side.prefix, tl!("Prefix"));
        text_row(ui, v, side.suffix, tl!("Suffix"));
    });
}

/// The scale whose tick values a value axis page shows, with the bounds and divisions it shows.
/// Without Separate Scales the right axis shows the left axis' tick values, and with it a side
/// without series shows the other side's; with Override Calculated Values off a scale shows the
/// values it is calculated with.
fn shown_scale(v: &GraphTypeValues, s: &GraphSubject, right: bool) -> (&'static Axis, (f64, f64), usize) {
    let (scale, _) = page_axes(v, s, right);
    let drawn = if scale_used(v, s, right) {
        scale
    } else if right {
        &LEFT
    } else {
        &RIGHT
    };
    if v.bool(drawn.overridden) {
        (drawn, bounds(v, s, drawn), divisions(v, s, drawn))
    } else {
        let (lo, hi, count) = calculated(s, drawn);
        (drawn, (lo, hi), count)
    }
}

fn category_page(ui: &mut Ui, v: &mut GraphTypeValues, s: &GraphSubject) {
    tick_marks(ui, v, "categoryTickLength", "categoryTickMarks");
    let enabled = active(v, s, "ticksBetweenLabels");
    check(ui, v, "ticksBetweenLabels", tl!("Draw Tick Marks Between Labels"), enabled);
}

/// Tick Marks: the length, and the tick marks per division while there are tick marks to draw.
fn tick_marks(ui: &mut Ui, v: &mut GraphTypeValues, length: &str, marks: &str) {
    crate::panels::section(tl!("Tick Marks")).show(ui, |ui, _| {
        choice(ui, v, length, tl!("Length"), &TICK_LENGTHS, true);
        let shown = number(v, marks).unwrap_or(1.0);
        let enabled = v.str(length) != "none";
        number_row(ui, v, marks, tl!("Tick Marks per Division"), shown, &MARK_COUNT, enabled);
    });
}

/// The picture of the type at its place in [`GraphKind::ALL`].
#[derive(MemoProps)]
struct KindGlyph(usize);

impl Glyph for KindGlyph {
    fn paint(&self, painter: &egui::Painter, _: &UiTokens, rect: egui::Rect, color: egui::Color32, _: bool) {
        if let Some(kind) = GraphKind::ALL.get(self.0) {
            paint_kind(painter, rect, *kind, color);
        }
    }
}

static KIND_GLYPHS: [KindGlyph; 9] =
    [KindGlyph(0), KindGlyph(1), KindGlyph(2), KindGlyph(3), KindGlyph(4), KindGlyph(5), KindGlyph(6), KindGlyph(7), KindGlyph(8)];

/// The nine Type buttons. With series picked, the types that cannot go in the graph are off.
fn type_row(ui: &mut Ui, v: &mut GraphTypeValues, s: &GraphSubject) {
    let own = (!s.series.is_empty()).then(|| graph_kind(v, s));
    let entries: Vec<Entry<&'static str>> = GraphKind::ALL
        .iter()
        .zip(&KIND_GLYPHS)
        .map(|(&kind, glyph)| {
            Entry::new(kind.id(), kind_name(kind)).icon(IconSpec::glyph(glyph)).enabled(own.is_none_or(|graph| kind.combines_with(graph)))
        })
        .collect();
    let shown = chosen(v).map_or(Mixed::Mixed, |kind| Mixed::Same(kind.id()));
    if let Some(id) = Segmented::new(tl!("Type"), Bind::given(shown), &entries).show(ui).events.committed().copied() {
        v.set("type", json!(id));
    }
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
    use vectorcraft_engine::Session;

    use super::*;

    /// A 400 by 400 document with a selected graph of type `kind`: series a and b over Q1 (1, 2)
    /// and Q2 (3, 4). The app has a window to host the dialog.
    fn app_with_graph(kind: &str) -> VectorcraftApp {
        let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
        app.run("file.new", json!({"width": 400, "height": 400})).unwrap();
        app.run("graph.create", json!({"type": kind, "x": 0, "y": 0, "width": 200, "height": 150, "csv": ",a,b\nQ1,1,2\nQ2,3,4"})).unwrap();
        app
    }

    fn ctx(app: &VectorcraftApp) -> Context {
        app.ui_ctx.clone().unwrap()
    }

    /// The open dialog, read or changed by `f`.
    fn with<R>(app: &VectorcraftApp, f: impl FnOnce(&mut GraphTypeDialog) -> R) -> R {
        app.dialogs().unwrap().with(|host| f(host.get_mut_by_id::<GraphTypeDialog>(dialog_id(KIND)).expect("Graph Type is open")))
    }

    /// `ui.dialog.set` on the open dialog.
    fn set(app: &VectorcraftApp, key: &str, value: Value) -> Result<(), String> {
        let ctx = ctx(app);
        app.dialogs().unwrap().with(|host| host.set_json_field(&ctx, dialog_id(KIND), key, value))
    }

    fn field(app: &VectorcraftApp, key: &str) -> Value {
        with(app, |d| d.json_fields().unwrap()[key].clone())
    }

    fn inactive(key: &str) -> Result<(), String> {
        Err(FieldRefusal::Inactive.message(key))
    }

    /// The pump of one frame.
    fn frame(app: &mut VectorcraftApp) {
        let ctx = ctx(app);
        pump(app, &ctx);
    }

    fn is_open(app: &VectorcraftApp) -> bool {
        app.dialogs().unwrap().with(|host| host.is_open::<GraphTypeDialog>())
    }

    /// OK through the host: the press files the values, the pump answers, and a second press closes
    /// the dialog the pump accepted. Whether it closed.
    fn ok(app: &mut VectorcraftApp) -> bool {
        let (ctx, dialogs) = (ctx(app), app.dialogs().unwrap());
        if dialogs.with(|host| host.confirm_by_id(&ctx, dialog_id(KIND))) {
            return true;
        }
        frame(app);
        dialogs.with(|host| host.confirm_by_id(&ctx, dialog_id(KIND)))
    }

    fn query(app: &mut VectorcraftApp) -> Value {
        app.session.execute(COMMAND, &json!({})).unwrap()
    }

    fn steps(app: &VectorcraftApp) -> usize {
        app.session.active().unwrap().history.undo.len()
    }

    /// The dialog edits a draft until OK. Escape from a field or popup must not leave its last
    /// proposal in that draft, while deliberate edits still apply together in one undo step.
    #[test]
    fn real_cancelled_and_committed_controls_apply_only_the_accepted_graph_options() {
        use crate::tests::panel_harness::Panel;
        use craft::ui::contract::Role;
        use egui::{Key, Modifiers};

        for commit in [false, true] {
            let mut app = app_with_graph("column");
            app.run(COMMAND, json!({"columnWidth": 80, "prefix": "USD", "tickLength": "short", "dropShadow": false})).unwrap();
            let original = query(&mut app);
            let document = app.session.active().unwrap().doc.clone();
            let before = steps(&app);
            let mut panel = Panel::new(app, crate::tests::picks::dialogs);
            panel.room(egui::vec2(900.0, 900.0));
            open(&mut panel.app()).unwrap();
            panel.frame();
            panel.click_role("Column Width", Role::SPIN_BUTTON);
            panel.key(Key::A, Modifiers::COMMAND);
            panel.type_text("140");
            if commit {
                panel.key(Key::Tab, Modifiers::NONE);
            } else {
                panel.key(Key::Escape, Modifiers::NONE);
                assert!(!is_open(&panel.app()), "Escape from a typed number discards the modal");
                assert!(panel.app().session.active().unwrap().doc == document);
                open(&mut panel.app()).unwrap();
            }
            panel.frame();
            assert_eq!(field(&panel.app(), "columnWidth"), if commit { json!(140.0) } else { json!(80.0) });
            if commit {
                panel.click("Add Drop Shadow");
                panel.click("Stacked Column Graph");
                assert_eq!(field(&panel.app(), "dropShadow"), true);
                assert_eq!(field(&panel.app(), "type"), "stackedColumn");
            }
            panel.click_role("Graph Type", Role::COMBO_BOX);
            panel.key(Key::ArrowDown, Modifiers::NONE);
            panel.key(Key::Escape, Modifiers::NONE);
            panel.frame();
            assert_eq!(field(&panel.app(), PAGE), "options", "the cancelled page choice stays on Graph Options");
            panel.click_role("Graph Type", Role::COMBO_BOX);
            panel.key(Key::ArrowDown, Modifiers::NONE);
            panel.key(Key::Enter, Modifiers::NONE);
            panel.frame();
            assert_eq!(field(&panel.app(), PAGE), "valueAxis");
            if commit {
                panel.click_role("Prefix", Role::TEXT_INPUT);
                panel.key(Key::A, Modifiers::COMMAND);
                panel.type_text("EUR");
                panel.key(Key::Tab, Modifiers::NONE);
                panel.frame();
            }
            assert_eq!(field(&panel.app(), "prefix"), if commit { json!("EUR") } else { json!("USD") });
            panel.click_role("Length", Role::COMBO_BOX);
            panel.key(Key::ArrowDown, Modifiers::NONE);
            panel.key(if commit { Key::Enter } else { Key::Escape }, Modifiers::NONE);
            panel.frame();
            assert_eq!(field(&panel.app(), "tickLength"), if commit { json!("full") } else { json!("short") });
            assert_eq!(*panel.app().session.active().unwrap().doc, *document, "the draft has no live artwork preview");
            assert_eq!(steps(&panel.app()), before);
            panel.click("OK");
            panel.frame();
            panel.frame();
            assert!(!is_open(&panel.app()));
            assert_eq!(steps(&panel.app()), before + usize::from(commit));
            let after = query(&mut panel.app());
            if commit {
                assert_eq!(
                    (after["columnWidth"].clone(), after["prefix"].clone(), after["tickLength"].clone()),
                    (json!(140.0), json!("EUR"), json!("full"))
                );
                assert_eq!(after["type"], "stackedColumn");
                assert_eq!(after["dropShadow"], true);
                panel.app().run("edit.undo", json!({})).unwrap();
                assert_eq!(query(&mut panel.app()), original);
            } else {
                assert_eq!(after, original);
                assert_eq!(*panel.app().session.active().unwrap().doc, *document);
            }
        }
    }

    /// Object › Graph › Type… opens the dialog on the selected graph's options, on its Graph
    /// Options page; each page the type has can be shown, and Override Calculated Values of the
    /// right axis takes input only while that axis has a scale of its own.
    #[test]
    fn the_graph_menu_opens_graph_type_on_the_selected_graph() {
        let mut app = app_with_graph("column");
        crate::menus::invoke(&mut app, COMMAND, json!({}));
        let f = with(&app, |d| d.json_fields().unwrap());
        assert_eq!((f["type"].clone(), f["valueAxis"].clone(), f[PAGE].clone()), (json!("column"), json!("left"), json!("options")));
        assert_eq!((f["overrideValues"].clone(), f["rightOverrideValues"].clone()), (json!(false), json!(false)));
        assert!(with(&app, |d| d.subject.series.is_empty()));
        for key in ["legend", "pieLegend", "piePosition", "pieSort", "firstRowInFront", "firstColumnInFront", "filledLines"] {
            assert!(f.get(key).is_some(), "{key} is held for every type");
        }
        for page in ["valueAxis", "categoryAxis", "options"] {
            set(&app, PAGE, json!(page)).unwrap();
        }
        assert!(set(&app, PAGE, json!("rightAxis")).is_err(), "one value axis: no right axis page");
        set(&app, "valueAxis", json!("both")).unwrap();
        set(&app, PAGE, json!("rightAxis")).unwrap();
        // The right axis shows the left axis' tick values until Separate Scales gives it its own.
        assert_eq!(set(&app, "rightOverrideValues", json!(true)), inactive("rightOverrideValues"));
        set(&app, "separateScales", json!(true)).unwrap();
        with(&app, |d| d.subject.right_series = vec![1]);
        set(&app, "rightOverrideValues", json!(true)).unwrap();
        // Without a selected graph the menu item opens no dialog.
        let mut app = app_with_graph("column");
        app.run("select.set", json!({"ids": []})).unwrap();
        assert!(open(&mut app).is_err());
        assert!(!is_open(&app));
    }

    /// A type picked changes the graph's type and keeps its data.
    #[test]
    fn a_type_button_changes_the_graph_type_and_keeps_its_data() {
        let mut app = app_with_graph("column");
        let data = app.session.execute("graph.setData", &json!({})).unwrap();
        assert_eq!(data["rows"], json!([[1.0, 2.0], [3.0, 4.0]]));
        open(&mut app).unwrap();
        for kind in GraphKind::ALL {
            set(&app, "type", json!(kind.id())).unwrap();
            assert_eq!(field(&app, "type"), kind.id());
        }
        set(&app, "type", json!("stackedBar")).unwrap();
        let before = steps(&app);
        assert!(ok(&mut app));
        assert_eq!(query(&mut app)["type"], "stackedBar");
        assert_eq!(app.session.execute("graph.setData", &json!({})).unwrap(), data);
        assert_eq!(steps(&app), before + 1, "one step of history");
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
        assert_eq!(set(&app, "separateScales", json!(true)), inactive("separateScales"), "one side: one scale");
        set(&app, "valueAxis", json!("both")).unwrap();
        set(&app, "separateScales", json!(true)).unwrap();
        assert!(ok(&mut app));
        assert_eq!(query(&mut app)["valueAxis"], "both");
    }

    /// Line Width opens at the graph's width, and OK gives the graph Draw Filled Lines with the
    /// width typed.
    #[test]
    fn line_width_goes_with_draw_filled_lines() {
        let mut app = app_with_graph("line");
        open(&mut app).unwrap();
        assert_eq!(field(&app, "lineWidth").as_f64(), Some(vectorcraft_doc::DEFAULT_FILLED_LINE_WIDTH));
        for (key, value) in [("connectPoints", json!(true)), ("filledLines", json!(true)), ("lineWidth", json!(6.5))] {
            set(&app, key, value).unwrap();
        }
        let before = steps(&app);
        assert!(ok(&mut app));
        let q = query(&mut app);
        assert_eq!((q["filledLines"].clone(), q["lineWidth"].clone()), (json!(true), json!(6.5)));
        assert_eq!(steps(&app), before + 1, "one step of history");
    }

    /// The menu at the top lists a page per axis of the type picked, and a second value axis page
    /// while the value axis is on both sides.
    #[test]
    fn the_menu_lists_a_page_per_axis_of_the_type() {
        let mut app = app_with_graph("column");
        open(&mut app).unwrap();
        let (mut v, s) = with(&app, |d| (d.values.clone(), d.subject.clone()));
        let ids = |v: &GraphTypeValues| pages(v, &s).iter().map(|(id, _)| *id).collect::<Vec<_>>();
        let names = |v: &GraphTypeValues| pages(v, &s).iter().map(|(_, name)| *name).collect::<Vec<_>>();
        for kind in ["column", "stackedColumn", "bar", "stackedBar", "line", "area"] {
            v.set("type", json!(kind));
            assert_eq!(ids(&v), ["options", "valueAxis", "categoryAxis"], "{kind}");
        }
        assert_eq!(names(&v), ["Graph Options", "Value Axis", "Category Axis"]);
        for kind in ["scatter", "radar"] {
            v.set("type", json!(kind));
            assert_eq!(ids(&v), ["options", "valueAxis"], "{kind}");
        }
        v.set("type", json!("pie"));
        assert_eq!(ids(&v), ["options"]);
        v.set("valueAxis", json!("both"));
        for kind in ["column", "stackedColumn", "line", "area"] {
            v.set("type", json!(kind));
            assert_eq!(ids(&v), ["options", "valueAxis", "rightAxis", "categoryAxis"], "{kind}");
            assert_eq!(names(&v), ["Graph Options", "Left Axis", "Right Axis", "Category Axis"], "{kind}");
        }
        v.set("type", json!("scatter"));
        assert_eq!(ids(&v), ["options", "valueAxis", "rightAxis"]);
        // A bar or radar graph's value axis has no sides, and a pie graph has no value axis.
        for (kind, pages) in
            [("bar", vec!["options", "valueAxis", "categoryAxis"]), ("radar", vec!["options", "valueAxis"]), ("pie", vec!["options"])]
        {
            v.set("type", json!(kind));
            assert_eq!(ids(&v), pages, "{kind}");
        }
    }

    /// Override Calculated Values starts from the tick values the axis is drawn with, OK sends
    /// them, and turning it off again gives the axis its calculated values back. Left off, it
    /// leaves a count the graph has alone.
    #[test]
    fn override_calculated_values_starts_from_the_calculated_tick_values() {
        let mut app = app_with_graph("column");
        open(&mut app).unwrap();
        assert_eq!(with(&app, |d| calculated(&d.subject, &LEFT)), (0.0, 4.0, 4));
        assert_eq!(set(&app, "axisMin", json!(1)), inactive("axisMin"), "calculated values take no input");
        set(&app, "overrideValues", json!(true)).unwrap();
        frame(&mut app);
        let axis = |app: &VectorcraftApp| (field(app, "axisMin").as_f64(), field(app, "axisMax").as_f64(), field(app, "ticks").as_f64());
        assert_eq!(axis(&app), (Some(0.0), Some(4.0), Some(4.0)));
        let before = steps(&app);
        assert!(ok(&mut app));
        let q = query(&mut app);
        assert_eq!((q["axisMin"].as_f64(), q["axisMax"].as_f64(), q["ticks"].as_u64()), (Some(0.0), Some(4.0), Some(4)));
        assert_eq!(steps(&app), before + 1);
        // It opens on, with the graph's values, and off it sends the calculated values back.
        open(&mut app).unwrap();
        assert_eq!(field(&app, "overrideValues"), true);
        set(&app, "axisMax", json!(8.0)).unwrap();
        assert!(ok(&mut app));
        assert_eq!(query(&mut app)["axisMax"], 8.0);
        open(&mut app).unwrap();
        set(&app, "overrideValues", json!(false)).unwrap();
        assert!(ok(&mut app));
        let q = query(&mut app);
        assert_eq!((q["axisMin"].clone(), q["axisMax"].clone(), q["ticks"].clone()), (Value::Null, Value::Null, json!(0)));
        // Left off, a count the graph has stays, whatever else OK changes.
        app.run(COMMAND, json!({"ticks": 7})).unwrap();
        open(&mut app).unwrap();
        set(&app, "dropShadow", json!(true)).unwrap();
        assert!(ok(&mut app));
        let q = query(&mut app);
        assert_eq!((q["ticks"].clone(), q["dropShadow"].clone(), q["axisMin"].clone()), (json!(7), json!(true), Value::Null));
    }

    /// An overridden value axis whose maximum is not more than its minimum turns OK off with the
    /// reason, and nothing is sent. The right axis counts while it has a scale of its own.
    #[test]
    fn an_overridden_axis_needs_a_maximum_above_its_minimum() {
        let mut app = app_with_graph("column");
        open(&mut app).unwrap();
        for (key, value) in [("overrideValues", json!(true)), ("axisMin", json!(10)), ("axisMax", json!(10))] {
            set(&app, key, value).unwrap();
        }
        assert_eq!(with(&app, |d| problem(&d.values, &d.subject)), Some("Maximum must be more than Minimum."));
        let (ctx, before) = (ctx(&app), steps(&app));
        assert!(!app.dialogs().unwrap().with(|host| host.confirm_by_id(&ctx, dialog_id(KIND))));
        pump(&mut app, &ctx);
        assert!(is_open(&app), "the dialog stays open");
        assert_eq!((steps(&app), query(&mut app)["axisMax"].clone()), (before, Value::Null));
        // A range too wide to divide is refused too, and a value that is not a number is not taken.
        set(&app, "axisMin", json!(-1.7e308)).unwrap();
        set(&app, "axisMax", json!(1.7e308)).unwrap();
        assert!(with(&app, |d| problem(&d.values, &d.subject)).is_some());
        assert!(set(&app, "axisMax", json!("ten")).is_err());
        set(&app, "axisMin", json!(10)).unwrap();
        set(&app, "axisMax", json!(20)).unwrap();
        assert!(ok(&mut app));
        let q = query(&mut app);
        assert_eq!((q["axisMin"].as_f64(), q["axisMax"].as_f64()), (Some(10.0), Some(20.0)));
        // The right axis has a scale of its own with Separate Scales and a series on its side.
        open(&mut app).unwrap();
        set(&app, "valueAxis", json!("both")).unwrap();
        set(&app, "separateScales", json!(true)).unwrap();
        with(&app, |d| d.subject.right_series = vec![1]);
        for (key, value) in [("rightOverrideValues", json!(true)), ("rightAxisMin", json!(5)), ("rightAxisMax", json!(1))] {
            set(&app, key, value).unwrap();
        }
        assert!(with(&app, |d| problem(&d.values, &d.subject)).is_some());
        // A pie graph has no value axis to refuse.
        set(&app, "type", json!("pie")).unwrap();
        assert_eq!(with(&app, |d| problem(&d.values, &d.subject)), None);
    }

    /// OK with nothing changed closes the dialog without a step of history, after every page and
    /// for every type.
    #[test]
    fn ok_with_nothing_changed_adds_no_step_of_history() {
        for kind in GraphKind::ALL {
            let mut app = app_with_graph(kind.id());
            app.run(COMMAND, json!({"valueAxis": "both", "axisMin": 0, "axisMax": 10})).unwrap();
            let before = steps(&app);
            open(&mut app).unwrap();
            let shown: Vec<&str> = with(&app, |d| pages(&d.values, &d.subject).iter().map(|(id, _)| *id).collect());
            for page in shown.into_iter().rev() {
                set(&app, PAGE, json!(page)).unwrap();
                frame(&mut app);
            }
            assert!(ok(&mut app), "{kind:?}");
            assert_eq!(steps(&app), before, "{kind:?}");
        }
    }

    /// With series picked by Group Selection, the type and the value axis go to those series: the
    /// types that cannot go in the graph are refused, Value Axis offers one side, and the other
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
        assert_eq!(with(&app, |d| d.subject.series.clone()), [1]);
        for kind in [GraphKind::Bar, GraphKind::StackedBar, GraphKind::Scatter, GraphKind::Pie, GraphKind::Radar] {
            assert_eq!(set(&app, "type", json!(kind.id())), inactive("type"), "{kind:?}");
            assert_eq!(field(&app, "type"), "column", "{kind:?}");
        }
        assert!(set(&app, "valueAxis", json!("both")).is_err(), "a series goes on one axis");
        set(&app, "valueAxis", json!("right")).unwrap();
        set(&app, "type", json!("line")).unwrap();
        // The picked line series brings the line options beside the columns'.
        set(&app, "markPoints", json!(true)).unwrap();
        set(&app, "columnWidth", json!(50.0)).unwrap();
        assert!(ok(&mut app));
        // The series stays picked, on its own type and axis.
        let q = query(&mut app);
        assert_eq!((q["type"].clone(), q["valueAxis"].clone(), q["columnWidth"].clone()), (json!("line"), json!("right"), json!(50.0)));
        let whole = app.session.execute(COMMAND, &json!({"id": graph.0})).unwrap();
        assert_eq!((whole["type"].clone(), whole["valueAxis"].clone()), (json!("column"), json!("left")));
        // Series on both value axes open with no side, and OK leaves each where it is.
        let both: Vec<u64> = {
            let st = app.session.active().unwrap();
            st.doc.node(graph).unwrap().children().unwrap().iter().filter(|c| c.extras().series_index.is_some()).map(|c| c.id.0).collect()
        };
        app.run("select.set", json!({"ids": both})).unwrap();
        open(&mut app).unwrap();
        assert_eq!((field(&app, "valueAxis"), field(&app, "type")), (Value::Null, Value::Null));
        assert_eq!(with(&app, |d| series_kinds(&d.values, &d.subject)), [GraphKind::Column, GraphKind::Line]);
        set(&app, "dropShadow", json!(true)).unwrap();
        assert!(ok(&mut app));
        let st = app.session.active().unwrap();
        let spec = st.doc.node(graph).unwrap().extras().graph.as_deref().unwrap();
        assert_eq!((spec.right_series.clone(), spec.drop_shadow), (vec![1], true));
        assert_eq!(spec.series_kinds, [None, Some(GraphKind::Line)], "each series keeps its type");
    }

    /// A value axis side without a scale of its own shows the tick values of the scale drawn on it,
    /// not bounds kept for it.
    #[test]
    fn a_side_without_its_own_scale_shows_the_scale_drawn_on_it() {
        let mut app = app_with_graph("column");
        app.run(COMMAND, json!({"valueAxis": "both", "rightAxisMin": 100, "rightAxisMax": 200})).unwrap();
        open(&mut app).unwrap();
        assert_eq!(field(&app, "rightOverrideValues"), true);
        let shown = |app: &VectorcraftApp| with(app, |d| shown_scale(&d.values, &d.subject, true).1);
        assert_eq!(shown(&app), (0.0, 4.0), "the left scale, calculated");
        // With Separate Scales and a series on the right axis, the side shows its own bounds.
        set(&app, "separateScales", json!(true)).unwrap();
        with(&app, |d| d.subject.right_series = vec![1]);
        assert_eq!(shown(&app), (100.0, 200.0));
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
        set(&app, "type", json!("line")).unwrap();
        app.run("select.set", json!({"ids": [second.0]})).unwrap();
        assert!(ok(&mut app));
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
        set(&app, "type", json!("area")).unwrap();
        app.run("select.set", json!({"ids": [first.0]})).unwrap();
        assert!(ok(&mut app));
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
        assert_eq!(with(&app, |d| calculated(&d.subject, &LEFT)).1, 4.0);
        // Stacked, Q2 reaches 3 + 4 = 7.
        set(&app, "type", json!("stackedColumn")).unwrap();
        set(&app, "overrideValues", json!(true)).unwrap();
        assert!(ok(&mut app));
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
        set(&app, "overrideValues", json!(true)).unwrap();
        frame(&mut app);
        assert_eq!(field(&app, "axisMax").as_f64(), Some(4.0));
        set(&app, "type", json!("stackedColumn")).unwrap();
        assert!(ok(&mut app));
        assert_eq!(sent(&query(&mut app)), (Some(stacked.0), Some(stacked.1), Some(stacked.2 as u64)));
        // Values typed in stay.
        open(&mut app).unwrap();
        set(&app, "axisMax", json!(50.0)).unwrap();
        set(&app, "type", json!("column")).unwrap();
        assert!(ok(&mut app));
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
        for (key, value) in [("overrideValues", json!(true)), ("axisMax", json!(50.0)), ("ticks", json!(10)), ("overrideValues", json!(false))] {
            set(&app, key, value).unwrap();
        }
        let (_, (lo, hi), count) = with(&app, |d| shown_scale(&d.values, &d.subject, false));
        assert_eq!((lo, hi, count), (0.0, 4.0, 4));
    }

    /// The tick values are calculated again when the graph's data changes while the dialog is open.
    #[test]
    fn a_data_change_calculates_the_tick_values_again() {
        let data = |app: &mut VectorcraftApp, csv: &str| app.session.execute("graph.setData", &json!({"csv": csv})).unwrap();
        let mut app = app_with_graph("column");
        open(&mut app).unwrap();
        assert_eq!(with(&app, |d| calculated(&d.subject, &LEFT)).1, 4.0);
        data(&mut app, ",a,b\nQ1,1,2\nQ2,3,40");
        let max = with(&app, |d| {
            recalculate(&app, d);
            calculated(&d.subject, &LEFT).1
        });
        assert!(max >= 40.0, "{max}");
        // Turned on now, Override Calculated Values starts from the new values.
        set(&app, "overrideValues", json!(true)).unwrap();
        assert!(ok(&mut app));
        assert!(query(&mut app)["axisMax"].as_f64().is_some_and(|max| max >= 40.0));
        // Tick values the graph has keep, even when they are the calculated ones.
        let mut app = app_with_graph("column");
        app.run(COMMAND, json!({"axisMin": 0, "axisMax": 4, "ticks": 4})).unwrap();
        open(&mut app).unwrap();
        data(&mut app, ",a,b\nQ1,1,2\nQ2,3,40");
        let before = steps(&app);
        assert!(ok(&mut app));
        assert_eq!((steps(&app), query(&mut app)["axisMax"].as_f64()), (before, Some(4.0)));
        // A value typed in stays, and the values filled beside it are calculated again.
        let mut app = app_with_graph("column");
        open(&mut app).unwrap();
        set(&app, "overrideValues", json!(true)).unwrap();
        with(&app, |d| settle(&mut d.values, &d.subject));
        set(&app, "axisMax", json!(50.0)).unwrap();
        data(&mut app, ",a,b\nQ1,-30,2\nQ2,3,4");
        let (lo, _, divisions) = with(&app, |d| {
            recalculate(&app, d);
            calculated(&d.subject, &LEFT)
        });
        assert!(lo < 0.0, "{lo}");
        assert!(ok(&mut app));
        let q = query(&mut app);
        assert_eq!((q["axisMin"].as_f64(), q["axisMax"].as_f64(), q["ticks"].as_u64()), (Some(lo), Some(50.0), Some(divisions as u64)));
        // With both bounds typed in, the divisions filled are calculated again, not the count an
        // axis without one is drawn in.
        let mut app = app_with_graph("column");
        open(&mut app).unwrap();
        set(&app, "overrideValues", json!(true)).unwrap();
        with(&app, |d| settle(&mut d.values, &d.subject));
        set(&app, "axisMin", json!(-100.0)).unwrap();
        set(&app, "axisMax", json!(100.0)).unwrap();
        data(&mut app, ",a,b\nQ1,-30,2\nQ2,3,4");
        let (ticks, calculated_ticks) = with(&app, |d| {
            recalculate(&app, d);
            settle(&mut d.values, &d.subject);
            (number(&d.values, "ticks"), calculated(&d.subject, &LEFT).2 as f64)
        });
        assert_eq!(ticks, Some(calculated_ticks));
        // Each value the graph had keeps, beside one typed in.
        let mut app = app_with_graph("column");
        app.run(COMMAND, json!({"axisMin": 0, "axisMax": 4, "ticks": 4})).unwrap();
        open(&mut app).unwrap();
        set(&app, "axisMax", json!(50.0)).unwrap();
        data(&mut app, ",a,b\nQ1,-30,2\nQ2,3,4");
        assert!(ok(&mut app));
        let q = query(&mut app);
        assert_eq!((q["axisMin"].as_f64(), q["axisMax"].as_f64(), q["ticks"].as_u64()), (Some(0.0), Some(50.0), Some(4)));
    }

    /// OK changes nothing while another document is active, even one with a graph of the same id.
    #[test]
    fn ok_changes_nothing_in_another_document() {
        let mut app = app_with_graph("column");
        let first = app.session.active().unwrap().selection.objects[0];
        open(&mut app).unwrap();
        set(&app, "type", json!("line")).unwrap();
        app.run("file.new", json!({"width": 400, "height": 400})).unwrap();
        app.run("graph.create", json!({"type": "column", "x": 0, "y": 0, "width": 200, "height": 150, "csv": ",a,b\nQ1,1,2\nQ2,3,4"})).unwrap();
        let second = app.session.active().unwrap().selection.objects[0];
        assert_eq!(first, second, "the same id in both documents");
        let before = steps(&app);
        assert!(ok(&mut app), "OK closes the dialog");
        assert_eq!(steps(&app), before);
        assert_eq!(app.session.execute(COMMAND, &json!({"id": second.0})).unwrap()["type"], "column");
    }

    #[test]
    fn graph_type_refuses_to_change_the_restored_graph() {
        let mut app = app_with_graph("column");
        let graph = app.session.active().unwrap().selection.objects[0];
        let dir = vectorcraft_testkit::TempDir::new("graph-type-revert");
        app.services.read = Some(Box::new(|path| std::fs::read(path).map_err(|e| e.to_string())));
        app.services.write = Some(Box::new(|path, bytes| std::fs::write(path, bytes).map_err(|e| e.to_string())));
        app.run("file.saveAs", json!({"path": dir.join("Art.vectorcraft")})).unwrap();
        app.run(COMMAND, json!({"id": graph.0, "type": "bar"})).unwrap();
        open(&mut app).unwrap();
        set(&app, "type", json!("line")).unwrap();

        app.run("file.revert", json!({"confirmed": true})).unwrap();
        app.run("select.set", json!({"ids": [graph.0]})).unwrap();
        let before = steps(&app);
        crate::tests::picks::answer_dialog(&mut app, dialog_id(KIND), &[], true);
        assert!(matches!(with(&app, |dialog| dialog.held_state()), Some(HeldOk::Refused(_))));
        assert_eq!(query(&mut app)["type"], "column");
        assert_eq!(steps(&app), before);

        let ctx = ctx(&app);
        app.dialogs().unwrap().with(|host| host.cancel_by_id(&ctx, dialog_id(KIND)));
        open(&mut app).unwrap();
        set(&app, "type", json!("line")).unwrap();
        assert!(ok(&mut app));
        assert_eq!(query(&mut app)["type"], "line");
    }
}
