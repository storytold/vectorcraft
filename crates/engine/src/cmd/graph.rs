//! Graphs: the nine graph tools and Object → Graph (Type…, Data…).
//!
//! A graph is a group carrying a [`GraphSpec`]; its children (axes, labels, one group per series
//! with that series' legend swatch, like Illustrator's group-selectable series) are generated from
//! the spec. Editing the data or the type regenerates them in place, keeping any move/scale the
//! user applied to the graph since it was generated.

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{
    Appearance, CharStyle, Document, GraphDesign, GraphKind, GraphSpec, Justify, Node, NodeId, NodeKind, TextObject, TickLength, ValueAxisSide,
};
use vectorcraft_geom::{Affine, BezPath, PathData, Point, Rect, shapes};

use super::typecmd::refresh_bounds;
use super::*;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "graph.create",
            "Create Graph",
            [],
            None,
            "{type: column|stackedColumn|bar|stackedBar|line|area|scatter|pie|radar, x, y, width, height, series?: [..], categories?: [..], rows?: [[number|null]], csv?} → {id}; an empty CSV cell or a null is a blank value, a number in straight quotes is a label",
            has_doc,
            create
        ),
        cmd!(
            "graph.setData",
            "Data…",
            ["Object", "Graph"],
            None,
            "{id?, series?, categories?, rows?, csv?: first row = series labels (first cell empty), then one row per category: label, values…; an empty cell or a null is a blank value, a quoted number a label, a `|` in a label a line break; transpose?: true swaps rows and columns (Transpose row/column), switchXY?: true swaps each scatter series' y and x columns (Switch x/y)} replace the graph's data; no data → the current {csv, series, categories, rows}",
            has_selection,
            set_data
        ),
        cmd!(
            "graph.setType",
            "Type…",
            ["Object", "Graph"],
            None,
            "{id?, type?, seriesIndexes?: [index], columnWidth?: %, clusterWidth?: %, legend?: bool, markPoints?: bool, connectPoints?: bool, edgeToEdge?: bool (line graphs: true runs the lines across the whole plot, false puts the points at the centres of their categories), ticks?: n, axisMin?, axisMax?, valueAxis?: left|right|both (the bar graphs' value axis stays along the bottom), separateScales?: bool, rightTicks?: n, rightAxisMin?, rightAxisMax?, tickLength?: none|short|full, tickMarks?: n per division, rightTickLength?, rightTickMarks?, categoryTickLength?: none|short|full, categoryTickMarks?: n, ticksBetweenLabels?: bool, prefix?, suffix?, rightPrefix?, rightSuffix?: text around the value axis numbers, dropShadow?: bool (Add Drop Shadow: a translucent black shadow down and right of the columns, bars, lines and pie wedges), legendAcrossTop?: bool (Add Legend Across Top: the legend in rows above the plot), pieLegend?: none|standard|wedges (Legends in Wedges: the series labels inside their wedges), piePosition?: ratio|even|stacked (several pies sized by their totals, at one size (the default), or stacked), pieSort?: all|first|none (wedges largest first in each pie, in the first pie's order, or in data order (the default)); the query lists the pie options for pie graphs} change the graph type and options; with `seriesIndexes`, or (no `id`) with only series selected with Group Selection, `type` goes to those series only and `valueAxis` (left|right; both is refused) puts them on that value axis, while the other options still apply to the whole graph (Combine different graph types: column, stacked column, line and area mix, and so do bar and stacked bar; a series given the graph's type follows the graph again) (axisMin and axisMax together override the calculated value axis: exactly that range in `ticks` divisions, 5 when 0; with the value axis on both sides, separateScales gives the series on the right axis a scale of their own, set the same way with rightTicks, rightAxisMin and rightAxisMax); no options → the current ones",
            has_selection,
            set_type
        ),
        cmd!(
            "graph.design",
            "Design…",
            ["Object", "Graph"],
            None,
            "{save?: name (the selected art becomes a graph design; put a rectangle at the back to set the size it draws at), paste?: name (a copy of the design's art into the document, selected, to edit and save again), delete?: name (graphs drawing it go back to their default marks)}; no params → {designs: [names]}",
            has_doc,
            design
        ),
        cmd!(
            "graph.marker",
            "Marker…",
            ["Object", "Graph"],
            None,
            "{id?, seriesIndexes?: [index], design?: name|null} draw the data points and legend swatch of the series selected with Group Selection (or listed, or every series) with a graph design, scaled so its backmost object fills the default marker's square; null = the default marker. Line, scatter and radar series. No design → {designs: [names], design: the selected series' design or null}",
            has_selection,
            set_marker
        ),
    ]
}

// ---------- data ----------

/// One CSV cell: its text, and whether it was in straight quotes (a quoted number is a label, not a value).
struct Cell {
    text: String,
    quoted: bool,
}

impl Cell {
    fn number(&self) -> Option<f64> {
        if self.quoted { None } else { self.text.parse::<f64>().ok().filter(|v| v.is_finite()) }
    }
}

fn parse_csv(csv: &str) -> (Vec<String>, Vec<String>, Vec<Vec<Option<f64>>>) {
    let rows: Vec<Vec<Cell>> = csv
        .lines()
        .map(|l| l.trim_end_matches('\r'))
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            let sep = if l.contains('\t') { '\t' } else { ',' };
            l.split(sep)
                .map(|c| {
                    let c = c.trim();
                    let quoted = c.len() >= 2 && c.starts_with('"') && c.ends_with('"');
                    Cell { text: c.trim_matches('"').to_string(), quoted }
                })
                .collect()
        })
        .collect();
    let Some(first) = rows.first() else { return (vec![], vec![], vec![]) };
    // A label is quoted or doesn't read as a number; an empty unquoted cell is a blank value.
    let label = |c: &Cell| c.quoted || (c.number().is_none() && !c.text.is_empty());
    // A header row has an empty first cell or a label after it, or is a label followed only by empty cells (`Year,,`).
    // All-empty series names mean no legend.
    let empty = |c: &Cell| c.text.is_empty() && !c.quoted;
    let header = first.first().is_some_and(empty)
        || first.iter().skip(1).any(label)
        || (first.len() > 1 && first.first().is_some_and(label) && first.iter().skip(1).all(empty));
    let mut series: Vec<String> = if header { first.iter().skip(1).map(|c| c.text.clone()).collect() } else { vec![] };
    if series.iter().all(String::is_empty) {
        series.clear();
    }
    let data = rows.get(usize::from(header)..).unwrap_or_default();
    // The first column holds the category labels if any row starts with one; then it does for every row, so a
    // row with an empty label doesn't shift its values one place left.
    let labelled = data.iter().any(|r| r.first().is_some_and(label));
    let mut categories = vec![];
    let mut values = vec![];
    for r in data {
        if labelled {
            categories.push(r.first().map(|c| c.text.clone()).unwrap_or_default());
        }
        values.push(r.iter().skip(labelled as usize).map(Cell::number).collect());
    }
    (series, categories, values)
}

fn to_csv(g: &GraphSpec) -> String {
    // Always a header row (empty first cell), so a first data row starting with a blank isn't read as one.
    let cells = g.cells();
    let width = cells.iter().map(Vec::len).max().unwrap_or(0).max(g.series.len());
    let mut out = String::from(",");
    out.push_str(&(0..width).map(|s| g.series.get(s).cloned().unwrap_or_default()).collect::<Vec<_>>().join(","));
    out.push('\n');
    let labelled = !g.categories.is_empty();
    for (i, r) in cells.iter().enumerate() {
        if labelled {
            // An empty label, one that reads as a number or one with spaces around it goes in quotes, so it reads back
            // as the same label.
            let c = g.categories.get(i).map(String::as_str).unwrap_or_default();
            if c.trim().is_empty() || c.trim().parse::<f64>().is_ok() || c != c.trim() {
                out.push_str(&format!("\"{c}\""));
            } else {
                out.push_str(c);
            }
            out.push(',');
        }
        out.push_str(&r.iter().map(|v| v.map(fmt_value).unwrap_or_default()).collect::<Vec<_>>().join(","));
        out.push('\n');
    }
    out
}

fn apply_data(g: &mut GraphSpec, p: &Value) -> bool {
    let mut changed = false;
    if let Some(csv) = str_param(p, "csv") {
        let (s, c, r) = parse_csv(csv);
        g.series = s;
        g.categories = c;
        g.set_cells(r);
        changed = true;
    }
    let strings =
        |v: &Value| v.as_array().map(|a| a.iter().map(|x| x.as_str().map(str::to_string).unwrap_or_else(|| x.to_string())).collect::<Vec<_>>());
    if let Some(s) = p.get("series").and_then(strings) {
        g.series = s;
        changed = true;
    }
    if let Some(c) = p.get("categories").and_then(strings) {
        g.categories = c;
        changed = true;
    }
    if let Some(rows) = p.get("rows").and_then(Value::as_array) {
        // `null` (or anything that isn't a finite number) is a blank cell.
        g.set_cells(
            rows.iter().map(|r| r.as_array().map(|a| a.iter().map(|v| v.as_f64().filter(|v| v.is_finite())).collect()).unwrap_or_default()).collect(),
        );
        changed = true;
    }
    drop_empty_rows(g);
    changed
}

/// Drop rows with neither a value nor a label; a label-only row stays, its values blank.
fn drop_empty_rows(g: &mut GraphSpec) {
    let mut cells = g.cells();
    let keep: Vec<bool> = (0..cells.len())
        .map(|c| cells.get(c).is_some_and(|r| r.iter().any(Option::is_some)) || g.categories.get(c).is_some_and(|l| !l.is_empty()))
        .collect();
    if keep.contains(&false) || cells.len() < g.rows.len() {
        let mut k = keep.iter();
        cells.retain(|_| k.next().copied().unwrap_or(false));
        let mut k = keep.iter();
        g.categories.retain(|_| k.next().copied().unwrap_or(true));
        g.set_cells(cells);
    }
}

fn fmt_value(v: f64) -> String {
    if (v - v.round()).abs() < 1e-9 {
        format!("{}", v.round() as i64)
    } else {
        format!("{:.2}", v).trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

// ---------- generation ----------

/// Series colours: black, then greys (graphs start in greyscale).
fn default_series_color(i: usize) -> Color {
    const LEVELS: [f32; 8] = [0.0, 0.55, 0.8, 0.3, 0.68, 0.15, 0.9, 0.42];
    let l = LEVELS[i % LEVELS.len()];
    Color::rgb(l, l, l)
}

fn default_series_paint(i: usize) -> Paint {
    Paint::solid(default_series_color(i))
}

/// Fill and stroke for one generated mark of series `i`. A stored fill is used only where the
/// generator paints with the series colour (`series_fill`). A stored stroke replaces that part's
/// own stroke, so a column's stroke stays a stroke and a line's markers do not repaint its line.
fn series_marks(g: &GraphSpec, i: usize, series_fill: bool, stroke: Paint, width: f64) -> (Paint, Paint, f64) {
    let slot = g.series_paints.get(i);
    let fill = if series_fill { slot.and_then(|p| p.fill.clone()).unwrap_or_else(|| default_series_paint(i)) } else { Paint::None };
    let stroke = slot.and_then(|p| p.stroke.clone()).unwrap_or(stroke);
    let width = if slot.is_some_and(|p| p.stroke.is_some()) { slot.and_then(|p| p.stroke_width).unwrap_or(width) } else { width };
    (fill, stroke, width)
}

const LABEL_SIZE: f64 = 9.0;
/// Most tick marks per division (Graph Type › Tick Marks), and most category tick marks a graph draws.
const MAX_TICK_MARKS: usize = 20;
const MAX_CATEGORY_TICKS: usize = 10_000;
/// Longest axis label prefix or suffix (Graph Type › Add Labels).
const MAX_AFFIX: usize = 64;

/// An axis label prefix or suffix: one line (no control characters or line separators), at most [`MAX_AFFIX`] long.
fn one_line(s: &str) -> String {
    s.chars().filter(|c| !c.is_control() && !matches!(c, '\u{2028}' | '\u{2029}')).take(MAX_AFFIX).collect()
}

/// A count parameter: a whole number, also as the float a dialog sends back (3.0), at most `max`.
fn count_param(p: &Value, key: &str, max: u64) -> Option<usize> {
    let v = p.get(key)?;
    let n = v.as_u64().or_else(|| v.as_f64().filter(|x| x.is_finite() && *x >= 0.0).map(|x| x.round().min(max as f64) as u64))?;
    usize::try_from(n.min(max)).ok()
}

/// A "nice" axis: (min, max, step).
fn nice_axis(lo: f64, hi: f64, ticks: usize) -> (f64, f64, f64) {
    let (lo, hi) = if (hi - lo).abs() < 1e-12 { (lo.min(0.0), lo.max(0.0) + 1.0) } else { (lo, hi) };
    let n = if ticks == 0 { 5 } else { ticks.clamp(1, 100) } as f64;
    let raw = (hi - lo) / n;
    let mag = 10f64.powf(raw.log10().floor());
    let step = [1.0, 2.0, 2.5, 5.0, 10.0].iter().map(|m| m * mag).find(|s| *s >= raw - 1e-12).unwrap_or(10.0 * mag);
    ((lo / step).floor() * step, (hi / step).ceil() * step, step)
}

/// A value axis: with both `min` and `max` set (Graph Type › Tick Values › Override Calculated Values), exactly that
/// range split into `ticks` divisions (5 when automatic); otherwise a nice axis around the data, using whichever bound
/// was given.
fn value_axis(ticks: usize, axis_min: Option<f64>, axis_max: Option<f64>, lo: f64, hi: f64) -> (f64, f64, f64) {
    match (axis_min, axis_max) {
        (Some(min), Some(max)) if max > min && (max - min).is_finite() => {
            let n = if ticks == 0 { 5 } else { ticks.clamp(1, 100) } as f64;
            (min, max, (max - min) / n)
        }
        _ => {
            // An override too wide for f64 (or NaN) gives a non-finite axis, which would reach `clamp` with NaN bounds
            // and panic: fall back to the data, then to 0..1.
            let finite = |(a, b, s): (f64, f64, f64)| a.is_finite() && b.is_finite() && s.is_finite() && b > a && s > 0.0;
            [nice_axis(axis_min.unwrap_or(lo), axis_max.unwrap_or(hi), ticks), nice_axis(lo, hi, ticks)]
                .into_iter()
                .find(|&a| finite(a))
                .unwrap_or((0.0, 1.0, 0.2))
        }
    }
}

/// The tick values from `lo` to `hi` every `step`, both ends included: counted, not accumulated, so the last tick
/// lands on `hi` and a step too small for the range (or not finite) can't loop forever. At most 1000 ticks.
fn tick_values(lo: f64, hi: f64, step: f64) -> Vec<f64> {
    let n = ((hi - lo) / step).round();
    if !(lo.is_finite() && step.is_finite() && step > 0.0 && n.is_finite() && n >= 0.0) {
        return vec![];
    }
    (0..=(n as usize).min(1000)).map(|i| lo + i as f64 * step).collect()
}

struct Gen<'a> {
    d: &'a mut Document,
    out: Vec<Arc<Node>>,
}

impl Gen<'_> {
    fn path(&mut self, pd: PathData, fill: Paint, stroke: Paint, w: f64) -> Arc<Node> {
        Arc::new(Node::path(self.d.alloc_id(), pd, Appearance::basic(fill, stroke, w)))
    }
    fn line(&mut self, a: Point, b: Point) -> Arc<Node> {
        let mut bp = BezPath::new();
        bp.move_to(a);
        bp.line_to(b);
        self.path(PathData::from_bezpath(&bp), Paint::None, Paint::solid(Color::BLACK), 0.5)
    }
    fn text(&mut self, at: Point, s: &str, justify: Justify) -> Arc<Node> {
        let style = CharStyle { size: LABEL_SIZE, fill: Paint::solid(Color::BLACK), ..CharStyle::default() };
        let mut t = TextObject::point(at, s, style);
        t.para.justify = justify;
        refresh_bounds(&mut t);
        Arc::new(Node::new(self.d.alloc_id(), NodeKind::Text(Box::new(t))))
    }
    /// A category or series label: a `|` in it starts a new line (Add graph labels). `middle`: the lines are centred
    /// on `at` (labels beside an axis or in a wedge) instead of running down from it.
    fn label(&mut self, at: Point, s: &str, justify: Justify, middle: bool) -> Arc<Node> {
        let lines = s.split('|').count();
        let at = if middle { Point::new(at.x, at.y - (lines - 1) as f64 * LABEL_SIZE * 1.2 / 2.0) } else { at };
        self.text(at, &s.replace('|', "\n"), justify)
    }
    fn group(&mut self, name: &str, children: Vec<Arc<Node>>) -> Arc<Node> {
        let mut g = Node::group(self.d.alloc_id(), children);
        g.name = Some(name.into());
        Arc::new(g)
    }
    /// A data point's marker filling `cell`: the series' marker design, else the default square in the series' paint.
    fn marker(&mut self, design: Option<&Arc<Node>>, cell: Rect, (fill, stroke, w): (Paint, Paint, f64)) -> Arc<Node> {
        match design.and_then(|a| fit_design(self.d, a, cell)) {
            Some(n) => n,
            None => self.path(shapes::rectangle(cell), fill, stroke, w),
        }
    }
    /// A series group. Empty series are omitted by the caller; the index is stored on the group
    /// so later paint edits find the series without counting siblings.
    fn series(&mut self, name: &str, index: usize, children: Vec<Arc<Node>>) -> Arc<Node> {
        let mut g = Node::group(self.d.alloc_id(), children);
        g.name = Some(name.into());
        g.series_index = u32::try_from(index).ok();
        Arc::new(g)
    }
}

/// Add Drop Shadow: how far the shadow sits down and right of its mark, and how dark it is.
const SHADOW_OFFSET: f64 = 2.0;
const SHADOW_OPACITY: f32 = 0.35;
/// Most marks a graph draws shadows for (each is a copy).
const SHADOW_BUDGET: usize = 200_000;

/// Mark `n`'s shadow: a black copy moved down and right, its fill and stroke (whichever it has) black.
fn shadow_of(d: &mut Document, n: &Node) -> Arc<Node> {
    // Generated marks carry only paint and a stroke width, so that's all a shadow keeps.
    let mut c = n.clone();
    c.transform(Affine::translate((SHADOW_OFFSET, SHADOW_OFFSET)), false);
    let fill = if c.appearance.fill_paint().is_none() { Paint::None } else { Paint::solid(Color::BLACK) };
    let stroke = if c.appearance.stroke_paint().is_none() { Paint::None } else { Paint::solid(Color::BLACK) };
    c.appearance = Appearance::basic(fill, stroke, c.appearance.stroke_width());
    Arc::new(d.reid(&c))
}

/// How wide label `s` is set (its widest line), without adding it to a document.
fn label_width(s: &str) -> f64 {
    let style = CharStyle { size: LABEL_SIZE, fill: Paint::solid(Color::BLACK), ..CharStyle::default() };
    let mut t = TextObject::point(Point::ZERO, &s.replace('|', "\n"), style);
    refresh_bounds(&mut t);
    Node::new(NodeId(0), NodeKind::Text(Box::new(t))).geometric_bounds().map_or(0.0, |b| b.width())
}

/// A label with nothing to show (empty, or only `|` line breaks).
fn blank_label(s: &str) -> bool {
    s.chars().all(|c| c == '|')
}

/// Text node `t` with its characters painted `colour`.
fn recolour_text(t: &Arc<Node>, colour: Color) -> Arc<Node> {
    let mut n = (**t).clone();
    if let NodeKind::Text(text) = &mut n.kind {
        for run in &mut text.runs {
            run.style.fill = Paint::solid(colour);
        }
    }
    Arc::new(n)
}

/// The stretches of two or more points between blank cells, which a line connects.
fn runs(pts: &[Option<Point>]) -> Vec<Vec<Point>> {
    pts.split(Option::is_none).map(|r| r.iter().flatten().copied().collect::<Vec<_>>()).filter(|r| r.len() > 1).collect()
}

fn polyline(pts: &[Point], closed: bool) -> PathData {
    let mut bp = BezPath::new();
    for (i, p) in pts.iter().enumerate() {
        if i == 0 {
            bp.move_to(*p);
        } else {
            bp.line_to(*p);
        }
    }
    if closed {
        bp.close_path();
    }
    PathData::from_bezpath(&bp)
}

/// Most objects a graph's marker designs may add (each data point and legend swatch draws a copy of its design).
const MARKER_BUDGET: usize = 200_000;

/// Each series' marker design art, when the document has the design.
fn marker_designs(d: &Document, g: &GraphSpec) -> Vec<Option<Arc<Node>>> {
    let nser = g.rows.iter().map(Vec::len).max().unwrap_or(0).clamp(1, vectorcraft_doc::MAX_GRAPH_SERIES);
    (0..nser)
        .map(|s| {
            let name = g.series_markers.get(s)?.as_deref()?;
            d.graph_designs.iter().find(|x| x.name == name).map(|x| x.art.clone())
        })
        .collect()
}

/// How many objects the marker designs `designs` add to graph `g`: a copy per data point and legend swatch.
fn marker_cost(g: &GraphSpec, designs: &[Option<Arc<Node>>]) -> usize {
    let points = g.rows.len().min(vectorcraft_doc::MAX_GRAPH_CATEGORIES) + 1;
    designs.iter().flatten().map(|a| a.count().saturating_mul(points)).fold(0, usize::saturating_add)
}

/// Graph design `art` scaled into `cell` (Object › Graph › Marker): its backmost object, the rectangle a design is
/// drawn around, fills the cell (the whole art does when the design is a single object), strokes scaling with it.
/// Fresh ids. `None` for art with no area.
fn fit_design(d: &mut Document, art: &Node, cell: Rect) -> Option<Arc<Node>> {
    let usable = |b: &Rect| b.width() > 1e-9 && b.height() > 1e-9 && b.width().is_finite() && b.height().is_finite();
    let group = matches!(art.kind, NodeKind::Group { .. });
    let backmost = art.children().filter(|_| group).and_then(|c| c.first()).and_then(|c| c.geometric_bounds()).filter(usable);
    let frame = backmost.or_else(|| art.geometric_bounds().filter(usable))?;
    let a = Affine::translate(cell.origin().to_vec2())
        * Affine::scale_non_uniform(cell.width() / frame.width(), cell.height() / frame.height())
        * Affine::translate(-frame.origin().to_vec2());
    // Strokes, effects and pattern tiles scale with the art.
    let art = super::place::transformed(art.clone(), a);
    Some(Arc::new(d.reid(&art)))
}

/// Build the graph's children.
fn generate(d: &mut Document, g: &GraphSpec) -> Vec<Arc<Node>> {
    let mark = |i: usize, series_fill: bool, stroke: Paint, width: f64| series_marks(g, i, series_fill, stroke, width);
    // Each series' marker design (Object › Graph › Marker), when the document has it; default squares for all
    // when the designs would draw more than MARKER_BUDGET objects.
    let mut designs = marker_designs(d, g);
    if marker_cost(g, &designs) > MARKER_BUDGET {
        designs.clear();
    }
    let design = |s: usize| designs.get(s).and_then(Option::as_ref);
    let mut b = Gen { d, out: vec![] };
    let r = g.rect;
    let nser = g.rows.iter().map(Vec::len).max().unwrap_or(0).max(1);
    let ncat = g.rows.len().max(1);
    let series_label = |i: usize| g.series.get(i).cloned().unwrap_or_default();
    let mut series: Vec<Vec<Arc<Node>>> = vec![vec![]; nser];
    // A blank cell (no value) is `None`: columns, bars and stacked segments leave it out, lines and scatter points
    // break around it and the value axis ignores it. Area, pie and radar draw it as 0, which is what it adds to them.
    let cells = g.cells();
    let cell = |c: usize, s: usize| cells.get(c).and_then(|r| r.get(s)).copied().flatten();
    let val = |c: usize, s: usize| cell(c, s).unwrap_or(0.0);
    // Each series' own type (Combine different graph types); one type for the whole graph unless a series has its own.
    let kinds: Vec<GraphKind> = (0..nser).map(|s| g.series_kind(s)).collect();
    let kind = |s: usize| kinds.get(s).copied().unwrap_or(g.kind);
    let horizontal = matches!(g.kind, GraphKind::Bar | GraphKind::StackedBar);
    // Where the art right of the plot ends (the right value axis' labels), so the legend starts after it.
    let mut right_edge = r.x1;

    // Labels drawn over the series: a pie's Legends in Wedges (in the Legend group) and stacked pies' names.
    let mut wedge_labels = vec![];
    let mut on_top = vec![];
    // Add Drop Shadow: the columns, bars, lines and wedges to cast one.
    let mut shadowed: Vec<(usize, Arc<Node>)> = vec![];
    match g.kind {
        GraphKind::Pie => {
            // One pie per row, a wedge per series, clockwise from 12 o'clock. Position: side by side at one size
            // (Even) or sized by their totals (Ratio, by area), or stacked on one centre, the largest at the back.
            let totals: Vec<f64> = (0..ncat).map(|c| (0..nser).map(|s| val(c, s).max(0.0)).sum()).collect();
            let biggest = totals.iter().copied().fold(0.0, f64::max);
            let stacked = g.pie_position == vectorcraft_doc::PiePosition::Stacked;
            let w = if stacked { r.width() } else { r.width() / ncat as f64 };
            let full = (w.min(r.height()) / 2.0 * 0.9).max(1.0);
            let radius = |total: f64| match g.pie_position {
                vectorcraft_doc::PiePosition::Even => full,
                _ if biggest > 0.0 && biggest.is_finite() => (full * (total / biggest).sqrt()).max(1.0),
                _ => full,
            };
            // Sort: data order, largest first in each pie, or the first pie's largest-first order everywhere.
            let order = |c: usize| {
                let mut o: Vec<usize> = (0..nser).collect();
                let by = match g.pie_sort {
                    vectorcraft_doc::PieSort::None => return o,
                    vectorcraft_doc::PieSort::All => c,
                    vectorcraft_doc::PieSort::First => 0,
                };
                o.sort_by(|a, b| val(by, *b).max(0.0).total_cmp(&val(by, *a).max(0.0)));
                o
            };
            let mut pies: Vec<usize> = (0..ncat).collect();
            if stacked {
                pies.sort_by(|a, b| totals.get(*b).copied().unwrap_or(0.0).total_cmp(&totals.get(*a).copied().unwrap_or(0.0)));
            }
            // Stacked pies share a centre: each one but the smallest is drawn as the ring outside the next smaller
            // one, so a pie never covers another (a pie as big as the next one has no ring and is left out).
            let inner: Vec<f64> = if stacked {
                (0..pies.len()).map(|k| pies.get(k + 1).map_or(0.0, |c| radius(totals.get(*c).copied().unwrap_or(0.0)))).collect()
            } else {
                vec![0.0; pies.len()]
            };
            for (k, c) in pies.into_iter().enumerate() {
                let total = totals.get(c).copied().unwrap_or(0.0);
                let rad = radius(total);
                let hole = inner.get(k).copied().unwrap_or(0.0);
                if hole >= rad - 1e-9 {
                    continue;
                }
                let centre = if stacked { r.center() } else { Point::new(r.x0 + w * (c as f64 + 0.5), r.y0 + r.height() / 2.0) };
                let mut a0 = -std::f64::consts::FRAC_PI_2;
                for s in order(c) {
                    let v = val(c, s).max(0.0);
                    if total <= 0.0 || v <= 0.0 {
                        continue;
                    }
                    let sweep = v / total * std::f64::consts::TAU;
                    let mut bp = BezPath::new();
                    let dir = |a: f64| vectorcraft_geom::Vec2::new(a.cos(), a.sin());
                    let arc = vectorcraft_geom::kurbo::Arc::new(centre, (rad, rad), a0, sweep, 0.0);
                    if hole > 0.0 {
                        // A ring segment: out along the outer arc, back along the inner one.
                        bp.move_to(centre + dir(a0) * rad);
                        arc.to_cubic_beziers(0.1, |p1, p2, p| bp.curve_to(p1, p2, p));
                        bp.line_to(centre + dir(a0 + sweep) * hole);
                        let back = vectorcraft_geom::kurbo::Arc::new(centre, (hole, hole), a0 + sweep, -sweep, 0.0);
                        back.to_cubic_beziers(0.1, |p1, p2, p| bp.curve_to(p1, p2, p));
                    } else {
                        bp.move_to(centre);
                        bp.line_to(centre + dir(a0) * rad);
                        arc.to_cubic_beziers(0.1, |p1, p2, p| bp.curve_to(p1, p2, p));
                    }
                    bp.close_path();
                    let (fill, stroke, sw) = mark(s, true, Paint::solid(Color::WHITE), 0.5);
                    // White text on a dark wedge.
                    let dark = fill.color().is_some_and(|c| {
                        let [r, g, b] = c.to_rgb();
                        0.2126 * r + 0.7152 * g + 0.0722 * b < 0.5
                    });
                    let n = b.path(PathData::from_bezpath(&bp), fill, stroke, sw);
                    shadowed.push((s, n.clone()));
                    if let Some(items) = series.get_mut(s) {
                        items.push(n);
                    }
                    // Legends in Wedges: the series label inside its wedge, centred two thirds of the way out.
                    if g.legend && g.pie_legend_in_wedges {
                        let label = series_label(s);
                        if !blank_label(&label) {
                            // Two thirds of the way out (of the ring, for a stacked pie).
                            let mid = a0 + sweep / 2.0;
                            let at = centre + dir(mid) * (hole + (rad - hole) * 0.62);
                            let t = b.label(Point::new(at.x, at.y + LABEL_SIZE * 0.35), &label, Justify::Center, true);
                            wedge_labels.push(if dark { recolour_text(&t, Color::WHITE) } else { t });
                        }
                    }
                    a0 += sweep;
                }
                if let Some(cat) = g.categories.get(c).filter(|c| !c.is_empty()) {
                    if stacked {
                        // Just above its own outline, all its lines, and drawn over the pies.
                        let lines = cat.split('|').count() as f64;
                        let at = Point::new(centre.x, centre.y - rad - LABEL_SIZE * 0.4 - (lines - 1.0) * LABEL_SIZE * 1.2);
                        let t = b.label(at, cat, Justify::Center, false);
                        on_top.push(t);
                    } else {
                        let t = b.label(Point::new(centre.x, centre.y + rad + LABEL_SIZE * 1.6), cat, Justify::Center, false);
                        b.out.push(t);
                    }
                }
            }
        }
        GraphKind::Radar => {
            let centre = r.center();
            let rad = (r.width().min(r.height()) / 2.0).max(1.0);
            let (lo, hi, step) = value_axis(g.ticks, g.axis_min, g.axis_max, 0.0, g.rows.iter().flatten().copied().fold(0.0, f64::max));
            let at = |c: usize, v: f64| {
                let a = -std::f64::consts::FRAC_PI_2 + std::f64::consts::TAU * c as f64 / ncat as f64;
                centre + vectorcraft_geom::Vec2::new(a.cos(), a.sin()) * (rad * ((v - lo) / (hi - lo)).clamp(0.0, 1.0))
            };
            let mut axes = vec![];
            for c in 0..ncat {
                axes.push(b.line(centre, at(c, hi)));
                if let Some(cat) = g.categories.get(c).filter(|c| !c.is_empty()) {
                    let p = at(c, hi) + (at(c, hi) - centre).normalize() * (LABEL_SIZE * 1.2);
                    axes.push(b.label(Point::new(p.x, p.y + LABEL_SIZE * 0.35), cat, Justify::Center, true));
                }
            }
            for v in tick_values(lo, hi, step).into_iter().skip(1) {
                let ring: Vec<Point> = (0..ncat).map(|c| at(c, v)).collect();
                axes.push(b.path(polyline(&ring, true), Paint::None, Paint::solid(Color::rgb(0.6, 0.6, 0.6)), 0.25));
            }
            let ax = b.group("Axes", axes);
            b.out.push(ax);
            for (s, items) in series.iter_mut().enumerate() {
                let pts: Vec<Point> = (0..ncat).map(|c| at(c, val(c, s))).collect();
                if g.connect_points && pts.len() > 1 {
                    let (fill, stroke, w) = mark(s, false, default_series_paint(s), 1.0);
                    let n = b.path(polyline(&pts, true), fill, stroke, w);
                    shadowed.push((s, n.clone()));
                    items.push(n);
                }
                if g.mark_points {
                    for p in pts {
                        let paint = mark(s, true, Paint::None, 0.0);
                        items.push(b.marker(design(s), Rect::from_center_size(p, (4.0, 4.0)), paint));
                    }
                }
            }
        }
        _ => {
            let scatter = g.kind == GraphKind::Scatter;
            // Separate Scales: with the value axis on both sides, the series assigned to the right axis get a scale
            // of their own there (axis 1); every other series, and every series without separate scales, uses axis 0.
            let separate = g.separate_scales && g.value_axis == ValueAxisSide::Both && !horizontal && !scatter;
            let mut on_right = vec![false; nser];
            if separate {
                for i in &g.right_series {
                    if let Some(x) = on_right.get_mut(*i) {
                        *x = true;
                    }
                }
            }
            let axis = |s: usize| usize::from(on_right.get(s).copied().unwrap_or(false));
            // Value range per axis (stacked graphs stack positives and negatives separately).
            let (mut lo, mut hi) = ([0.0f64; 2], [0.0f64; 2]);
            let (mut xlo, mut xhi) = (f64::MAX, f64::MIN);
            for c in 0..ncat {
                if !scatter {
                    for (a, (lo, hi)) in lo.iter_mut().zip(hi.iter_mut()).enumerate() {
                        // Stacked columns (or bars) stack together, and so do areas; other series count one value each.
                        for stack in [[GraphKind::StackedColumn, GraphKind::StackedBar], [GraphKind::Area, GraphKind::Area]] {
                            let members = || (0..nser).filter(|s| axis(*s) == a && stack.contains(&kind(*s)));
                            *hi = hi.max(members().map(|s| val(c, s).max(0.0)).sum());
                            *lo = lo.min(members().map(|s| val(c, s).min(0.0)).sum());
                        }
                        let single = |s: &usize| axis(*s) == a && matches!(kind(*s), GraphKind::Column | GraphKind::Bar | GraphKind::Line);
                        for v in (0..nser).filter(single).filter_map(|s| cell(c, s)) {
                            *hi = hi.max(v);
                            *lo = lo.min(v);
                        }
                    }
                } else {
                    for s in (0..nser).step_by(2) {
                        if let (Some(y), Some(x)) = (cell(c, s), cell(c, s + 1)) {
                            hi[0] = hi[0].max(y);
                            lo[0] = lo[0].min(y);
                            xlo = xlo.min(x);
                            xhi = xhi.max(x);
                        }
                    }
                }
            }
            // An axis with no series of its own shows the other one's scale.
            let left = value_axis(g.ticks, g.axis_min, g.axis_max, lo[0], hi[0]);
            let used = |a: usize| (0..nser).any(|s| axis(s) == a);
            let (left, right) = if !separate || !used(1) {
                (left, left)
            } else {
                let right = value_axis(g.right_ticks, g.right_axis_min, g.right_axis_max, lo[1], hi[1]);
                if used(0) { (left, right) } else { (right, right) }
            };
            let scale = |a: usize| if a == 1 { right } else { left };
            // Value → coordinate along value axis `a` (y for columns/lines, x for bars), and that axis' zero (or the
            // end nearest it).
            let vpos = |a: usize, v: f64| {
                let (lo, hi, _) = scale(a);
                let t = (v - lo) / (hi - lo);
                if horizontal { r.x0 + t * r.width() } else { r.y1 - t * r.height() }
            };
            let zero = |a: usize| {
                let (lo, hi, _) = scale(a);
                vpos(a, 0.0f64.clamp(lo, hi))
            };
            let mut axes = vec![];
            // Value axes with ticks and labels: along the bottom for bar graphs, else on the chosen sides (the right
            // one shows the right scale).
            let (on_left_side, on_right_side) =
                if horizontal { (true, false) } else { (g.value_axis != ValueAxisSide::Right, g.value_axis != ValueAxisSide::Left) };
            // Add Labels text per axis, kept to one short line whatever the file holds.
            let affix = |s: &str| one_line(s);
            let affixes = [(affix(&g.prefix), affix(&g.suffix)), (affix(&g.right_prefix), affix(&g.right_suffix))];
            // Each label's tick mark, then the extra tick marks of its division (Tick Marks per division).
            let value_ticks = |b: &mut Gen, axes: &mut Vec<Arc<Node>>, a: usize, v: f64, step: f64, hi: f64| {
                let (len, marks) = if a == 1 { (g.right_tick_length, g.right_tick_marks) } else { (g.tick_length, g.tick_marks) };
                let n = marks.clamp(1, MAX_TICK_MARKS);
                for k in 0..n {
                    let w = v + step * k as f64 / n as f64;
                    if k > 0 && w >= hi - step * 1e-9 {
                        break;
                    }
                    let q = vpos(a, w);
                    // A full-length tick on the zero line, or repeating the left side's on one shared scale, adds nothing.
                    let repeat = a == 1 && on_left_side && g.tick_length == TickLength::Full && right == left;
                    let len = if len == TickLength::Full && ((q - zero(a)).abs() < 1e-9 || repeat) { TickLength::None } else { len };
                    let ends = match (len, horizontal, a == 1) {
                        (TickLength::None, ..) => None,
                        (TickLength::Short, true, _) => Some((Point::new(q, r.y1), Point::new(q, r.y1 + 4.0))),
                        (TickLength::Short, false, false) => Some((Point::new(r.x0 - 4.0, q), Point::new(r.x0, q))),
                        (TickLength::Short, false, true) => Some((Point::new(r.x1, q), Point::new(r.x1 + 4.0, q))),
                        (TickLength::Full, true, _) => Some((Point::new(q, r.y0), Point::new(q, r.y1))),
                        (TickLength::Full, false, _) => Some((Point::new(r.x0, q), Point::new(r.x1, q))),
                    };
                    if let Some((p0, p1)) = ends {
                        axes.push(b.line(p0, p1));
                    }
                    if k == 0 {
                        // The label goes right after its own tick mark.
                        let (prefix, suffix) = affixes.get(a).map_or(("", ""), |(p, s)| (p.as_str(), s.as_str()));
                        let label = format!("{prefix}{}{suffix}", fmt_value(v));
                        let t = if horizontal {
                            b.text(Point::new(q, r.y1 + 4.0 + LABEL_SIZE * 1.1), &label, Justify::Center)
                        } else if a == 1 {
                            b.text(Point::new(r.x1 + 6.0, q + LABEL_SIZE * 0.35), &label, Justify::Left)
                        } else {
                            b.text(Point::new(r.x0 - 6.0, q + LABEL_SIZE * 0.35), &label, Justify::Right)
                        };
                        axes.push(t);
                    }
                }
            };
            if on_left_side {
                let (lo, hi, step) = left;
                for v in tick_values(lo, hi, step) {
                    value_ticks(&mut b, &mut axes, 0, v, step, hi);
                }
            }
            if on_right_side {
                let (lo, hi, step) = right;
                for v in tick_values(lo, hi, step) {
                    value_ticks(&mut b, &mut axes, 1, v, step, hi);
                }
            }
            // The legend starts after the value labels right of the plot: the right axis' (14 points on), or a bar
            // graph's last bottom label running past the plot (6 points on; a short number stays within the gap).
            for t in &axes {
                if matches!(t.kind, NodeKind::Text(_))
                    && let Some(bb) = t.geometric_bounds()
                    && bb.x1 > r.x1
                {
                    right_edge = right_edge.max(if bb.x0 > r.x1 { bb.x1 } else { bb.x1 - 8.0 });
                }
            }
            if horizontal {
                axes.push(b.line(Point::new(r.x0, r.y1), Point::new(r.x1, r.y1)));
                axes.push(b.line(Point::new(zero(0), r.y0), Point::new(zero(0), r.y1)));
            } else {
                if on_left_side {
                    axes.push(b.line(Point::new(r.x0, r.y0), Point::new(r.x0, r.y1)));
                }
                if on_right_side {
                    axes.push(b.line(Point::new(r.x1, r.y0), Point::new(r.x1, r.y1)));
                }
                axes.push(b.line(Point::new(r.x0, zero(0)), Point::new(r.x1, zero(0))));
                // The right scale's zero, where its columns start, when it isn't the left one's.
                if (zero(1) - zero(0)).abs() > 1e-9 {
                    axes.push(b.line(Point::new(r.x0, zero(1)), Point::new(r.x1, zero(1))));
                }
            }
            // Category positions.
            let span = if horizontal { r.height() } else { r.width() };
            let cat_w = span / ncat as f64;
            let cat_start = |c: usize| if horizontal { r.y0 + c as f64 * cat_w } else { r.x0 + c as f64 * cat_w };
            // Area bands always span the plot; lines only with Edge-to-Edge Lines. Category labels sit where every
            // series puts its points: at the edges when all of them span the plot, else at the category centres.
            let spans = |k: GraphKind| k == GraphKind::Area || (k == GraphKind::Line && g.edge_to_edge);
            let mid = |c: usize, edges: bool| {
                if edges && ncat > 1 { r.x0 + r.width() * c as f64 / (ncat - 1) as f64 } else { cat_start(c) + cat_w / 2.0 }
            };
            let label_edges = kinds.iter().all(|k| spans(*k));
            let cat_mid = |c: usize| mid(c, label_edges);
            if !scatter {
                for c in 0..ncat {
                    if let Some(cat) = g.categories.get(c).filter(|c| !c.is_empty()) {
                        let t = if horizontal {
                            b.label(Point::new(r.x0 - 6.0, cat_mid(c) + LABEL_SIZE * 0.35), cat, Justify::Right, true)
                        } else {
                            b.label(Point::new(cat_mid(c), r.y1 + LABEL_SIZE * 1.4), cat, Justify::Center, false)
                        };
                        axes.push(t);
                    }
                }
                // Category tick marks (none by default): at the labels, or between them (at the category edges),
                // with the extra tick marks of each division between those.
                let len = g.category_tick_length;
                if len != TickLength::None {
                    let shown = ncat.min(MAX_CATEGORY_TICKS);
                    let base: Vec<f64> = match (g.ticks_between_labels, label_edges) {
                        // Labels at the plot's edges: halfway between neighbouring labels.
                        (true, true) => (1..shown).map(|c| (cat_mid(c - 1) + cat_mid(c)) / 2.0).collect(),
                        (true, false) => (0..=shown).map(cat_start).collect(),
                        (false, _) => (0..shown).map(cat_mid).collect(),
                    };
                    // Fewer per division on a long axis, so the cap still ticks all of it.
                    let n = g.category_tick_marks.clamp(1, MAX_TICK_MARKS).min(MAX_CATEGORY_TICKS / base.len().max(1)).max(1);
                    let at = base.iter().enumerate().flat_map(|(i, q)| {
                        let next = base.get(i + 1).copied();
                        (0..n).filter_map(move |k| if k == 0 { Some(*q) } else { next.map(|x| q + (x - q) * k as f64 / n as f64) })
                    });
                    // A full-length tick on a drawn axis line adds nothing.
                    let on_axis_line = |q: f64| {
                        if horizontal {
                            (q - r.y1).abs() < 1e-9
                        } else {
                            ((q - r.x0).abs() < 1e-9 && on_left_side) || ((q - r.x1).abs() < 1e-9 && on_right_side)
                        }
                    };
                    for q in at.take(MAX_CATEGORY_TICKS) {
                        if len == TickLength::Full && on_axis_line(q) {
                            continue;
                        }
                        let (p0, p1) = match (len, horizontal) {
                            (TickLength::Full, false) => (Point::new(q, r.y0), Point::new(q, r.y1)),
                            (TickLength::Full, true) => (Point::new(r.x0, q), Point::new(r.x1, q)),
                            (_, false) => (Point::new(q, r.y1), Point::new(q, r.y1 + 4.0)),
                            (_, true) => (Point::new(r.x0 - 4.0, q), Point::new(r.x0, q)),
                        };
                        axes.push(b.line(p0, p1));
                    }
                }
            } else {
                // Scatter: a horizontal value axis too.
                let (xl, xh, xs) = nice_axis(if xlo == f64::MAX { 0.0 } else { xlo.min(0.0) }, if xhi == f64::MIN { 1.0 } else { xhi }, g.ticks);
                let mut x = xl;
                while x <= xh + xs * 1e-6 {
                    let q = r.x0 + (x - xl) / (xh - xl) * r.width();
                    axes.push(b.line(Point::new(q, r.y1), Point::new(q, r.y1 + 4.0)));
                    axes.push(b.text(Point::new(q, r.y1 + 4.0 + LABEL_SIZE * 1.1), &fmt_value(x), Justify::Center));
                    x += xs;
                }
                for (si, s) in (0..nser).step_by(2).enumerate() {
                    let pts: Vec<Option<Point>> = (0..ncat)
                        .map(|c| match (cell(c, s), cell(c, s + 1)) {
                            (Some(y), Some(x)) => Some(Point::new(r.x0 + (x - xl) / (xh - xl) * r.width(), vpos(0, y))),
                            _ => None,
                        })
                        .collect();
                    if g.connect_points {
                        for run in runs(&pts) {
                            let (fill, stroke, w) = mark(si, false, default_series_paint(si), 1.0);
                            let n = b.path(polyline(&run, false), fill, stroke, w);
                            shadowed.push((si, n.clone()));
                            series[si].push(n);
                        }
                    }
                    if g.mark_points {
                        for p in pts.iter().flatten() {
                            let paint = mark(si, true, Paint::None, 0.0);
                            let n = b.marker(design(si), Rect::from_center_size(*p, (5.0, 5.0)), paint);
                            series[si].push(n);
                        }
                    }
                }
            }
            let ax = b.group("Axes", axes);
            b.out.push(ax);
            // Columns (or bars) of one category side by side; stacked columns take one slot of that cluster, or, with
            // no plain columns, a width of their own.
            let columns: Vec<usize> = (0..nser).filter(|s| matches!(kind(*s), GraphKind::Column | GraphKind::Bar)).collect();
            let stacks: Vec<usize> = (0..nser).filter(|s| matches!(kind(*s), GraphKind::StackedColumn | GraphKind::StackedBar)).collect();
            // One stack per value axis that has stacked series, each in a slot of its own.
            let stack_axes: Vec<usize> = (0..2).filter(|a| stacks.iter().any(|s| axis(*s) == *a)).collect();
            let own_width = columns.is_empty() && stack_axes.len() < 2;
            let slots = columns.len() + stack_axes.len();
            let cluster = cat_w * (g.cluster_width / 100.0).clamp(0.01, 1.0);
            let slot = cluster / slots.max(1) as f64;
            let bar = slot * (g.column_width / 100.0).clamp(0.01, 1.0);
            let bar_rect = |a: f64, w: f64, v0: f64, v1: f64| {
                if horizontal { Rect::new(v0.min(v1), a, v0.max(v1), a + w) } else { Rect::new(a, v0.min(v1), a + w, v0.max(v1)) }
            };
            for c in 0..ncat {
                let c0 = cat_start(c) + (cat_w - cluster) / 2.0;
                for (i, &s) in columns.iter().enumerate() {
                    let Some(v) = cell(c, s) else { continue };
                    let a = c0 + slot * i as f64 + (slot - bar) / 2.0;
                    let (fill, stroke, w) = mark(s, true, Paint::None, 0.0);
                    let n = b.path(shapes::rectangle(bar_rect(a, bar, zero(axis(s)), vpos(axis(s), v))), fill, stroke, w);
                    shadowed.push((s, n.clone()));
                    if let Some(items) = series.get_mut(s) {
                        items.push(n);
                    }
                }
                let place = |ax: usize| {
                    if own_width {
                        let w = (cat_w * (g.column_width / 100.0).clamp(0.01, 1.0) * (g.cluster_width / 100.0).clamp(0.01, 1.0) * 1.2).min(cat_w);
                        (cat_start(c) + (cat_w - w) / 2.0, w)
                    } else {
                        let i = columns.len() + stack_axes.iter().position(|a| *a == ax).unwrap_or(0);
                        (c0 + slot * i as f64 + (slot - bar) / 2.0, bar)
                    }
                };
                // Each axis stacks its own series.
                let (mut pos, mut neg) = ([0.0f64; 2], [0.0f64; 2]);
                for &s in &stacks {
                    let Some(v) = cell(c, s) else { continue };
                    let ax = axis(s);
                    let (a, w) = place(ax);
                    let Some(base) = (if v >= 0.0 { pos.get_mut(ax) } else { neg.get_mut(ax) }) else { continue };
                    let (v0, v1) = (vpos(ax, *base), vpos(ax, *base + v));
                    *base += v;
                    let (fill, stroke, sw) = mark(s, true, Paint::solid(Color::WHITE), 0.25);
                    let n = b.path(shapes::rectangle(bar_rect(a, w, v0, v1)), fill, stroke, sw);
                    // A segment of 0 has no area to shade.
                    if v != 0.0 {
                        shadowed.push((s, n.clone()));
                    }
                    if let Some(items) = series.get_mut(s) {
                        items.push(n);
                    }
                }
            }
            // Cumulative area bands, each from the previous area total on its axis up to its own.
            let mut belows: [Vec<f64>; 2] = [vec![0.0; ncat], vec![0.0; ncat]];
            for s in (0..nser).filter(|s| kind(*s) == GraphKind::Area) {
                let a = axis(s);
                let Some(below) = belows.get_mut(a) else { continue };
                let above: Vec<f64> = below.iter().enumerate().map(|(c, b)| b + val(c, s)).collect();
                // Mixed with series at the category centres, areas take their points there too.
                let at = |c: usize, v: &[f64]| Point::new(mid(c, label_edges), vpos(a, v.get(c).copied().unwrap_or(0.0)));
                let mut pts: Vec<Point> = (0..ncat).map(|c| at(c, &above)).collect();
                pts.extend((0..ncat).rev().map(|c| at(c, below)));
                let (fill, stroke, w) = mark(s, true, Paint::solid(Color::WHITE), 0.25);
                let n = b.path(polyline(&pts, true), fill, stroke, w);
                if let Some(items) = series.get_mut(s) {
                    items.push(n);
                }
                *below = above;
            }
            for s in (0..nser).filter(|s| kind(*s) == GraphKind::Line) {
                let pts: Vec<Option<Point>> =
                    (0..ncat).map(|c| cell(c, s).map(|v| Point::new(mid(c, g.edge_to_edge && label_edges), vpos(axis(s), v)))).collect();
                if g.connect_points {
                    for run in runs(&pts) {
                        let (fill, stroke, w) = mark(s, false, default_series_paint(s), 1.0);
                        let n = b.path(polyline(&run, false), fill, stroke, w);
                        shadowed.push((s, n.clone()));
                        if let Some(items) = series.get_mut(s) {
                            items.push(n);
                        }
                    }
                }
                if g.mark_points {
                    for p in pts.iter().flatten() {
                        let paint = mark(s, true, Paint::None, 0.0);
                        let n = b.marker(design(s), Rect::from_center_size(*p, (5.0, 5.0)), paint);
                        if let Some(items) = series.get_mut(s) {
                            items.push(n);
                        }
                    }
                }
            }
        }
    }
    // Legend: a swatch + label per series, right of the plot; the swatch joins its series group.
    let legend_series = if g.kind == GraphKind::Scatter { nser.div_ceil(2) } else { nser };
    let mut labels = vec![];
    let legend_on = g.legend && !g.series.is_empty() && !(g.kind == GraphKind::Pie && g.pie_legend_in_wedges);
    if legend_on && g.legend_across_top {
        // Add Legend Across Top: swatch and label after swatch and label, in rows as wide as the plot, the last row
        // just above it.
        let entries: Vec<(usize, String)> = (0..legend_series)
            .map(|s| (s, if g.kind == GraphKind::Scatter { series_label(s * 2) } else { series_label(s) }))
            .filter(|(_, l)| !blank_label(l))
            .collect();
        let mut rows: Vec<Vec<(usize, String, f64)>> = vec![vec![]];
        let mut used = 0.0;
        for (s, label) in entries {
            let w = 12.0 + label_width(&label) + 12.0;
            // The gap after an entry may run past the plot's edge.
            if used + w - 12.0 > r.width() && rows.last().is_some_and(|row| !row.is_empty()) {
                rows.push(vec![]);
                used = 0.0;
            }
            used += w;
            if let Some(row) = rows.last_mut() {
                row.push((s, label, w));
            }
        }
        let height = |row: &[(usize, String, f64)]| LABEL_SIZE * 1.8 * row.iter().map(|(_, l, _)| l.split('|').count()).max().unwrap_or(1) as f64;
        // Above the plot and anything drawn over it (radar and stacked-pie labels).
        let top = b.out.iter().chain(on_top.iter()).filter_map(|n| n.geometric_bounds()).map(|bb| bb.y0).fold(r.y0, f64::min);
        let mut y = top - 8.0 - rows.iter().map(|row| height(row)).sum::<f64>();
        for row in &rows {
            let mut x = r.x0;
            for (s, label, w) in row {
                let marked = matches!(g.kind, GraphKind::Scatter | GraphKind::Radar) || kind(*s) == GraphKind::Line;
                let paint = mark(*s, true, Paint::None, 0.0);
                let swatch = b.marker(design(*s).filter(|_| marked), Rect::new(x, y, x + 8.0, y + 8.0), paint);
                if let Some(items) = series.get_mut(*s) {
                    items.push(swatch);
                }
                labels.push(b.label(Point::new(x + 12.0, y + 7.5), label, Justify::Left, false));
                x += w;
            }
            y += height(row);
        }
    } else if legend_on {
        let x = right_edge + 14.0;
        // Each row as tall as its label's lines (a `|` in a label starts a new line).
        let mut y = r.y0;
        for (s, items) in series.iter_mut().enumerate().take(legend_series) {
            let label = if g.kind == GraphKind::Scatter { series_label(s * 2) } else { series_label(s) };
            let row_y = y;
            if blank_label(&label) {
                y += LABEL_SIZE * 1.8;
                continue;
            }
            y += LABEL_SIZE * 1.8 * label.split('|').count() as f64;
            let y = row_y;
            // Series drawn with markers show their marker design in the legend.
            let marked = matches!(g.kind, GraphKind::Scatter | GraphKind::Radar) || kind(s) == GraphKind::Line;
            let paint = mark(s, true, Paint::None, 0.0);
            items.push(b.marker(design(s).filter(|_| marked), Rect::new(x, y, x + 8.0, y + 8.0), paint));
            labels.push(b.label(Point::new(x + 12.0, y + 7.5), &label, Justify::Left, false));
        }
    }
    // Areas at the back, lines and points in front of columns; one graph type keeps the series order.
    let depth = |s: usize| match kind(s) {
        GraphKind::Area => 0,
        GraphKind::Line => 2,
        _ => 1,
    };
    // Add Drop Shadow: a translucent black copy of each column, bar, line and wedge, a little down and right, just
    // behind the series of its layer (areas, columns, lines), so a layer in front doesn't hide the shadows of the one
    // behind. Left out past SHADOW_BUDGET marks.
    if !g.drop_shadow || shadowed.len() > SHADOW_BUDGET {
        shadowed.clear();
    }
    let mut series: Vec<(usize, Vec<Arc<Node>>)> = series.into_iter().enumerate().collect();
    series.sort_by_key(|(s, _)| depth(*s));
    let mut layer = None;
    for (s, items) in series {
        if items.is_empty() {
            continue;
        }
        if layer != Some(depth(s)) {
            layer = Some(depth(s));
            let copies: Vec<Arc<Node>> = shadowed.iter().filter(|(t, _)| depth(*t) == depth(s)).map(|(_, n)| shadow_of(b.d, n)).collect();
            if !copies.is_empty() {
                let mut grp = Node::group(b.d.alloc_id(), copies);
                grp.name = Some("Drop Shadow".into());
                grp.opacity = SHADOW_OPACITY;
                b.out.push(Arc::new(grp));
            }
        }
        let name = g.series.get(s).cloned().filter(|n| !n.is_empty()).unwrap_or_else(|| format!("Series {}", s + 1));
        let grp = b.series(&name, s, items);
        b.out.push(grp);
    }
    b.out.append(&mut on_top);
    labels.append(&mut wedge_labels);
    if !labels.is_empty() {
        let grp = b.group("Legend", labels);
        b.out.push(grp);
    }
    b.out
}

fn children_bounds(children: &[Arc<Node>]) -> Option<Rect> {
    children.iter().filter_map(|c| c.geometric_bounds()).reduce(|a, b| a.union(b))
}

/// Regenerate graph `id` from `spec` (adjusting the plot rect for any move/scale since the last
/// generation, which is recorded in the spec's `placed` bounds).
fn regenerate(d: &mut Document, id: NodeId, mut spec: GraphSpec) -> Result<()> {
    let n = d.node(id).ok_or(EngineError::NoNode(id))?;
    if let (Some(placed), Some(now)) = (n.graph.as_ref().and_then(|g| g.placed), n.children().and_then(|c| children_bounds(c)))
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
    n.graph = Some(Box::new(spec));
    Ok(())
}

fn create(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "graph.create";
    let kind =
        str_param(p, "type").map(|t| GraphKind::parse(t).ok_or_else(|| bad(C, format!("unknown graph type `{t}`")))).transpose()?.unwrap_or_default();
    let (x, y) = (f64_req(p, "x", C)?, f64_req(p, "y", C)?);
    let (w, h) = (f64_or(p, "width", 200.0).abs().max(1.0), f64_or(p, "height", 150.0).abs().max(1.0));
    let mut spec = GraphSpec { kind, rect: Rect::new(x, y, x + w, y + h), ..GraphSpec::default() };
    if !apply_data(&mut spec, p) {
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

/// The selected graph (or the graph containing the selection).
fn target(s: &Session, p: &Value, cmd: &str) -> Result<NodeId> {
    let st = s.doc()?;
    let start = id_param(p, "id").into_iter().chain(st.selection.objects.iter().copied());
    for id in start {
        let mut cur = Some(id);
        while let Some(c) = cur {
            if st.doc.node(c).is_some_and(|n| n.graph.is_some()) {
                return Ok(c);
            }
            cur = st.doc.parent_of(c);
        }
    }
    Err(bad(cmd, "select a graph"))
}

fn spec_of(s: &Session, id: NodeId) -> Result<GraphSpec> {
    s.doc()?.doc.node(id).and_then(|n| n.graph.as_deref().cloned()).ok_or(EngineError::NoNode(id))
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
        let Some(spec) = n.graph.as_deref() else { return };
        let count = spec.rows.iter().map(Vec::len).max().unwrap_or(0).clamp(1, vectorcraft_doc::MAX_GRAPH_SERIES);
        for group in n.children().into_iter().flatten() {
            let Some(index) = group.series_index.and_then(|i| usize::try_from(i).ok()).filter(|i| *i < count) else { continue };
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
pub(crate) fn capture_series_paints(doc: &mut Document, ids: &[NodeId], fill: bool) {
    let members = series_members(doc);
    if members.is_empty() {
        return;
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
    for (graph, index, paint, width) in captures {
        let Some(spec) = doc.node_mut(graph).and_then(|n| n.graph.as_deref_mut()) else { continue };
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
}

/// Graph Data › Transpose row/column: categories become series and series categories.
fn transpose(g: &mut GraphSpec) {
    let cells = g.cells();
    let width = cells.iter().map(Vec::len).max().unwrap_or(0);
    let flipped: Vec<Vec<Option<f64>>> = (0..width).map(|s| cells.iter().map(|row| row.get(s).copied().flatten()).collect()).collect();
    std::mem::swap(&mut g.series, &mut g.categories);
    g.set_cells(flipped);
}

/// Graph Data › Switch x/y (scatter graphs): each series' y and x columns trade places, with their labels.
fn switch_xy(g: &mut GraphSpec) {
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

fn set_data(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "graph.setData";
    let id = target(s, p, C)?;
    let mut spec = spec_of(s, id)?;
    let flip = bool_or(p, "transpose", false);
    let swap = bool_or(p, "switchXY", false);
    if swap && spec.kind != GraphKind::Scatter {
        return Err(bad(C, "Switch x/y is for scatter graphs"));
    }
    // Transpose and Switch x/y act after any new data in the same call.
    let changed = apply_data(&mut spec, p);
    if flip {
        // The categories become series: at most as many as a graph has series.
        if spec.cells().len() > vectorcraft_doc::MAX_GRAPH_SERIES {
            return Err(bad(
                C,
                format!("a graph has at most {} series, so it transposes with at most that many categories", vectorcraft_doc::MAX_GRAPH_SERIES),
            ));
        }
        transpose(&mut spec);
        drop_empty_rows(&mut spec);
    }
    if swap {
        switch_xy(&mut spec);
    }
    if !(changed || flip || swap) {
        // Blank cells come back as null, so the rows can be edited and sent back as they are.
        return Ok(json!({ "csv": to_csv(&spec), "series": spec.series, "categories": spec.categories, "rows": spec.cells() }));
    }
    // Series assigned to the right axis that the new data no longer has are dropped, so a series added later starts
    // on the left.
    let count = spec.rows.iter().map(Vec::len).max().unwrap_or(0);
    spec.right_series.retain(|i| *i < count);
    s.edit("Graph Data", |d, _| regenerate(d, id, spec))?;
    Ok(json!({ "id": id.0 }))
}

/// The series a command on graph `id` (of `count` series) applies to: `seriesIndexes`, else, with no `id` param, a
/// selection made only of this graph's series (Group Selection). Empty: the whole graph.
fn picked_series(s: &Session, p: &Value, id: NodeId, count: usize, cmd: &str) -> Result<Vec<usize>> {
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
fn reselect_series(d: &Document, sel: &mut vectorcraft_doc::Selection, id: NodeId, indexes: &[u32]) {
    let groups: Vec<NodeId> = d
        .node(id)
        .and_then(Node::children)
        .into_iter()
        .flatten()
        .filter(|c| c.series_index.is_some_and(|i| indexes.contains(&i)))
        .map(|c| c.id)
        .collect();
    sel.set(groups);
}

/// The document's graph design names, in order.
fn design_names(d: &Document) -> Vec<String> {
    d.graph_designs.iter().map(|x| x.name.clone()).collect()
}

/// Regenerate the graphs whose series use design `name` (it was added or removed).
fn refresh_graphs_using(d: &mut Document, name: &str) -> Result<()> {
    let mut ids = vec![];
    d.walk(|n| {
        if n.graph.as_deref().is_some_and(|g| g.series_markers.iter().flatten().any(|m| m == name)) {
            ids.push(n.id);
        }
    });
    for id in ids {
        if let Some(spec) = d.node(id).and_then(|n| n.graph.as_deref().cloned()) {
            regenerate(d, id, spec)?;
        }
    }
    Ok(())
}

fn design(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "graph.design";
    for key in ["save", "paste", "delete"] {
        if p.get(key).is_some_and(|v| !v.is_string()) {
            return Err(bad(C, format!("`{key}` is a design name")));
        }
    }
    if let Some(name) = str_param(p, "save") {
        let name = GraphDesign::clean_name(name).ok_or_else(|| bad(C, "a design needs a name"))?;
        if s.doc()?.doc.graph_designs.iter().any(|x| x.name == name) {
            return Err(bad(C, format!("there is already a design named `{name}`")));
        }
        if s.doc()?.doc.graph_designs.len() >= GraphDesign::MAX {
            return Err(bad(C, format!("a document keeps at most {} graph designs", GraphDesign::MAX)));
        }
        let art = super::brushsym::selection_art(s, p)?.ok_or_else(|| bad(C, "select the art for the design"))?;
        if art.count() > GraphDesign::MAX_NODES {
            return Err(bad(C, format!("a design holds at most {} objects", GraphDesign::MAX_NODES)));
        }
        // A graph saved as a design is kept as the art it draws.
        let art = GraphDesign::plain_art(&art);
        let saved = name.clone();
        s.edit("Graph Design", |d, _| {
            let art = d.reid(&art);
            d.graph_designs.push(GraphDesign { name: saved.clone(), art: Arc::new(art) });
            refresh_graphs_using(d, &saved)
        })?;
        return Ok(json!({ "name": name }));
    }
    if let Some(name) = str_param(p, "paste") {
        let art = s
            .doc()?
            .doc
            .graph_designs
            .iter()
            .find(|x| x.name == name)
            .map(|x| x.art.clone())
            .ok_or_else(|| bad(C, format!("no design named `{name}`")))?;
        let parent = s.doc()?.insertion_parent();
        let id = s.edit("Paste Design", |d, sel| {
            let n = d.reid(&art);
            let id = n.id;
            d.insert(parent, usize::MAX, n)?;
            sel.set([id]);
            Ok(id)
        })?;
        return Ok(json!({ "id": id.0 }));
    }
    if let Some(name) = str_param(p, "delete") {
        if !s.doc()?.doc.graph_designs.iter().any(|x| x.name == name) {
            return Err(bad(C, format!("no design named `{name}`")));
        }
        let name = name.to_string();
        s.edit("Delete Design", |d, _| {
            d.graph_designs.retain(|x| x.name != name);
            refresh_graphs_using(d, &name)
        })?;
        return ok();
    }
    Ok(json!({ "designs": design_names(&s.doc()?.doc) }))
}

fn set_marker(s: &mut Session, p: &Value) -> Result<Value> {
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

fn set_type(s: &mut Session, p: &Value) -> Result<Value> {
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
    if p.get("seriesIndexes").is_some() && str_param(p, "type").is_none() && str_param(p, "valueAxis").is_none() {
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
        "pieLegend",
        "piePosition",
        "pieSort",
    ];
    if !keys.iter().any(|k| p.get(*k).is_some()) {
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
        });
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
        return Ok(fields);
    }
    // Series picked with Group Selection stay selected through the regeneration (their groups are new nodes).
    let reselect: Vec<u32> = if p.get("seriesIndexes").is_none() { picked.iter().filter_map(|i| u32::try_from(*i).ok()).collect() } else { vec![] };
    if let Some(t) = str_param(p, "type") {
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
    if let Some(v) = str_param(p, "valueAxis") {
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
    spec.separate_scales = bool_or(p, "separateScales", spec.separate_scales);
    for (key, slot) in [
        ("tickLength", &mut spec.tick_length),
        ("rightTickLength", &mut spec.right_tick_length),
        ("categoryTickLength", &mut spec.category_tick_length),
    ] {
        if let Some(v) = str_param(p, key) {
            *slot = TickLength::parse(v).ok_or_else(|| bad(C, format!("unknown {key} `{v}` (none, short or full)")))?;
        }
    }
    for (key, slot) in
        [("tickMarks", &mut spec.tick_marks), ("rightTickMarks", &mut spec.right_tick_marks), ("categoryTickMarks", &mut spec.category_tick_marks)]
    {
        *slot = count_param(p, key, MAX_TICK_MARKS as u64).map_or(*slot, |n| n.max(1));
    }
    spec.ticks_between_labels = bool_or(p, "ticksBetweenLabels", spec.ticks_between_labels);
    if let Some(v) = str_param(p, "piePosition") {
        spec.pie_position =
            vectorcraft_doc::PiePosition::parse(v).ok_or_else(|| bad(C, format!("unknown piePosition `{v}` (even, ratio or stacked)")))?;
    }
    if let Some(v) = str_param(p, "pieSort") {
        spec.pie_sort = vectorcraft_doc::PieSort::parse(v).ok_or_else(|| bad(C, format!("unknown pieSort `{v}` (none, all or first)")))?;
    }
    for (key, slot) in
        [("prefix", &mut spec.prefix), ("suffix", &mut spec.suffix), ("rightPrefix", &mut spec.right_prefix), ("rightSuffix", &mut spec.right_suffix)]
    {
        if let Some(v) = str_param(p, key) {
            // A label, not a paragraph: one line, at most MAX_AFFIX characters.
            *slot = one_line(v);
        }
    }
    spec.right_ticks = count_param(p, "rightTicks", 100).unwrap_or(spec.right_ticks);
    if let Some(v) = p.get("rightAxisMin") {
        spec.right_axis_min = v.as_f64();
    }
    if let Some(v) = p.get("rightAxisMax") {
        spec.right_axis_max = v.as_f64();
    }
    spec.column_width = f64_or(p, "columnWidth", spec.column_width).clamp(1.0, 1000.0);
    spec.cluster_width = f64_or(p, "clusterWidth", spec.cluster_width).clamp(1.0, 100.0);
    spec.legend = bool_or(p, "legend", spec.legend);
    spec.drop_shadow = bool_or(p, "dropShadow", spec.drop_shadow);
    spec.legend_across_top = bool_or(p, "legendAcrossTop", spec.legend_across_top);
    // After `legend`, which the Graph Type dialog sends too.
    if let Some(v) = str_param(p, "pieLegend") {
        (spec.legend, spec.pie_legend_in_wedges) = match v.to_ascii_lowercase().as_str() {
            "none" => (false, false),
            "standard" => (true, false),
            "wedges" => (true, true),
            _ => return Err(bad(C, format!("unknown pieLegend `{v}` (none, standard or wedges)"))),
        };
    }
    spec.mark_points = bool_or(p, "markPoints", spec.mark_points);
    spec.connect_points = bool_or(p, "connectPoints", spec.connect_points);
    spec.edge_to_edge = bool_or(p, "edgeToEdge", spec.edge_to_edge);
    spec.ticks = count_param(p, "ticks", 100).unwrap_or(spec.ticks);
    if let Some(v) = p.get("axisMin") {
        spec.axis_min = v.as_f64();
    }
    if let Some(v) = p.get("axisMax") {
        spec.axis_max = v.as_f64();
    }
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

#[cfg(test)]
mod tests {
    use serde_json::json;
    use vectorcraft_color::Paint;
    use vectorcraft_doc::{GraphKind, GraphSpec, NodeKind};

    use crate::{NodeId, Session};

    fn graph(s: &mut Session, ty: &str) -> NodeId {
        let csv = ",2024,2025\nQ1,3,2\nQ2,5,4\nQ3,-1,6";
        NodeId(
            s.execute("graph.create", &json!({"type": ty, "x": 100, "y": 100, "width": 300, "height": 200, "csv": csv})).unwrap()["id"]
                .as_u64()
                .unwrap(),
        )
    }

    fn group<'a>(n: &'a vectorcraft_doc::Node, name: &str) -> &'a vectorcraft_doc::Node {
        n.children().unwrap().iter().find(|c| c.name.as_deref() == Some(name)).unwrap_or_else(|| panic!("no {name}"))
    }

    /// The generated group stored for series `index`, not a sibling that happens to share its name.
    fn series(n: &vectorcraft_doc::Node, index: u32) -> &vectorcraft_doc::Node {
        n.children().unwrap().iter().find(|c| c.series_index == Some(index)).unwrap_or_else(|| panic!("no series {index}"))
    }

    #[test]
    fn every_graph_type_generates_series_groups() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
        for ty in ["column", "stackedColumn", "bar", "stackedBar", "line", "area", "scatter", "pie", "radar"] {
            let id = graph(&mut s, ty);
            let n = s.doc().unwrap().doc.node(id).unwrap().clone();
            assert!(n.graph.is_some(), "{ty}");
            if ty != "scatter" {
                assert!(!group(&n, "2024").children().unwrap().is_empty(), "{ty}");
                assert!(!group(&n, "2025").children().unwrap().is_empty(), "{ty}");
            }
            assert!(n.geometric_bounds().unwrap().width() > 100.0, "{ty}");
        }
    }

    #[test]
    fn column_heights_follow_data_and_data_edits_regenerate_in_place() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
        let id = graph(&mut s, "column");
        let heights = |s: &Session| -> Vec<f64> {
            let n = s.doc().unwrap().doc.node(id).unwrap().clone();
            // Bars only (the legend swatch is the 8 × 8 square at the end).
            let g = group(&n, "2025");
            let ch = g.children().unwrap();
            ch[..ch.len() - 1].iter().map(|c| c.geometric_bounds().unwrap().height()).collect()
        };
        let h = heights(&s);
        assert!((h[2] / h[0] - 3.0).abs() < 1e-6, "{h:?}");
        // Move the graph, then change the data: it stays where it was moved.
        s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
        s.execute("object.move", &json!({"dx": 50, "dy": 0})).unwrap();
        let x0 = s.doc().unwrap().doc.node(id).unwrap().geometric_bounds().unwrap().x0;
        let v = s.execute("graph.setData", &json!({})).unwrap();
        assert!(v["csv"].as_str().unwrap().contains("Q2,5,4"));
        s.execute("graph.setData", &json!({"csv": ",2024,2025\nQ1,3,2\nQ2,5,8\nQ3,-1,6"})).unwrap();
        let h2 = heights(&s);
        assert!((h2[1] / h2[0] - 4.0).abs() < 1e-6, "{h2:?}");
        assert!((s.doc().unwrap().doc.node(id).unwrap().geometric_bounds().unwrap().x0 - x0).abs() < 1.0);
        // Type change keeps the data; undo restores the columns.
        s.execute("graph.setType", &json!({"type": "pie"})).unwrap();
        assert_eq!(s.doc().unwrap().doc.node(id).unwrap().graph.as_ref().unwrap().kind, vectorcraft_doc::GraphKind::Pie);
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(s.execute("graph.setType", &json!({})).unwrap()["type"], json!("column"));
        // Selecting a bar inside still targets the graph.
        let n = s.doc().unwrap().doc.node(id).unwrap().clone();
        let bar = group(&n, "2024").children().unwrap()[0].id;
        s.execute("select.set", &json!({"ids": [bar.0]})).unwrap();
        assert!(s.execute("graph.setData", &json!({})).is_ok());
        assert!(matches!(n.kind, NodeKind::Group { .. }));
    }

    /// The x of every marker of series `index`, left to right.
    fn marker_xs(s: &Session, id: NodeId, index: u32) -> Vec<f64> {
        let n = s.doc().unwrap().doc.node(id).unwrap().clone();
        let mut xs: Vec<f64> = series(&n, index)
            .children()
            .unwrap()
            .iter()
            .map(|c| c.geometric_bounds().unwrap())
            .filter(|b| (b.width() - 5.0).abs() < 1e-6)
            .map(|b| b.center().x)
            .collect();
        xs.sort_by(f64::total_cmp);
        xs
    }

    #[test]
    fn edge_to_edge_lines_off_puts_line_points_at_category_centres() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
        // 300 pt wide, three categories: centres at 150, 250, 350 (off by default); edges at 100 and 400.
        let id = graph(&mut s, "line");
        assert_eq!(marker_xs(&s, id, 0), vec![150.0, 250.0, 350.0]);
        assert_eq!(s.execute("graph.setType", &json!({})).unwrap()["edgeToEdge"], json!(false));
        s.execute("graph.setType", &json!({"edgeToEdge": true})).unwrap();
        assert_eq!(marker_xs(&s, id, 0), vec![100.0, 250.0, 400.0]);
        // A graph saved before the option existed keeps its edge-to-edge lines.
        let old: GraphSpec = serde_json::from_value(json!({"kind": "line"})).unwrap();
        assert!(old.edge_to_edge);
    }

    #[test]
    fn radar_graphs_follow_mark_and_connect_data_points() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
        let id = graph(&mut s, "radar");
        let parts = |s: &Session| series(&s.doc().unwrap().doc.node(id).unwrap().clone(), 0).children().unwrap().len();
        // The ring, three markers and the legend swatch.
        assert_eq!(parts(&s), 5);
        s.execute("graph.setType", &json!({"markPoints": false})).unwrap();
        assert_eq!(parts(&s), 2);
        s.execute("graph.setType", &json!({"markPoints": true, "connectPoints": false})).unwrap();
        assert_eq!(parts(&s), 4);
    }

    #[test]
    fn an_overridden_value_axis_is_used_exactly() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
        let r = s.execute("graph.create", &json!({"type": "column", "x": 20, "y": 20, "width": 170, "height": 74, "series": ["a"], "categories": ["x", "y"], "rows": [[4839], [10488]]})).unwrap();
        let id = NodeId(r["id"].as_u64().unwrap());
        let tallest = |s: &Session| -> f64 {
            let n = s.doc().unwrap().doc.node(id).unwrap().clone();
            let bars = series(&n, 0).children().unwrap();
            bars.iter().map(|c| c.geometric_bounds().unwrap().height()).fold(0.0, f64::max)
        };
        // Calculated: a nice axis (0 to 12500 here), so the 10488 column falls short of 10488 / 12000 of the plot.
        assert!((tallest(&s) - 74.0 * 10488.0 / 12000.0).abs() > 1.0);
        s.execute("graph.setType", &json!({"axisMin": 0, "axisMax": 12000})).unwrap();
        assert!((tallest(&s) - 74.0 * 10488.0 / 12000.0).abs() < 1e-6, "{}", tallest(&s));
        let o = s.execute("graph.setType", &json!({})).unwrap();
        assert_eq!((o["axisMin"].as_f64(), o["axisMax"].as_f64()), (Some(0.0), Some(12000.0)));
        // One bound only keeps the calculated (nice) axis around it.
        s.execute("graph.setType", &json!({"axisMax": null})).unwrap();
        assert!((tallest(&s) - 74.0 * 10488.0 / 12000.0).abs() > 1.0);
    }

    #[test]
    fn tick_values_end_on_the_maximum_and_never_run_away() {
        // Counted, not accumulated: an offset range keeps its last tick.
        let t = super::tick_values(1e10, 1e10 + 1.0, 0.01);
        assert_eq!(t.len(), 101);
        assert!((t[100] - (1e10 + 1.0)).abs() < 1e-3);
        // A range too wide for f64, or a step too small to move, gives no ticks or a capped list.
        assert!(super::tick_values(-1.7e308, 1.7e308, f64::INFINITY).is_empty());
        assert!(super::tick_values(1e20, 1e20 + 16384.0, 163.84).len() <= 1001);
        assert!(super::tick_values(0.0, 1.0, 0.0).is_empty());
        // An override whose span overflows falls back to the calculated axis instead of hanging.
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
        graph(&mut s, "column");
        s.execute("graph.setType", &json!({"axisMin": -1.7e308, "axisMax": 1.7e308})).unwrap();
    }

    #[test]
    fn group_selection_paint_becomes_the_series_colour_and_survives_regeneration() {
        let mut s = Session::new();
        s.execute("file.new", &json!({})).unwrap();
        for ty in GraphKind::ALL {
            let id = graph(&mut s, ty.id());
            let series_id = group(s.doc().unwrap().doc.node(id).unwrap(), "2024").id;
            s.execute("paint.setFill", &json!({"ids": [series_id.0], "color": "#ff0000"})).unwrap();
            let spec = s.doc().unwrap().doc.node(id).unwrap().graph.as_deref().unwrap().clone();
            let restored: GraphSpec = serde_json::from_value(serde_json::to_value(&spec).unwrap()).unwrap();
            assert_eq!(spec, restored);
            let red = Paint::solid(vectorcraft_color::Color::rgb(1.0, 0.0, 0.0));
            assert_eq!(spec.series_paints.first().and_then(|p| p.fill.clone()), Some(red.clone()), "{ty:?}");
            let selected_group = group(s.doc().unwrap().doc.node(id).unwrap(), "2024");
            assert!(selected_group.children().unwrap().iter().all(|n| n.appearance.fill_paint() == red), "{ty:?}");
            s.execute("graph.setData", &json!({"rows": [[2, 3], [4, 5]]})).unwrap();
            s.execute("graph.setType", &json!({"type": "line"})).unwrap();
            let spec = s.doc().unwrap().doc.node(id).unwrap().graph.as_deref().unwrap();
            assert_eq!(spec.series_paints.first().and_then(|p| p.fill.clone()), Some(red.clone()), "{ty:?}");
            let series = group(s.doc().unwrap().doc.node(id).unwrap(), "2024").children().unwrap();
            assert!(series.iter().any(|n| n.appearance.fill_paint() == red), "{ty:?}");
        }
    }

    #[test]
    fn painting_a_series_is_one_undoable_edit() {
        let mut s = Session::new();
        s.execute("file.new", &json!({})).unwrap();
        let id = graph(&mut s, "column");
        let series_id = group(s.doc().unwrap().doc.node(id).unwrap(), "2024").id;
        s.execute("paint.setFill", &json!({"ids": [series_id.0], "color": "#e8573f"})).unwrap();
        assert_eq!(
            s.doc().unwrap().doc.node(id).unwrap().graph.as_ref().unwrap().series_paints[0].fill.as_ref().and_then(Paint::color).unwrap().to_hex(),
            "#e8573f"
        );
        s.execute("edit.undo", &json!({})).unwrap();
        assert!(s.doc().unwrap().doc.node(id).unwrap().graph.as_ref().unwrap().series_paints.is_empty());
        assert_eq!(
            group(s.doc().unwrap().doc.node(id).unwrap(), "2024").children().unwrap()[0].appearance.fill_paint().color(),
            Some(super::default_series_color(0))
        );
        s.execute("edit.redo", &json!({})).unwrap();
        assert_eq!(
            s.doc().unwrap().doc.node(id).unwrap().graph.as_ref().unwrap().series_paints[0].fill.as_ref().and_then(Paint::color).unwrap().to_hex(),
            "#e8573f"
        );
    }

    #[test]
    fn painting_one_bar_sets_the_colour_used_by_the_whole_series_on_rebuild() {
        let mut s = Session::new();
        s.execute("file.new", &json!({})).unwrap();
        let id = graph(&mut s, "column");
        let bar = group(s.doc().unwrap().doc.node(id).unwrap(), "2024").children().unwrap()[0].id;
        s.execute("paint.setFill", &json!({"ids": [bar.0], "color": "#12ab34"})).unwrap();
        s.execute("graph.setData", &json!({"rows": [[1, 2], [3, 4]]})).unwrap();
        let node = s.doc().unwrap().doc.node(id).unwrap();
        let green = vectorcraft_color::Color::rgb(18.0 / 255.0, 171.0 / 255.0, 52.0 / 255.0);
        assert_eq!(node.graph.as_ref().unwrap().series_paints[0].fill.as_ref().and_then(Paint::color), Some(green));
        assert!(group(node, "2024").children().unwrap().iter().all(|n| n.appearance.fill_paint().color() == Some(green)));
    }

    /// A series index read from a file past the graph's series is not a series: painting its art
    /// paints the art alone and stores nothing (no allocation sized by the file).
    #[test]
    fn a_series_index_past_the_graphs_series_is_ignored() {
        let mut s = Session::new();
        s.execute("file.new", &json!({})).unwrap();
        let id = graph(&mut s, "column");
        let series_id = group(s.doc().unwrap().doc.node(id).unwrap(), "2024").id;
        s.edit("Junk", |d, _| {
            d.node_mut(series_id).unwrap().series_index = Some(u32::MAX);
            Ok(())
        })
        .unwrap();
        let bar = group(s.doc().unwrap().doc.node(id).unwrap(), "2024").children().unwrap()[0].id;
        s.execute("paint.setFill", &json!({"ids": [bar.0, series_id.0], "color": "#12ab34"})).unwrap();
        let node = s.doc().unwrap().doc.node(id).unwrap();
        assert!(node.graph.as_ref().unwrap().series_paints.is_empty());
        assert_eq!(group(node, "2024").children().unwrap()[0].appearance.fill_paint().color().map(|c| c.to_hex()), Some("#12ab34".into()));
    }

    #[test]
    fn painting_a_line_series_stroke_sets_its_persistent_colour() {
        let mut s = Session::new();
        s.execute("file.new", &json!({})).unwrap();
        let id = graph(&mut s, "line");
        let series_id = group(s.doc().unwrap().doc.node(id).unwrap(), "2024").id;
        s.execute("paint.setStroke", &json!({"ids": [series_id.0], "color": "#3751c8"})).unwrap();
        let blue = vectorcraft_color::Color::rgb(55.0 / 255.0, 81.0 / 255.0, 200.0 / 255.0);
        assert_eq!(
            s.doc().unwrap().doc.node(id).unwrap().graph.as_ref().unwrap().series_paints[0].stroke.as_ref().and_then(Paint::color),
            Some(blue)
        );
        assert!(s.doc().unwrap().doc.node(id).unwrap().graph.as_ref().unwrap().series_paints[0].fill.is_none());
        s.execute("graph.setData", &json!({"rows": [[1, 2], [3, 4]]})).unwrap();
        assert!(
            group(s.doc().unwrap().doc.node(id).unwrap(), "2024")
                .children()
                .unwrap()
                .iter()
                .any(|n| n.appearance.stroke_paint().color() == Some(blue))
        );
    }

    #[test]
    fn series_colours_survive_native_save_open_and_export() {
        let mut s = Session::new();
        s.execute("file.new", &json!({})).unwrap();
        let id = graph(&mut s, "column");
        let first = group(s.doc().unwrap().doc.node(id).unwrap(), "2024").id;
        let second = group(s.doc().unwrap().doc.node(id).unwrap(), "2025").id;
        s.execute("paint.setFill", &json!({"ids": [first.0], "color": "#f00"})).unwrap();
        s.execute("paint.setFill", &json!({"ids": [second.0], "color": "#008080"})).unwrap();
        let dir = vectorcraft_testkit::temp_dir("graph-palette");
        let path = dir.join("palette.vectorcraft");
        s.execute("document.save", &json!({"path": path})).unwrap();
        s.execute("document.open", &json!({"path": path})).unwrap();
        let paints = &s.doc().unwrap().doc.node(id).unwrap().graph.as_ref().unwrap().series_paints;
        assert_eq!(paints.iter().map(|p| p.fill.as_ref().and_then(Paint::color).unwrap().to_hex()).collect::<Vec<_>>(), vec!["#ff0000", "#008080"]);
        s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
        s.execute("graph.setData", &json!({"id": id.0, "rows": [[3, 4, 5]]})).unwrap();
        let n = s.doc().unwrap().doc.node(id).unwrap();
        let swatch = group(n, "2025").children().unwrap().last().unwrap();
        assert_eq!(swatch.appearance.fill_paint().color().unwrap().to_hex(), "#008080");
        let third = group(n, "Series 3").children().unwrap().first().unwrap();
        assert_eq!(third.appearance.fill_paint().color().unwrap().to_hex(), "#cccccc");
        let svg = s.execute("document.serialize", &json!({"format": "svg"})).unwrap();
        let svg = svg["text"].as_str().unwrap();
        assert!(svg.contains("#ff0000") && svg.contains("#008080"));
        let png = dir.join("palette.png");
        s.execute("document.export", &json!({"format": "png", "path": png})).unwrap();
        let pixels = image::open(&png).unwrap().to_rgba8();
        assert!(pixels.pixels().any(|p| p.0 == [255, 0, 0, 255]));
        assert!(pixels.pixels().any(|p| p.0 == [0, 128, 128, 255]));
        let old: GraphSpec = serde_json::from_value(json!({"rows": [[1.0]]})).unwrap();
        assert!(old.series_paints.is_empty(), "older files keep the greyscale default");
    }

    #[test]
    fn a_column_stroke_stays_a_stroke_and_a_marker_fill_does_not_recolour_its_line() {
        let mut s = Session::new();
        s.execute("file.new", &json!({})).unwrap();
        let id = graph(&mut s, "column");
        let series_id = group(s.doc().unwrap().doc.node(id).unwrap(), "2024").id;
        s.execute("paint.setStroke", &json!({"ids": [series_id.0], "color": "#0000ff"})).unwrap();
        s.execute("graph.setData", &json!({"rows": [[1, 2], [3, 4]]})).unwrap();
        let spec = s.doc().unwrap().doc.node(id).unwrap().graph.as_deref().unwrap().clone();
        let blue = Paint::solid(vectorcraft_color::Color::rgb(0.0, 0.0, 1.0));
        assert_eq!(spec.series_paints[0].stroke, Some(blue.clone()));
        assert!(spec.series_paints[0].fill.is_none());
        let bar = &group(s.doc().unwrap().doc.node(id).unwrap(), "2024").children().unwrap()[0];
        assert_eq!(bar.appearance.fill_paint().color(), Some(super::default_series_color(0)));
        assert_eq!(bar.appearance.stroke_paint(), blue);

        let id = graph(&mut s, "line");
        let marker = group(s.doc().unwrap().doc.node(id).unwrap(), "2024").children().unwrap()[1].id;
        s.execute("paint.setFill", &json!({"ids": [marker.0], "color": "#ff0000"})).unwrap();
        s.execute("graph.setData", &json!({"rows": [[1, 2], [3, 4]]})).unwrap();
        let spec = s.doc().unwrap().doc.node(id).unwrap().graph.as_deref().unwrap().clone();
        assert!(spec.series_paints[0].stroke.is_none());
        let series = group(s.doc().unwrap().doc.node(id).unwrap(), "2024").children().unwrap();
        assert_eq!(series[0].appearance.stroke_paint().color(), Some(super::default_series_color(0)));
        assert_eq!(series[1].appearance.fill_paint().color(), Some(vectorcraft_color::Color::rgb(1.0, 0.0, 0.0)));
    }

    #[test]
    fn a_gradient_fill_survives_regeneration() {
        let mut s = Session::new();
        s.execute("file.new", &json!({})).unwrap();
        let id = graph(&mut s, "column");
        let series_id = group(s.doc().unwrap().doc.node(id).unwrap(), "2024").id;
        s.execute(
            "paint.setFill",
            &json!({"ids": [series_id.0], "gradient": {"kind": "linear", "stops": [{"offset": 0, "color": "#ff0000"}, {"offset": 1, "color": "#0000ff"}]}}),
        )
        .unwrap();
        s.execute("graph.setData", &json!({"rows": [[2, 3], [4, 5]]})).unwrap();
        let spec = s.doc().unwrap().doc.node(id).unwrap().graph.as_deref().unwrap().clone();
        assert!(matches!(spec.series_paints[0].fill, Some(Paint::Gradient(_))));
        let bar = &group(s.doc().unwrap().doc.node(id).unwrap(), "2024").children().unwrap()[0];
        assert!(matches!(bar.appearance.fill_paint(), Paint::Gradient(_)));
    }

    #[test]
    fn a_series_named_legend_keeps_its_own_index_and_an_empty_series_is_omitted() {
        let mut s = Session::new();
        s.execute("file.new", &json!({})).unwrap();
        let csv = ",Legend,Axes\nQ1,3,2\nQ2,5,4";
        let id = NodeId(s.execute("graph.create", &json!({"type": "column", "x": 0, "y": 0, "csv": csv})).unwrap()["id"].as_u64().unwrap());
        let legend_id = {
            let n = s.doc().unwrap().doc.node(id).unwrap();
            let legend = series(n, 0);
            assert_eq!(legend.name.as_deref(), Some("Legend"));
            assert!(n.children().unwrap().iter().any(|c| c.name.as_deref() == Some("Legend") && c.series_index.is_none()));
            assert!(n.children().unwrap().iter().any(|c| c.name.as_deref() == Some("Axes") && c.series_index.is_none()));
            legend.id
        };
        s.execute("paint.setFill", &json!({"ids": [legend_id.0], "color": "#ff0000"})).unwrap();
        s.execute("graph.setData", &json!({"csv": csv})).unwrap();
        let n = s.doc().unwrap().doc.node(id).unwrap();
        assert_eq!(n.graph.as_ref().unwrap().series_paints[0].fill.as_ref().and_then(Paint::color).unwrap().to_hex(), "#ff0000");
        assert!(n.graph.as_ref().unwrap().series_paints.get(1).is_none_or(|p| p.fill.is_none()));
        assert_eq!(series(n, 0).children().unwrap()[0].appearance.fill_paint().color().unwrap().to_hex(), "#ff0000");
        assert_eq!(series(n, 1).name.as_deref(), Some("Axes"));
        assert_ne!(series(n, 1).children().unwrap()[0].appearance.fill_paint().color().unwrap().to_hex(), "#ff0000");

        let id = graph(&mut s, "pie");
        s.execute("graph.setType", &json!({"id": id.0, "legend": false})).unwrap();
        s.execute("graph.setData", &json!({"id": id.0, "csv": ",Keep,Drop\nQ1,4,0\nQ2,2,0"})).unwrap();
        let n = s.doc().unwrap().doc.node(id).unwrap();
        assert!(n.children().unwrap().iter().any(|c| c.name.as_deref() == Some("Keep")));
        assert!(n.children().unwrap().iter().all(|c| c.name.as_deref() != Some("Drop")));
        let keep = group(n, "Keep").id;
        s.execute("paint.setFill", &json!({"ids": [keep.0], "color": "#00ff00"})).unwrap();
        s.execute("graph.setData", &json!({"id": id.0, "rows": [[8, 0], [1, 0]]})).unwrap();
        let n = s.doc().unwrap().doc.node(id).unwrap();
        assert_eq!(group(n, "Keep").series_index, Some(0));
        assert_eq!(n.graph.as_ref().unwrap().series_paints[0].fill.as_ref().and_then(Paint::color).unwrap().to_hex(), "#00ff00");
        assert!(n.children().unwrap().iter().all(|c| c.name.as_deref() != Some("Drop")));
    }

    #[test]
    fn csv_parsing() {
        let (s, c, r) = super::parse_csv("\t2024\t2025\nQ1\t1\t2\nQ2\t3\t4\n");
        assert_eq!(s, vec!["2024", "2025"]);
        assert_eq!(c, vec!["Q1", "Q2"]);
        assert_eq!(r, vec![vec![Some(1.0), Some(2.0)], vec![Some(3.0), Some(4.0)]]);
        let (s, c, r) = super::parse_csv("1,2\n3,4");
        assert!(s.is_empty() && c.is_empty());
        assert_eq!(r.len(), 2);
        assert_eq!(super::nice_axis(0.0, 7.0, 0), (0.0, 8.0, 2.0));
    }

    #[test]
    fn empty_cells_are_blank_and_quoted_numbers_are_labels() {
        // Quoted years label the rows; empty and non-numeric cells are blank values, not zeros.
        let (s, c, r) = super::parse_csv(",a,b\n\"2023\",1,\n\"2024\",,x\n\"2025\",3,4");
        assert_eq!(s, vec!["a", "b"]);
        assert_eq!(c, vec!["2023", "2024", "2025"]);
        assert_eq!(r, vec![vec![Some(1.0), None], vec![None, None], vec![Some(3.0), Some(4.0)]]);
        // Back to CSV: blanks stay empty and numeric labels keep their quotes, so it reads back the same.
        let mut g = GraphSpec { series: s, categories: c, ..GraphSpec::default() };
        g.set_cells(r);
        assert_eq!(g.blanks, vec![[0, 1], [1, 0], [1, 1]]);
        let csv = super::to_csv(&g);
        assert!(csv.contains("\"2024\",,"), "{csv}");
        let (s2, c2, r2) = super::parse_csv(&csv);
        assert_eq!((s2, c2, r2), (g.series.clone(), g.categories.clone(), g.cells()));
    }

    #[test]
    fn a_row_with_an_empty_label_keeps_its_values_in_place() {
        // The label column is the first column of every row once any row has a label.
        let (_, c, r) = super::parse_csv(",a,b\nQ1,1,2\n,3,4");
        assert_eq!(c, vec!["Q1", ""]);
        assert_eq!(r, vec![vec![Some(1.0), Some(2.0)], vec![Some(3.0), Some(4.0)]]);
        // And a data window round trip keeps it (the empty label is written as "").
        let mut g = GraphSpec { series: vec!["a".into(), "b".into()], categories: c, ..GraphSpec::default() };
        g.set_cells(r);
        assert_eq!(super::parse_csv(&super::to_csv(&g)).2, g.cells());
        // Without series or labels, a first row starting with a blank is still data after a round trip.
        let mut g = GraphSpec::default();
        g.set_cells(vec![vec![None, Some(2.0)], vec![Some(3.0), Some(4.0)]]);
        let (s, c, r) = super::parse_csv(&super::to_csv(&g));
        assert!(s.is_empty() && c.is_empty());
        assert_eq!(r, g.cells());
        // A label with spaces around a number stays a label, spaces and all.
        let mut g = GraphSpec { categories: vec!["2024 ".into()], ..GraphSpec::default() };
        g.set_cells(vec![vec![Some(1.0)]]);
        assert_eq!(super::parse_csv(&super::to_csv(&g)).1, vec!["2024 "]);
    }

    #[test]
    fn blanks_read_back_as_null_and_year_header_rows_still_read_as_headers() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
        let r =
            s.execute("graph.create", &json!({"type": "line", "x": 0, "y": 0, "width": 300, "height": 200, "rows": [[1, null], [2, 3]]})).unwrap();
        let id = NodeId(r["id"].as_u64().unwrap());
        assert_eq!(s.execute("graph.setData", &json!({"id": id.0})).unwrap()["rows"], json!([[1.0, null], [2.0, 3.0]]));
        // A label followed by empty cells is a header with no series names, as before blanks existed.
        let (s, c, r) = super::parse_csv("Year,,\n2023,1,2");
        assert!(s.is_empty());
        assert_eq!(c, Vec::<String>::new());
        assert_eq!(r, vec![vec![Some(2023.0), Some(1.0), Some(2.0)]]);
    }

    #[test]
    fn a_huge_ragged_grid_is_capped() {
        let g = GraphSpec { rows: vec![vec![0.0; vectorcraft_doc::MAX_GRAPH_SERIES + 10]; 3], ..GraphSpec::default() };
        assert!(g.cells().iter().all(|r| r.len() == vectorcraft_doc::MAX_GRAPH_SERIES));
    }

    #[test]
    fn a_label_only_row_stays_with_blank_values() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
        let r =
            s.execute("graph.create", &json!({"type": "column", "x": 0, "y": 0, "width": 300, "height": 200, "csv": ",a\nQ1,1\nQ2\nQ3,3"})).unwrap();
        let id = NodeId(r["id"].as_u64().unwrap());
        let spec = s.doc().unwrap().doc.node(id).unwrap().graph.clone().unwrap();
        assert_eq!(spec.categories, vec!["Q1", "Q2", "Q3"]);
        assert_eq!(spec.cells(), vec![vec![Some(1.0)], vec![None], vec![Some(3.0)]]);
    }

    #[test]
    fn blank_cells_leave_out_their_columns_and_break_lines() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
        let rows = json!([[3, 1], [null, 2], [5, null], [4, 4]]);
        let r = s
            .execute("graph.create", &json!({"type": "column", "x": 100, "y": 100, "width": 400, "height": 200, "series": ["a", "b"], "rows": rows}))
            .unwrap();
        let id = NodeId(r["id"].as_u64().unwrap());
        let count = |s: &Session, index: u32| series(&s.doc().unwrap().doc.node(id).unwrap().clone(), index).children().unwrap().len();
        // Three columns each (one blank), plus the legend swatch.
        assert_eq!((count(&s, 0), count(&s, 1)), (4, 4));
        s.execute("graph.setType", &json!({"type": "stackedColumn"})).unwrap();
        assert_eq!((count(&s, 0), count(&s, 1)), (4, 4));
        // Line: series a is 3, blank, 5, 4. The lone first point has nothing to connect to, so one line (5 to 4), three
        // markers and the legend swatch.
        s.execute("graph.setType", &json!({"type": "line"})).unwrap();
        assert_eq!(count(&s, 0), 1 + 3 + 1);
        // Series b is 1, 2, blank, 4: one line (1 to 2), three markers and the swatch.
        assert_eq!(count(&s, 1), 1 + 3 + 1);
        // A data window round trip keeps the blanks.
        let csv = s.execute("graph.setData", &json!({})).unwrap()["csv"].as_str().unwrap().to_string();
        assert_eq!(csv.lines().nth(2), Some(",2"), "{csv}");
        let spec = s.doc().unwrap().doc.node(id).unwrap().graph.clone().unwrap();
        assert_eq!(spec.cells()[1], vec![None, Some(2.0)]);
        // In the file a blank is a 0 placeholder plus its place in `blanks`, so a version without blanks reads
        // numbers (as zeros); a graph without blanks writes no `blanks` at all.
        let v = serde_json::to_value(&spec).unwrap();
        assert_eq!(v["rows"][1], json!([0.0, 2.0]));
        assert_eq!(v["blanks"], json!([[1, 0], [2, 1]]));
        assert!(serde_json::to_value(GraphSpec::default()).unwrap().get("blanks").is_none());
    }

    fn spec(s: &Session, id: NodeId) -> GraphSpec {
        s.doc().unwrap().doc.node(id).unwrap().graph.as_deref().unwrap().clone()
    }

    /// Bounds of a series' marks, without its legend swatch (the last child).
    fn marks(s: &Session, id: NodeId, index: u32) -> Vec<vectorcraft_geom::Rect> {
        let n = s.doc().unwrap().doc.node(id).unwrap().clone();
        let ch = series(&n, index).children().unwrap().to_vec();
        ch[..ch.len() - 1].iter().map(|c| c.geometric_bounds().unwrap()).collect()
    }

    #[test]
    fn a_series_given_its_own_type_is_drawn_as_that_type_in_front_of_the_columns() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
        let id = graph(&mut s, "column");
        let one_column = marks(&s, id, 0)[0].width();
        s.execute("graph.setType", &json!({"seriesIndexes": [1], "type": "line"})).unwrap();
        let g = spec(&s, id);
        assert_eq!((g.kind, g.series_kind(0), g.series_kind(1)), (GraphKind::Column, GraphKind::Column, GraphKind::Line));
        // The columns left take the whole cluster; the line runs through the category centres, one marker a value.
        let cols = marks(&s, id, 0);
        assert_eq!(cols.len(), 3);
        assert!((cols[0].width() - 2.0 * one_column).abs() < 1e-6, "{} vs {one_column}", cols[0].width());
        let line = marks(&s, id, 1);
        assert_eq!(line.len(), 4, "a line and three markers");
        assert!((line[1].center().x - cols[0].center().x).abs() < 1e-6);
        // The line series is drawn after (in front of) the column series.
        let n = s.doc().unwrap().doc.node(id).unwrap().clone();
        let order: Vec<u32> = n.children().unwrap().iter().filter_map(|c| c.series_index).collect();
        assert_eq!(order, [0, 1]);
        s.execute("graph.setType", &json!({"seriesIndexes": [0], "type": "line"})).unwrap();
        s.execute("graph.setType", &json!({"seriesIndexes": [1], "type": "column"})).unwrap();
        let n = s.doc().unwrap().doc.node(id).unwrap().clone();
        let order: Vec<u32> = n.children().unwrap().iter().filter_map(|c| c.series_index).collect();
        assert_eq!(order, [1, 0]);
        // Back to the graph's own type: nothing is stored for that series.
        assert_eq!(spec(&s, id).series_kinds, [Some(GraphKind::Line)]);
        // One undo per change.
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(spec(&s, id).series_kinds, [Some(GraphKind::Line), Some(GraphKind::Line)]);
    }

    #[test]
    fn group_selecting_a_series_retypes_only_that_series() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
        let id = graph(&mut s, "column");
        let n = s.doc().unwrap().doc.node(id).unwrap().clone();
        let grp = series(&n, 1).id;
        s.execute("select.set", &json!({"ids": [grp.0]})).unwrap();
        // The dialog opens on the series' type, and its OK (every option sent back) keeps it.
        assert_eq!(s.execute("graph.setType", &json!({})).unwrap()["type"], "column");
        s.execute("graph.setType", &json!({"type": "area"})).unwrap();
        let g = spec(&s, id);
        assert_eq!((g.kind, g.series_kind(1)), (GraphKind::Column, GraphKind::Area));
        let mut all = s.execute("graph.setType", &json!({})).unwrap();
        assert_eq!(all["type"], "area");
        all["ticks"] = json!(4);
        s.execute("graph.setType", &all).unwrap();
        assert_eq!(spec(&s, id).series_kind(1), GraphKind::Area);
        // The series stays selected through each change.
        let grp = series(s.doc().unwrap().doc.node(id).unwrap(), 1).id;
        assert_eq!(s.doc().unwrap().selection.objects.to_vec(), [grp]);
        // A series and something outside it selected, or an explicit `id`: the whole graph.
        s.execute("select.set", &json!({"ids": [grp.0, id.0]})).unwrap();
        assert_eq!(s.execute("graph.setType", &json!({})).unwrap()["type"], "column");
        s.execute("select.set", &json!({"ids": [grp.0]})).unwrap();
        assert_eq!(s.execute("graph.setType", &json!({"id": id.0})).unwrap()["type"], "column");
        // The whole graph selected: the graph's type changes, the series keeps its own.
        s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
        s.execute("graph.setType", &json!({"type": "line"})).unwrap();
        let g = spec(&s, id);
        assert_eq!((g.kind, g.series_kind(0), g.series_kind(1)), (GraphKind::Line, GraphKind::Line, GraphKind::Area));
    }

    #[test]
    fn types_that_do_not_share_a_value_axis_are_refused_or_ignored() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
        let id = graph(&mut s, "column");
        for ty in ["pie", "radar", "scatter", "bar"] {
            assert!(s.execute("graph.setType", &json!({"seriesIndexes": [1], "type": ty})).is_err(), "{ty}");
        }
        for bad in [json!([2]), json!([]), json!([-1]), json!(["x"]), json!([1e30]), json!({}), json!(1)] {
            assert!(s.execute("graph.setType", &json!({"seriesIndexes": bad, "type": "line"})).is_err(), "{bad}");
        }
        assert!(s.execute("graph.setType", &json!({"seriesIndexes": [1]})).is_err(), "no type");
        assert!(spec(&s, id).series_kinds.is_empty());
        // A series type the graph's new type can't take is drawn as the graph's type, and kept for when it can.
        s.execute("graph.setType", &json!({"seriesIndexes": [1], "type": "line"})).unwrap();
        s.execute("graph.setType", &json!({"type": "pie"})).unwrap();
        let g = spec(&s, id);
        assert_eq!(g.series_kind(1), GraphKind::Pie);
        s.execute("graph.setType", &json!({"type": "column"})).unwrap();
        assert_eq!(spec(&s, id).series_kind(1), GraphKind::Line);
    }

    #[test]
    fn stacked_columns_take_one_slot_and_the_value_axis_covers_their_total() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
        let csv = ",a,b,c\nQ1,10,20,30\nQ2,10,20,30";
        let id = NodeId(
            s.execute("graph.create", &json!({"type": "stackedColumn", "x": 0, "y": 0, "width": 300, "height": 200, "csv": csv})).unwrap()["id"]
                .as_u64()
                .unwrap(),
        );
        s.execute("graph.setType", &json!({"seriesIndexes": [2], "type": "line"})).unwrap();
        let (a, b) = (marks(&s, id, 0), marks(&s, id, 1));
        assert!((a[0].x0 - b[0].x0).abs() < 1e-6 && (a[0].y0 - b[0].y1).abs() < 1e-6, "b sits on a");
        // a + b stack to 30, the line's highest value is 30: the axis ends at 30 and the stack reaches the top.
        assert!((b[0].y0 - 0.0).abs() < 1e-6, "{:?}", b[0]);
        // With a plain column too, the stack is the cluster's last slot, beside it.
        s.execute("graph.setType", &json!({"seriesIndexes": [0], "type": "column"})).unwrap();
        let (a, b) = (marks(&s, id, 0), marks(&s, id, 1));
        assert!((a[0].width() - b[0].width()).abs() < 1e-6 && b[0].x0 > a[0].x1, "{:?} {:?}", a[0], b[0]);
    }

    #[test]
    fn areas_mixed_with_columns_put_their_points_at_the_category_centres() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
        let id = graph(&mut s, "column");
        s.execute("graph.setType", &json!({"seriesIndexes": [0], "type": "area"})).unwrap();
        let (area, cols) = (marks(&s, id, 0)[0], marks(&s, id, 1));
        assert!((area.x0 - cols[0].center().x).abs() < 1e-6 && (area.x1 - cols[2].center().x).abs() < 1e-6, "{area:?}");
        // The area is drawn behind the columns.
        let n = s.doc().unwrap().doc.node(id).unwrap().clone();
        let order: Vec<u32> = n.children().unwrap().iter().filter_map(|c| c.series_index).collect();
        assert_eq!(order, [0, 1]);
        // An area graph alone still spans the plot.
        s.execute("graph.setType", &json!({"seriesIndexes": [1], "type": "area"})).unwrap();
        assert!((marks(&s, id, 0)[0].x0 - 100.0).abs() < 1e-6);
    }

    #[test]
    fn bar_graphs_take_stacked_bar_series() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
        let csv = ",a,b,c\nQ1,10,20,-5\nQ2,10,20,-5";
        let id = NodeId(
            s.execute("graph.create", &json!({"type": "bar", "x": 0, "y": 0, "width": 300, "height": 200, "csv": csv})).unwrap()["id"]
                .as_u64()
                .unwrap(),
        );
        assert!(s.execute("graph.setType", &json!({"seriesIndexes": [1], "type": "column"})).is_err());
        s.execute("graph.setType", &json!({"seriesIndexes": [1, 2], "type": "stackedBar"})).unwrap();
        let (a, b, c) = (marks(&s, id, 0), marks(&s, id, 1), marks(&s, id, 2));
        // b and c stack along x in the cluster's second slot, below a's bar; c, negative, runs left of the zero line.
        assert!((a[0].height() - b[0].height()).abs() < 1e-6 && b[0].y0 > a[0].y1, "{:?} {:?}", a[0], b[0]);
        assert!((b[0].y0 - c[0].y0).abs() < 1e-6 && (c[0].x1 - b[0].x0).abs() < 1e-6, "{:?} {:?}", b[0], c[0]);
    }

    #[test]
    fn series_types_survive_save_and_open_and_older_files_have_none() {
        let mut s = Session::new();
        s.execute("file.new", &json!({})).unwrap();
        let id = graph(&mut s, "column");
        s.execute("graph.setType", &json!({"seriesIndexes": [1], "type": "line"})).unwrap();
        let path = vectorcraft_testkit::temp_dir("graph-kinds").join("kinds.vectorcraft");
        s.execute("document.save", &json!({"path": path})).unwrap();
        s.execute("document.open", &json!({"path": path})).unwrap();
        assert_eq!(spec(&s, id).series_kinds, [None, Some(GraphKind::Line)]);
        let g: GraphSpec = serde_json::from_value(json!({"kind": "line", "rows": [[1.0, 2.0]]})).unwrap();
        assert!(g.series_kinds.is_empty());
        assert!(serde_json::to_value(&g).unwrap().get("seriesKinds").is_none());
        // A stored type the graph can't take, or past its series, is drawn as the graph's type.
        let g: GraphSpec = serde_json::from_value(json!({"kind": "pie", "seriesKinds": ["line", null, "column"], "rows": [[1.0]]})).unwrap();
        assert_eq!((g.series_kind(0), g.series_kind(5)), (GraphKind::Pie, GraphKind::Pie));
    }

    /// The value axis labels: text and bounds of the texts in the Axes group that read as numbers.
    fn value_labels(s: &Session, id: NodeId) -> Vec<(String, vectorcraft_geom::Rect)> {
        let n = s.doc().unwrap().doc.node(id).unwrap().clone();
        group(&n, "Axes")
            .children()
            .unwrap()
            .iter()
            .filter_map(|c| match &c.kind {
                NodeKind::Text(t) => Some((t.plain_text(), c.geometric_bounds().unwrap())),
                _ => None,
            })
            .filter(|(t, _)| t.parse::<f64>().is_ok())
            .collect()
    }

    fn two_scale_graph(s: &mut Session) -> NodeId {
        s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
        let csv = ",a,b\nQ1,10,1000\nQ2,5,500";
        NodeId(
            s.execute("graph.create", &json!({"type": "column", "x": 100, "y": 100, "width": 300, "height": 200, "csv": csv})).unwrap()["id"]
                .as_u64()
                .unwrap(),
        )
    }

    #[test]
    fn the_value_axis_goes_on_the_left_the_right_or_both_sides() {
        let mut s = Session::new();
        let id = two_scale_graph(&mut s);
        let left: Vec<String> = value_labels(&s, id).into_iter().inspect(|(_, b)| assert!(b.x1 < 100.0, "{b:?}")).map(|(t, _)| t).collect();
        assert_eq!(left, ["0", "200", "400", "600", "800", "1000"]);
        s.execute("graph.setType", &json!({"valueAxis": "right"})).unwrap();
        let right = value_labels(&s, id);
        assert!(right.iter().all(|(_, b)| b.x0 > 400.0), "{right:?}");
        assert_eq!(right.iter().map(|(t, _)| t.as_str()).collect::<Vec<_>>(), left);
        // Both sides, one scale: the same labels on each side, and the legend moves past the right ones.
        s.execute("graph.setType", &json!({"valueAxis": "both"})).unwrap();
        let both = value_labels(&s, id);
        assert_eq!(both.len(), 12);
        let edge = both.iter().map(|(_, b)| b.x1).fold(0.0, f64::max);
        let n = s.doc().unwrap().doc.node(id).unwrap().clone();
        let swatch = series(&n, 0).children().unwrap().last().unwrap().geometric_bounds().unwrap();
        assert!(swatch.x0 > edge, "{swatch:?} vs {edge}");
        assert_eq!(s.execute("graph.setType", &json!({})).unwrap()["valueAxis"], "both");
        assert!(s.execute("graph.setType", &json!({"valueAxis": "top"})).is_err());
    }

    #[test]
    fn separate_scales_give_the_right_axis_series_a_scale_of_their_own() {
        let mut s = Session::new();
        let id = two_scale_graph(&mut s);
        s.execute("graph.setType", &json!({"seriesIndexes": [1], "valueAxis": "right"})).unwrap();
        // Assigned, but drawn on one scale until separate scales are on with both sides.
        assert_eq!(spec(&s, id).right_series, [1]);
        s.execute("graph.setType", &json!({"separateScales": true})).unwrap();
        assert!((marks(&s, id, 0)[0].height() - 2.0).abs() < 1e-6, "a's 10 of 1000");
        s.execute("graph.setType", &json!({"valueAxis": "both"})).unwrap();
        // a's 10 and b's 1000 both reach the top: 0..10 on the left, 0..1000 on the right.
        let (a, b) = (marks(&s, id, 0), marks(&s, id, 1));
        assert!((a[0].y0 - 100.0).abs() < 1e-6 && (b[0].y0 - 100.0).abs() < 1e-6, "{:?} {:?}", a[0], b[0]);
        let labels = value_labels(&s, id);
        let side = |right: bool| labels.iter().filter(|(_, b)| (b.x0 > 400.0) == right).map(|(t, _)| t.clone()).collect::<Vec<_>>();
        assert_eq!(side(false), ["0", "2", "4", "6", "8", "10"]);
        assert_eq!(side(true), ["0", "200", "400", "600", "800", "1000"]);
        // The right axis' own tick values.
        s.execute("graph.setType", &json!({"rightAxisMin": 0, "rightAxisMax": 2000, "rightTicks": 4})).unwrap();
        assert!((marks(&s, id, 1)[0].y0 - 200.0).abs() < 1e-6, "1000 of 2000");
        assert!((marks(&s, id, 0)[0].y0 - 100.0).abs() < 1e-6, "the left axis is unchanged");
        let v = s.execute("graph.setType", &json!({})).unwrap();
        assert_eq!((v["separateScales"].clone(), v["rightTicks"].clone(), v["rightAxisMax"].clone()), (json!(true), json!(4), json!(2000.0)));
        // Stacked series stack on their own axis, each axis' stack in a slot of its own.
        s.execute("graph.setType", &json!({"type": "stackedColumn"})).unwrap();
        let (a, b) = (marks(&s, id, 0), marks(&s, id, 1));
        assert!((a[0].y1 - 300.0).abs() < 1e-6 && (b[0].y1 - 300.0).abs() < 1e-6, "{:?} {:?}", a[0], b[0]);
        assert!(b[0].x0 >= a[0].x1 - 1e-6 && (a[0].width() - b[0].width()).abs() < 1e-6, "side by side: {:?} {:?}", a[0], b[0]);
        // A line on the right axis follows the right scale.
        s.execute("graph.setType", &json!({"type": "column"})).unwrap();
        s.execute("graph.setType", &json!({"seriesIndexes": [1], "type": "line"})).unwrap();
        let line = marks(&s, id, 1);
        assert!((line[1].center().y - 200.0).abs() < 1e-6 && (line[2].center().y - 250.0).abs() < 1e-6, "1000 and 500 of 2000: {line:?}");
    }

    #[test]
    fn group_selected_series_go_on_the_chosen_axis() {
        let mut s = Session::new();
        let id = two_scale_graph(&mut s);
        let grp = series(s.doc().unwrap().doc.node(id).unwrap(), 1).id;
        s.execute("select.set", &json!({"ids": [grp.0]})).unwrap();
        assert_eq!(s.execute("graph.setType", &json!({})).unwrap()["valueAxis"], "left");
        assert!(s.execute("graph.setType", &json!({"valueAxis": "both"})).is_err(), "a series takes one axis");
        s.execute("graph.setType", &json!({"valueAxis": "right"})).unwrap();
        let g = spec(&s, id);
        assert_eq!((g.right_series.clone(), g.value_axis), (vec![1], vectorcraft_doc::ValueAxisSide::Left));
        assert_eq!(s.execute("graph.setType", &json!({})).unwrap()["valueAxis"], "right");
        s.execute("graph.setType", &json!({"valueAxis": "left"})).unwrap();
        assert!(spec(&s, id).right_series.is_empty());
    }

    #[test]
    fn bar_graphs_keep_their_value_axis_along_the_bottom() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
        let id = graph(&mut s, "bar");
        let before = (value_labels(&s, id), marks(&s, id, 0));
        s.execute("graph.setType", &json!({"valueAxis": "both", "separateScales": true, "seriesIndexes": [1]})).unwrap_err();
        s.execute("graph.setType", &json!({"valueAxis": "both", "separateScales": true})).unwrap();
        s.execute("graph.setType", &json!({"seriesIndexes": [1], "valueAxis": "right"})).unwrap();
        assert_eq!((value_labels(&s, id), marks(&s, id, 0)), before);
    }

    #[test]
    fn value_axis_options_survive_save_and_open_and_are_left_out_when_unset() {
        let mut s = Session::new();
        let id = two_scale_graph(&mut s);
        s.execute("graph.setType", &json!({"valueAxis": "both", "separateScales": true, "rightAxisMin": 0, "rightAxisMax": 2000})).unwrap();
        s.execute("graph.setType", &json!({"seriesIndexes": [1], "valueAxis": "right"})).unwrap();
        let path = vectorcraft_testkit::temp_dir("graph-axes").join("axes.vectorcraft");
        s.execute("document.save", &json!({"path": path})).unwrap();
        s.execute("document.open", &json!({"path": path})).unwrap();
        let g = spec(&s, id);
        assert_eq!(
            (g.value_axis, g.separate_scales, g.right_series, g.right_axis_max),
            (vectorcraft_doc::ValueAxisSide::Both, true, vec![1], Some(2000.0))
        );
        let v = serde_json::to_value(GraphSpec::default()).unwrap();
        for k in ["valueAxis", "separateScales", "rightSeries", "rightTicks", "rightAxisMin", "rightAxisMax"] {
            assert!(v.get(k).is_none(), "{k}");
        }
    }

    #[test]
    fn the_dialog_round_trip_keeps_series_on_both_axes_where_they_are() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
        let csv = ",a,b,c\nQ1,10,1000,3\nQ2,5,500,4";
        let id = NodeId(
            s.execute("graph.create", &json!({"type": "column", "x": 100, "y": 100, "width": 300, "height": 200, "csv": csv})).unwrap()["id"]
                .as_u64()
                .unwrap(),
        );
        s.execute("graph.setType", &json!({"seriesIndexes": [1], "valueAxis": "right"})).unwrap();
        for side in ["both", "left"] {
            s.execute("graph.setType", &json!({"valueAxis": side, "separateScales": true})).unwrap();
            let n = s.doc().unwrap().doc.node(id).unwrap().clone();
            s.execute("select.set", &json!({"ids": [series(&n, 0).id.0, series(&n, 1).id.0]})).unwrap();
            let fields = s.execute("graph.setType", &json!({})).unwrap();
            assert!(fields["valueAxis"].is_null(), "{side}: {fields}");
            s.execute("graph.setType", &fields).unwrap();
            let g = spec(&s, id);
            assert_eq!((g.right_series, g.value_axis.id()), (vec![1], side), "{side}");
            s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
        }
    }

    #[test]
    fn an_axis_with_no_series_shows_the_other_scale() {
        let mut s = Session::new();
        let id = two_scale_graph(&mut s);
        s.execute("graph.setType", &json!({"valueAxis": "both", "separateScales": true})).unwrap();
        let labels = |s: &Session| {
            let l = value_labels(s, id);
            let side = |right: bool| l.iter().filter(|(_, b)| (b.x0 > 400.0) == right).map(|(t, _)| t.clone()).collect::<Vec<_>>();
            (side(false), side(true))
        };
        let (left, right) = labels(&s);
        assert_eq!(left, right);
        assert_eq!(right.last().unwrap(), "1000");
        // Every series on the right: the left axis shows the right scale.
        s.execute("graph.setType", &json!({"seriesIndexes": [0, 1], "valueAxis": "right", "rightAxisMin": 0, "rightAxisMax": 2000})).unwrap();
        let (left, right) = labels(&s);
        assert_eq!(left, right);
        assert_eq!(right.last().unwrap(), "2000");
    }

    #[test]
    fn right_axis_columns_start_at_the_right_scales_zero() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
        let csv = ",a,b\nQ1,10,-500\nQ2,5,500";
        let id = NodeId(
            s.execute("graph.create", &json!({"type": "column", "x": 100, "y": 100, "width": 300, "height": 200, "csv": csv})).unwrap()["id"]
                .as_u64()
                .unwrap(),
        );
        s.execute("graph.setType", &json!({"valueAxis": "both", "separateScales": true, "seriesIndexes": [1]})).unwrap_err();
        s.execute("graph.setType", &json!({"valueAxis": "both", "separateScales": true})).unwrap();
        s.execute("graph.setType", &json!({"seriesIndexes": [1], "valueAxis": "right"})).unwrap();
        // Right scale -600..600 (zero at y 200), left 0..10 (zero at y 300): b's columns meet at the right zero.
        let b = marks(&s, id, 1);
        assert!((b[0].y0 - 200.0).abs() < 1e-6 && (b[1].y1 - 200.0).abs() < 1e-6, "{b:?}");
        let n = s.doc().unwrap().doc.node(id).unwrap().clone();
        let at_200 = group(&n, "Axes")
            .children()
            .unwrap()
            .iter()
            .filter(|c| matches!(c.kind, NodeKind::Path { .. }))
            .filter_map(|c| c.geometric_bounds())
            .any(|bb| bb.height() < 1e-6 && (bb.y0 - 200.0).abs() < 1e-6 && (bb.width() - 300.0).abs() < 1e-6);
        assert!(at_200, "a zero line for the right scale");
    }

    #[test]
    fn data_without_a_right_axis_series_drops_its_assignment() {
        let mut s = Session::new();
        let id = two_scale_graph(&mut s);
        s.execute("graph.setType", &json!({"seriesIndexes": [1], "valueAxis": "right"})).unwrap();
        s.execute("graph.setData", &json!({"csv": ",a\nQ1,10\nQ2,5"})).unwrap();
        assert!(spec(&s, id).right_series.is_empty());
        s.execute("graph.setData", &json!({"csv": ",a,b\nQ1,10,1000\nQ2,5,500"})).unwrap();
        assert!(spec(&s, id).right_series.is_empty(), "a new series starts on the left");
    }

    /// Bounds of the straight lines in the Axes group.
    fn axis_lines(s: &Session, id: NodeId) -> Vec<vectorcraft_geom::Rect> {
        let n = s.doc().unwrap().doc.node(id).unwrap().clone();
        group(&n, "Axes")
            .children()
            .unwrap()
            .iter()
            .filter(|c| matches!(c.kind, NodeKind::Path { .. }))
            .filter_map(|c| c.geometric_bounds())
            .collect()
    }

    fn axis_texts(s: &Session, id: NodeId) -> Vec<String> {
        let n = s.doc().unwrap().doc.node(id).unwrap().clone();
        group(&n, "Axes")
            .children()
            .unwrap()
            .iter()
            .filter_map(|c| match &c.kind {
                NodeKind::Text(t) => Some(t.plain_text()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn value_axis_tick_marks_take_a_length_and_a_count_per_division() {
        let mut s = Session::new();
        let id = two_scale_graph(&mut s);
        let short = |s: &Session, x0: f64| axis_lines(s, id).iter().filter(|b| (b.x0 - x0).abs() < 1e-6 && (b.width() - 4.0).abs() < 1e-6).count();
        let across = |s: &Session| axis_lines(s, id).iter().filter(|b| b.height() < 1e-6 && (b.width() - 300.0).abs() < 1e-6).count();
        // Short (the default): one at each of the six labels; the zero line runs across.
        assert_eq!((short(&s, 96.0), across(&s)), (6, 1));
        s.execute("graph.setType", &json!({"tickLength": "full"})).unwrap();
        // Across the plot at 200…1000; the one at 0 would be the zero line again.
        assert_eq!((short(&s, 96.0), across(&s)), (0, 6));
        s.execute("graph.setType", &json!({"tickLength": "none"})).unwrap();
        assert_eq!((short(&s, 96.0), across(&s)), (0, 1));
        // Two per division: one more halfway between labels, none past the last label.
        s.execute("graph.setType", &json!({"tickLength": "short", "tickMarks": 2})).unwrap();
        assert_eq!(short(&s, 96.0), 11);
        assert!(axis_lines(&s, id).iter().any(|b| (b.x0 - 96.0).abs() < 1e-6 && (b.y0 - 280.0).abs() < 1e-6), "100 of 0..1000 sits at y 280");
        assert_eq!(axis_texts(&s, id).iter().filter(|t| t.parse::<f64>().is_ok()).count(), 6, "labels only at divisions");
        // The right axis has its own.
        s.execute("graph.setType", &json!({"valueAxis": "both", "rightTickLength": "none"})).unwrap();
        assert_eq!((short(&s, 96.0), short(&s, 400.0)), (11, 0));
        s.execute("graph.setType", &json!({"rightTickLength": "short", "rightTickMarks": 4})).unwrap();
        assert_eq!(short(&s, 400.0), 21);
        let v = s.execute("graph.setType", &json!({})).unwrap();
        assert_eq!((v["tickLength"].clone(), v["tickMarks"].clone(), v["rightTickMarks"].clone()), (json!("short"), json!(2), json!(4)));
        assert!(s.execute("graph.setType", &json!({"tickLength": "long"})).is_err());
        s.execute("graph.setType", &json!({"tickMarks": 1_000_000})).unwrap();
        assert_eq!(spec(&s, id).tick_marks, 20);
    }

    #[test]
    fn category_tick_marks_go_at_or_between_the_labels() {
        let mut s = Session::new();
        let id = two_scale_graph(&mut s);
        let below = |s: &Session| {
            let mut xs: Vec<f64> = axis_lines(s, id)
                .iter()
                .filter(|b| b.width() < 1e-6 && (b.y0 - 300.0).abs() < 1e-6 && (b.height() - 4.0).abs() < 1e-6)
                .map(|b| b.x0)
                .collect();
            xs.sort_by(f64::total_cmp);
            xs
        };
        // None by default, as before.
        assert!(below(&s).is_empty());
        s.execute("graph.setType", &json!({"categoryTickLength": "short"})).unwrap();
        assert_eq!(below(&s), [175.0, 325.0]);
        s.execute("graph.setType", &json!({"ticksBetweenLabels": true})).unwrap();
        assert_eq!(below(&s), [100.0, 250.0, 400.0]);
        s.execute("graph.setType", &json!({"categoryTickMarks": 2})).unwrap();
        assert_eq!(below(&s), [100.0, 175.0, 250.0, 325.0, 400.0]);
        s.execute("graph.setType", &json!({"categoryTickLength": "full"})).unwrap();
        let full = axis_lines(&s, id).iter().filter(|b| b.width() < 1e-6 && (b.height() - 200.0).abs() < 1e-6 && b.x0 > 100.0 + 1e-6).count();
        assert_eq!(full, 4, "inside the plot: 175, 250, 325 and 400");
        // Bar graphs: the category axis is the left one.
        let bar = graph(&mut s, "bar");
        s.execute("graph.setType", &json!({"id": bar.0, "categoryTickLength": "short", "ticksBetweenLabels": true})).unwrap();
        let left = axis_lines(&s, bar).iter().filter(|b| b.height() < 1e-6 && (b.x0 - 96.0).abs() < 1e-6 && (b.width() - 4.0).abs() < 1e-6).count();
        assert_eq!(left, 4, "three categories, four edges");
    }

    #[test]
    fn value_axis_numbers_take_a_prefix_and_a_suffix() {
        let mut s = Session::new();
        let id = two_scale_graph(&mut s);
        s.execute("graph.setType", &json!({"prefix": "$", "suffix": " M", "valueAxis": "both", "separateScales": true})).unwrap();
        s.execute("graph.setType", &json!({"seriesIndexes": [1], "valueAxis": "right", "rightSuffix": "%"})).unwrap();
        let texts = axis_texts(&s, id);
        assert!(texts.contains(&"$10 M".to_string()) && texts.contains(&"$0 M".to_string()), "{texts:?}");
        assert!(texts.contains(&"1000%".to_string()) && !texts.contains(&"$1000 M".to_string()), "{texts:?}");
        // One short line: control characters dropped, the length capped.
        s.execute("graph.setType", &json!({"prefix": format!("a\nb{}", "x".repeat(500))})).unwrap();
        let p = spec(&s, id).prefix;
        assert!(p.starts_with("ab") && p.chars().count() == 64, "{p}");
        let v = s.execute("graph.setType", &json!({})).unwrap();
        assert_eq!((v["suffix"].clone(), v["rightSuffix"].clone()), (json!(" M"), json!("%")));
    }

    #[test]
    fn tick_and_label_options_survive_save_and_open_and_are_left_out_when_unset() {
        let mut s = Session::new();
        let id = two_scale_graph(&mut s);
        s.execute(
            "graph.setType",
            &json!({"tickLength": "full", "tickMarks": 3, "categoryTickLength": "short", "ticksBetweenLabels": true, "suffix": "%"}),
        )
        .unwrap();
        let path = vectorcraft_testkit::temp_dir("graph-ticks").join("ticks.vectorcraft");
        s.execute("document.save", &json!({"path": path})).unwrap();
        s.execute("document.open", &json!({"path": path})).unwrap();
        let g = spec(&s, id);
        assert_eq!(
            (g.tick_length, g.tick_marks, g.category_tick_length, g.ticks_between_labels, g.suffix.as_str()),
            (vectorcraft_doc::TickLength::Full, 3, vectorcraft_doc::TickLength::Short, true, "%")
        );
        let v = serde_json::to_value(GraphSpec::default()).unwrap();
        for k in [
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
        ] {
            assert!(v.get(k).is_none(), "{k}");
        }
        // A file's huge values draw within the caps.
        let g: GraphSpec = serde_json::from_value(json!({"rows": [[1.0]], "tickMarks": 1_000_000_000u64, "prefix": "y".repeat(10_000)})).unwrap();
        let mut d = vectorcraft_doc::Document::new(800.0, 800.0);
        let art = super::generate(&mut d, &g);
        let axes = art.iter().find(|n| n.name.as_deref() == Some("Axes")).unwrap().children().unwrap().to_vec();
        assert!(axes.len() < 200, "{} axis parts", axes.len());
        for t in axes.iter().filter_map(|n| match &n.kind {
            NodeKind::Text(t) => Some(t.plain_text()),
            _ => None,
        }) {
            assert!(t.chars().count() < 80, "{} characters", t.chars().count());
        }
    }

    #[test]
    fn between_labels_ticks_on_an_edge_to_edge_graph_fall_halfway_between_its_labels() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
        let id = graph(&mut s, "area");
        s.execute("graph.setType", &json!({"categoryTickLength": "short", "ticksBetweenLabels": true})).unwrap();
        // Labels at 100, 250 and 400 (the plot's edges and middle): ticks at 175 and 325.
        let mut xs: Vec<f64> = axis_lines(&s, id)
            .iter()
            .filter(|b| b.width() < 1e-6 && (b.y0 - 300.0).abs() < 1e-6 && (b.height() - 4.0).abs() < 1e-6)
            .map(|b| b.x0)
            .collect();
        xs.sort_by(f64::total_cmp);
        assert_eq!(xs, [175.0, 325.0]);
    }

    #[test]
    fn bar_graph_value_ticks_run_up_the_plot_and_long_labels_push_the_legend() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
        let id = graph(&mut s, "bar");
        let legend_x = |s: &Session| {
            let n = s.doc().unwrap().doc.node(id).unwrap().clone();
            series(&n, 0).children().unwrap().last().unwrap().geometric_bounds().unwrap().x0
        };
        let before = legend_x(&s);
        assert!((before - 414.0).abs() < 1e-6, "a short last label leaves the legend where it was: {before}");
        s.execute("graph.setType", &json!({"tickLength": "full", "tickMarks": 2})).unwrap();
        let up: Vec<_> = axis_lines(&s, id).into_iter().filter(|b| b.width() < 1e-6 && (b.height() - 200.0).abs() < 1e-6).collect();
        // -2..6 in 2s: five labels and four halves; the one at 0 is the zero line already, the left end the axis.
        assert_eq!(up.len(), 9, "{up:?}");
        s.execute("graph.setType", &json!({"suffix": " million tonnes"})).unwrap();
        let last = value_label_edge(&s, id);
        assert!(legend_x(&s) >= last + 6.0 - 1e-6, "{} vs {last}", legend_x(&s));
    }

    /// The right end of the Axes group's texts.
    fn value_label_edge(s: &Session, id: NodeId) -> f64 {
        let n = s.doc().unwrap().doc.node(id).unwrap().clone();
        group(&n, "Axes")
            .children()
            .unwrap()
            .iter()
            .filter(|c| matches!(c.kind, NodeKind::Text(_)))
            .filter_map(|c| c.geometric_bounds())
            .map(|b| b.x1)
            .fold(f64::MIN, f64::max)
    }

    #[test]
    fn counts_come_back_from_the_dialog_as_floats_and_are_capped() {
        let mut s = Session::new();
        let id = two_scale_graph(&mut s);
        s.execute("graph.setType", &json!({"tickMarks": 3.0, "ticks": 4.0, "categoryTickMarks": 2.6})).unwrap();
        let g = spec(&s, id);
        assert_eq!((g.tick_marks, g.ticks, g.category_tick_marks), (3, 4, 3));
        s.execute("graph.setType", &json!({"categoryTickMarks": 0, "rightTickMarks": 1e300, "rightTicks": -2.0})).unwrap();
        let g = spec(&s, id);
        assert_eq!((g.category_tick_marks, g.right_tick_marks, g.right_ticks), (1, 20, 0));
        // A file's values come back as drawn.
        let mut fields = s.execute("graph.setType", &json!({})).unwrap();
        assert_eq!(fields["rightTickMarks"], 20);
        fields["tickLength"] = json!("full");
        s.execute("graph.setType", &fields).unwrap();
        assert_eq!(spec(&s, id).tick_length, vectorcraft_doc::TickLength::Full);
    }

    #[test]
    fn a_long_category_axis_is_ticked_all_the_way_within_the_cap() {
        let rows: Vec<Vec<f64>> = (0..5000).map(|i| vec![i as f64]).collect();
        let g = GraphSpec { rows, category_tick_length: vectorcraft_doc::TickLength::Short, category_tick_marks: 20, ..GraphSpec::default() };
        let mut d = vectorcraft_doc::Document::new(800.0, 800.0);
        let art = super::generate(&mut d, &g);
        let axes = art.iter().find(|n| n.name.as_deref() == Some("Axes")).unwrap();
        let ticks: Vec<f64> = axes
            .children()
            .unwrap()
            .iter()
            .filter_map(|c| c.geometric_bounds())
            .filter(|b| b.width() < 1e-9 && (b.height() - 4.0).abs() < 1e-9)
            .map(|b| b.x0)
            .collect();
        assert!(ticks.len() <= 10_000 && ticks.len() >= 5000, "{}", ticks.len());
        assert!(ticks.iter().copied().fold(f64::MIN, f64::max) > 199.0, "the last category is ticked");
    }

    /// A graph design: a 10 × 10 rectangle at the back and a 20-wide ellipse over it, saved as `name`.
    fn save_design(s: &mut Session, name: &str) {
        let r = s.execute("shape.rectangle", &json!({"x": 600, "y": 600, "width": 10, "height": 10})).unwrap()["id"].as_u64().unwrap();
        let e = s.execute("shape.ellipse", &json!({"x": 595, "y": 600, "width": 20, "height": 10})).unwrap()["id"].as_u64().unwrap();
        s.execute("select.set", &json!({"ids": [r, e]})).unwrap();
        s.execute("graph.design", &json!({"save": name})).unwrap();
    }

    /// The marks of series `index` (legend swatch last) and their bounds.
    fn series_parts(s: &Session, id: NodeId, index: u32) -> Vec<vectorcraft_doc::Node> {
        let n = s.doc().unwrap().doc.node(id).unwrap().clone();
        series(&n, index).children().unwrap().iter().map(|c| (**c).clone()).collect()
    }

    #[test]
    fn a_marker_design_draws_each_data_point_sized_by_its_backmost_object() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
        save_design(&mut s, "Pill");
        assert_eq!(s.execute("graph.design", &json!({})).unwrap()["designs"], json!(["Pill"]));
        let id = graph(&mut s, "line");
        let before = series_parts(&s, id, 1);
        let grp = series(s.doc().unwrap().doc.node(id).unwrap(), 1).id;
        s.execute("select.set", &json!({"ids": [grp.0]})).unwrap();
        s.execute("graph.marker", &json!({"design": "Pill"})).unwrap();
        let after = series_parts(&s, id, 1);
        assert_eq!(after.len(), before.len(), "a line, three markers and the swatch");
        for (old, new) in before.iter().zip(&after).skip(1) {
            let (o, n) = (old.geometric_bounds().unwrap(), new.geometric_bounds().unwrap());
            assert!(matches!(new.kind, NodeKind::Group { .. }), "the design's art");
            // The 10-point rectangle fills the marker's square, the 20-wide ellipse twice as wide.
            assert!((n.center() - o.center()).hypot() < 1e-6, "{o:?} {n:?}");
            assert!((n.width() - 2.0 * o.width()).abs() < 1e-6 && (n.height() - o.height()).abs() < 1e-6, "{o:?} {n:?}");
        }
        // The series stays selected; the other series keeps its squares; the query names the design.
        let grp = series(s.doc().unwrap().doc.node(id).unwrap(), 1).id;
        assert_eq!(s.doc().unwrap().selection.objects.to_vec(), [grp]);
        assert!(series_parts(&s, id, 0).iter().all(|n| matches!(n.kind, NodeKind::Path { .. })));
        assert_eq!(s.execute("graph.marker", &json!({})).unwrap()["design"], "Pill");
        assert_eq!(spec(&s, id).series_markers, [None, Some("Pill".to_string())]);
        // Back to the default square.
        s.execute("graph.marker", &json!({"design": null})).unwrap();
        assert!(spec(&s, id).series_markers.is_empty());
        assert!(series_parts(&s, id, 1).iter().all(|n| matches!(n.kind, NodeKind::Path { .. })));
        assert!(s.execute("graph.marker", &json!({"design": "Nope"})).is_err());
        assert!(s.execute("graph.marker", &json!({"design": 3})).is_err());
    }

    #[test]
    fn marker_designs_apply_to_markers_not_to_columns() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
        save_design(&mut s, "Pill");
        let id = graph(&mut s, "column");
        s.execute("graph.setType", &json!({"seriesIndexes": [1], "type": "line"})).unwrap();
        // Every series (none picked): the line series' markers and swatch; the columns and their swatch stay.
        s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
        s.execute("graph.marker", &json!({"design": "Pill"})).unwrap();
        assert!(series_parts(&s, id, 0).iter().all(|n| matches!(n.kind, NodeKind::Path { .. })));
        let line = series_parts(&s, id, 1);
        assert!(line.iter().skip(1).all(|n| matches!(n.kind, NodeKind::Group { .. })), "markers and swatch");
        let swatch = line.last().unwrap().geometric_bounds().unwrap();
        assert!((swatch.width() - 16.0).abs() < 1e-6 && (swatch.height() - 8.0).abs() < 1e-6, "{swatch:?}");
        // Scatter and radar graphs draw them too.
        for ty in ["scatter", "radar"] {
            let g = graph(&mut s, ty);
            s.execute("select.set", &json!({"ids": [g.0]})).unwrap();
            s.execute("graph.marker", &json!({"design": "Pill"})).unwrap();
            assert!(series_parts(&s, g, 0).iter().any(|n| matches!(n.kind, NodeKind::Group { .. })), "{ty}");
        }
    }

    #[test]
    fn graph_designs_are_saved_pasted_and_deleted() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
        assert!(s.execute("graph.design", &json!({"save": "Pill"})).is_err(), "nothing selected");
        save_design(&mut s, "Pill");
        let r = s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap()["id"].as_u64().unwrap();
        s.execute("select.set", &json!({"ids": [r]})).unwrap();
        assert!(s.execute("graph.design", &json!({"save": "Pill"})).is_err(), "the name is taken");
        assert!(s.execute("graph.design", &json!({"save": "  "})).is_err(), "no name");
        // Paste Design: a copy of the art, selected, with ids of its own.
        let pasted = NodeId(s.execute("graph.design", &json!({"paste": "Pill"})).unwrap()["id"].as_u64().unwrap());
        assert_eq!(s.doc().unwrap().selection.objects.to_vec(), [pasted]);
        let art = s.doc().unwrap().doc.graph_designs[0].art.clone();
        let copy = s.doc().unwrap().doc.node(pasted).unwrap().clone();
        assert_ne!(copy.id, art.id);
        assert_eq!(copy.geometric_bounds(), art.geometric_bounds());
        assert!(s.execute("graph.design", &json!({"paste": "Nope"})).is_err());
        // Deleting a design draws its graphs' default markers again; undo brings both back.
        let id = graph(&mut s, "line");
        s.execute("graph.marker", &json!({"design": "Pill"})).unwrap();
        s.execute("graph.design", &json!({"delete": "Pill"})).unwrap();
        assert!(s.doc().unwrap().doc.graph_designs.is_empty());
        assert!(series_parts(&s, id, 0).iter().all(|n| matches!(n.kind, NodeKind::Path { .. })));
        s.execute("edit.undo", &json!({})).unwrap();
        assert!(series_parts(&s, id, 0).iter().skip(1).all(|n| matches!(n.kind, NodeKind::Group { .. })));
        assert!(s.execute("graph.design", &json!({"delete": "Nope"})).is_err());
    }

    #[test]
    fn designs_and_markers_survive_save_and_open() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
        save_design(&mut s, "Pill");
        let id = graph(&mut s, "line");
        s.execute("graph.marker", &json!({"seriesIndexes": [0], "design": "Pill"})).unwrap();
        let path = vectorcraft_testkit::temp_dir("graph-designs").join("designs.vectorcraft");
        s.execute("document.save", &json!({"path": path})).unwrap();
        s.execute("document.open", &json!({"path": path})).unwrap();
        assert_eq!(s.execute("graph.design", &json!({})).unwrap()["designs"], json!(["Pill"]));
        assert_eq!(spec(&s, id).series_markers, [Some("Pill".to_string())]);
        assert!(serde_json::to_value(GraphSpec::default()).unwrap().get("seriesMarkers").is_none());
        // A design whose art has no area draws the default square.
        let g = GraphSpec { kind: GraphKind::Line, series_markers: vec![Some("Dot".into())], ..GraphSpec::default() };
        let mut d = vectorcraft_doc::Document::new(800.0, 800.0);
        let dot = vectorcraft_doc::Node::group(d.alloc_id(), vec![]);
        d.graph_designs.push(vectorcraft_doc::GraphDesign { name: "Dot".into(), art: std::sync::Arc::new(dot) });
        let art = super::generate(&mut d, &g);
        let s0 = art.iter().find(|n| n.series_index == Some(0)).unwrap();
        assert!(s0.children().unwrap().iter().all(|n| matches!(n.kind, NodeKind::Path { .. })));
    }

    #[test]
    fn a_graph_saved_as_a_design_is_plain_art_even_as_its_own_marker() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
        save_design(&mut s, "Pill");
        let id = graph(&mut s, "line");
        s.execute("graph.marker", &json!({"design": "Pill"})).unwrap();
        s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
        s.execute("graph.design", &json!({"save": "Chart"})).unwrap();
        let art = s.doc().unwrap().doc.graph_designs[1].art.clone();
        let mut graphs = 0;
        art.walk(&mut |n| graphs += usize::from(n.graph.is_some() || n.series_index.is_some()));
        assert_eq!(graphs, 0, "no graph inside a design");
        // The graph drawn with itself as a marker, then the design it used deleted: still one graph to edit.
        s.execute("graph.marker", &json!({"id": id.0, "seriesIndexes": [1], "design": "Chart"})).unwrap();
        s.execute("graph.design", &json!({"delete": "Pill"})).unwrap();
        let mut found = 0;
        s.doc().unwrap().doc.walk(|n| found += usize::from(n.graph.is_some()));
        assert_eq!(found, 1);
        // A deleted design reads as none in the query, so the dialog opens on the default marker.
        s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
        s.execute("graph.marker", &json!({"seriesIndexes": [0], "design": null})).unwrap();
        let v = s.execute("graph.marker", &json!({"seriesIndexes": [1, 0]})).unwrap();
        assert!(v["design"].is_null(), "{v}");
    }

    #[test]
    fn designs_too_big_for_a_graph_are_refused_and_drawn_as_squares() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
        // 1,500 objects at each of 200 points is past the budget.
        let ids: Vec<u64> = (0..1500)
            .map(|i| s.execute("shape.rectangle", &json!({"x": i % 40, "y": i / 40, "width": 1, "height": 1})).unwrap()["id"].as_u64().unwrap())
            .collect();
        s.execute("select.set", &json!({"ids": ids})).unwrap();
        s.execute("graph.design", &json!({"save": "Big"})).unwrap();
        let rows: Vec<Vec<f64>> = (0..200).map(|i| vec![i as f64]).collect();
        let id = NodeId(
            s.execute("graph.create", &json!({"type": "line", "x": 0, "y": 0, "width": 300, "height": 200, "rows": rows})).unwrap()["id"]
                .as_u64()
                .unwrap(),
        );
        assert!(s.execute("graph.marker", &json!({"id": id.0, "design": "Big"})).is_err());
        assert!(spec(&s, id).series_markers.is_empty());
        // A file asking for it anyway draws the default squares.
        let mut g = spec(&s, id);
        g.series_markers = vec![Some("Big".into())];
        let mut d = (*s.doc().unwrap().doc).clone();
        let art = super::generate(&mut d, &g);
        let s0 = art.iter().find(|n| n.series_index == Some(0)).unwrap();
        assert!(s0.children().unwrap().iter().all(|n| matches!(n.kind, NodeKind::Path { .. })));
        // Too many objects for one design.
        let ids: Vec<u64> = (0..2001)
            .map(|i| s.execute("shape.rectangle", &json!({"x": i % 40, "y": i / 40, "width": 1, "height": 1})).unwrap()["id"].as_u64().unwrap())
            .collect();
        s.execute("select.set", &json!({"ids": ids})).unwrap();
        assert!(s.execute("graph.design", &json!({"save": "Huge"})).is_err());
    }

    #[test]
    fn a_files_designs_are_tidied() {
        let mut d = vectorcraft_doc::Document::new(100.0, 100.0);
        let art = |d: &mut vectorcraft_doc::Document| std::sync::Arc::new(vectorcraft_doc::Node::group(d.alloc_id(), vec![]));
        let mut graphy = vectorcraft_doc::Node::group(d.alloc_id(), vec![]);
        graphy.graph = Some(Box::new(GraphSpec::default()));
        for name in ["  A\u{7}  ", "A", "", "   ", &"x".repeat(100)] {
            let a = art(&mut d);
            d.graph_designs.push(vectorcraft_doc::GraphDesign { name: name.into(), art: a });
        }
        d.graph_designs.push(vectorcraft_doc::GraphDesign { name: "G".into(), art: std::sync::Arc::new(graphy) });
        d.tidy_graph_designs();
        let names: Vec<_> = d.graph_designs.iter().map(|x| x.name.clone()).collect();
        assert_eq!(names, ["A".to_string(), "x".repeat(64), "G".to_string()]);
        assert!(d.graph_designs[2].art.graph.is_none());
    }

    #[test]
    fn painting_a_series_drawn_with_a_design_leaves_the_design_alone() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
        save_design(&mut s, "Pill");
        let id = graph(&mut s, "line");
        s.execute("graph.marker", &json!({"design": "Pill"})).unwrap();
        let grp = series(s.doc().unwrap().doc.node(id).unwrap(), 0).id;
        s.execute("paint.setFill", &json!({"ids": [grp.0], "color": "#f00"})).unwrap();
        // The design's paths keep the design's paint, before and after the graph is drawn again.
        let design_fill = |s: &Session| {
            let parts = series_parts(s, id, 0);
            let mut fills = vec![];
            parts[1].walk(&mut |n| {
                if let NodeKind::Path { .. } = n.kind {
                    fills.push(n.appearance.fill_paint().color().map(|c| c.to_hex()));
                }
            });
            fills
        };
        let painted = design_fill(&s);
        assert!(!painted.contains(&Some("#ff0000".into())), "{painted:?}");
        s.execute("graph.setData", &json!({"id": id.0, "rows": [[1, 2], [3, 4]]})).unwrap();
        assert_eq!(design_fill(&s), painted);
    }

    #[test]
    fn graph_design_params_are_names() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
        for p in [json!({"save": 5}), json!({"paste": null}), json!({"delete": ["a"]})] {
            assert!(s.execute("graph.design", &p).is_err(), "{p}");
        }
    }

    #[test]
    fn document_wide_passes_see_design_art() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
        let r = s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap()["id"].as_u64().unwrap();
        s.execute("paint.setFill", &json!({"ids": [r], "color": "#123456"})).unwrap();
        s.execute("select.set", &json!({"ids": [r]})).unwrap();
        s.execute("graph.design", &json!({"save": "Blue"})).unwrap();
        s.execute("edit.clear", &json!({})).unwrap();
        let mut seen = false;
        s.doc().unwrap().doc.visit_paints(&mut |p| seen |= p.color().is_some_and(|c| c.to_hex() == "#123456"));
        assert!(seen, "the colour is used by the design");
    }

    fn make(s: &mut Session, ty: &str, csv: &str) -> NodeId {
        NodeId(
            s.execute("graph.create", &json!({"type": ty, "x": 100, "y": 100, "width": 300, "height": 200, "csv": csv})).unwrap()["id"]
                .as_u64()
                .unwrap(),
        )
    }

    /// Where each wedge of series `index` starts: its angle from 12 o'clock, clockwise, in degrees (the legend
    /// swatch, last, left out).
    fn wedge_starts(s: &Session, id: NodeId, index: u32) -> Vec<f64> {
        let parts = series_parts(s, id, index);
        parts[..parts.len() - 1]
            .iter()
            .filter_map(|n| match &n.kind {
                NodeKind::Path { path, .. } => {
                    let bp = path.to_bezpath();
                    let pts: Vec<_> = bp.elements().iter().filter_map(|e| e.end_point()).collect();
                    let (c, p) = (pts.first()?, pts.get(1)?);
                    let deg = (p.x - c.x).atan2(-(p.y - c.y)).to_degrees();
                    Some(if deg < -1e-6 { deg + 360.0 } else { deg })
                }
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_bar_in_a_label_breaks_the_line() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
        let id = make(&mut s, "column", ",Total|Sales|2023,b\nQ1|first,3,2\nQ2,5,4");
        let texts = axis_texts(&s, id);
        assert!(texts.contains(&"Q1\nfirst".to_string()), "{texts:?}");
        // The legend: a label of three lines, and the next row below all three.
        let n = s.doc().unwrap().doc.node(id).unwrap().clone();
        let legend: Vec<_> = group(&n, "Legend").children().unwrap().iter().map(|c| (**c).clone()).collect();
        let NodeKind::Text(t) = &legend[0].kind else { panic!() };
        assert_eq!(t.plain_text(), "Total\nSales\n2023");
        let (a, b) = (
            series_parts(&s, id, 0).last().unwrap().geometric_bounds().unwrap(),
            series_parts(&s, id, 1).last().unwrap().geometric_bounds().unwrap(),
        );
        assert!((b.y0 - a.y0 - 3.0 * 9.0 * 1.8).abs() < 1e-6, "{a:?} {b:?}");
    }

    #[test]
    fn transpose_swaps_rows_and_columns_and_switch_xy_swaps_scatter_pairs() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
        let id = make(&mut s, "column", ",a,b\nQ1,1,2\nQ2,3,");
        let before = spec(&s, id);
        s.execute("graph.setData", &json!({"transpose": true})).unwrap();
        let g = spec(&s, id);
        assert_eq!((g.series.clone(), g.categories.clone()), (vec!["Q1".to_string(), "Q2".into()], vec!["a".to_string(), "b".into()]));
        assert_eq!(g.cells(), [[Some(1.0), Some(3.0)], [Some(2.0), None]]);
        s.execute("graph.setData", &json!({"transpose": true})).unwrap();
        assert_eq!(spec(&s, id).cells(), before.cells());
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(spec(&s, id).cells(), g.cells());
        assert!(s.execute("graph.setData", &json!({"switchXY": true})).is_err(), "scatter graphs only");
        let sc = make(&mut s, "scatter", ",y1,x1,y2,x2\nP,1,10,2,20");
        s.execute("graph.setData", &json!({"switchXY": true})).unwrap();
        let g = spec(&s, sc);
        assert_eq!(g.cells(), [[Some(10.0), Some(1.0), Some(20.0), Some(2.0)]]);
        assert_eq!(g.series, ["x1", "y1", "x2", "y2"]);
    }

    #[test]
    fn pie_wedges_sort_largest_first_in_each_pie_or_in_the_first_pies_order() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
        let id = make(&mut s, "pie", ",a,b,c\nP1,1,3,2\nP2,3,1,2");
        // None (data order): a starts at 12 o'clock in both pies.
        assert_eq!(wedge_starts(&s, id, 0).iter().map(|d| d.round()).collect::<Vec<_>>(), [0.0, 0.0]);
        s.execute("graph.setType", &json!({"pieSort": "all"})).unwrap();
        // P1: b (3) first; P2: a (3) first.
        let (a, b) = (wedge_starts(&s, id, 0), wedge_starts(&s, id, 1));
        assert!(b[0].abs() < 1e-6 && a[1].abs() < 1e-6, "{a:?} {b:?}");
        s.execute("graph.setType", &json!({"pieSort": "first"})).unwrap();
        // P1's order (b, c, a) in both: b first in P2 too.
        let b = wedge_starts(&s, id, 1);
        assert!(b.iter().all(|d| d.abs() < 1e-6), "{b:?}");
        assert_eq!(s.execute("graph.setType", &json!({})).unwrap()["pieSort"], "first");
        assert!(s.execute("graph.setType", &json!({"pieSort": "random"})).is_err());
    }

    #[test]
    fn pies_sized_by_their_totals_side_by_side_or_stacked() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
        let id = make(&mut s, "pie", ",a,b\nSmall,5,5\nBig,20,20");
        let pie = |s: &Session, c: usize| {
            let wedges = |i: u32| {
                let parts = series_parts(s, id, i);
                parts[..parts.len() - 1].to_vec()
            };
            let rects: Vec<_> = (0..2).flat_map(wedges).filter_map(|n| n.geometric_bounds()).collect();
            rects.into_iter().skip(c).step_by(2).reduce(|a, b| a.union(b)).unwrap()
        };
        let even = (pie(&s, 0), pie(&s, 1));
        assert!((even.0.width() - even.1.width()).abs() < 1e-6);
        s.execute("graph.setType", &json!({"piePosition": "ratio"})).unwrap();
        let (small, big) = (pie(&s, 0), pie(&s, 1));
        // Areas in proportion to the totals: 10 and 40, so radii 1 : 2.
        assert!((big.width() / small.width() - 2.0).abs() < 1e-6, "{small:?} {big:?}");
        assert!((big.width() - even.1.width()).abs() < 1e-6, "the biggest keeps the full size");
        s.execute("graph.setType", &json!({"piePosition": "stacked"})).unwrap();
        // Drawn largest first, so the big pie's wedges come first in each series.
        let (big, small) = (pie(&s, 0), pie(&s, 1));
        assert!((small.center() - big.center()).hypot() < 1e-6 && (big.width() / small.width() - 2.0).abs() < 1e-6);
        // The big pie is drawn first, under the small one.
        let n = s.doc().unwrap().doc.node(id).unwrap().clone();
        let wedges: Vec<f64> = series(&n, 0).children().unwrap().iter().filter_map(|c| c.geometric_bounds()).map(|b| b.width()).collect();
        assert!(wedges[0] > wedges[1], "{wedges:?}");
    }

    #[test]
    fn legends_in_wedges_put_the_series_labels_inside_the_pie() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
        let id = make(&mut s, "pie", ",North,South|East\nP,1,1");
        s.execute("graph.setType", &json!({"pieLegend": "wedges"})).unwrap();
        let n = s.doc().unwrap().doc.node(id).unwrap().clone();
        let pie =
            series(&n, 0).children().unwrap()[0].geometric_bounds().unwrap().union(series(&n, 1).children().unwrap()[0].geometric_bounds().unwrap());
        let labels: Vec<_> = group(&n, "Legend").children().unwrap().iter().map(|c| (**c).clone()).collect();
        assert_eq!(labels.len(), 2);
        for l in &labels {
            let b = l.geometric_bounds().unwrap();
            assert!(pie.contains(b.center()), "{b:?} inside {pie:?}");
        }
        // No swatches beside the pie: each series group holds only its wedge.
        assert_eq!((series(&n, 0).children().unwrap().len(), series(&n, 1).children().unwrap().len()), (1, 1));
        let v = s.execute("graph.setType", &json!({})).unwrap();
        assert_eq!((v["pieLegend"].clone(), v["piePosition"].clone()), (json!("wedges"), json!("even")));
        assert!(v.get("legend").is_none(), "a pie's Legend stands for the checkbox");
        // The labels are drawn over the wedges, and on the black first wedge in white.
        let order: Vec<_> = n.children().unwrap().iter().map(|c| c.name.clone().unwrap_or_default()).collect();
        assert_eq!(order.last().map(String::as_str), Some("Legend"), "{order:?}");
        let colour = |l: &vectorcraft_doc::Node| match &l.kind {
            NodeKind::Text(t) => t.runs[0].style.fill.color().unwrap().to_hex(),
            _ => panic!(),
        };
        assert_eq!((colour(&labels[0]), colour(&labels[1])), ("#ffffff".to_string(), "#000000".to_string()));
        // No Legend wins over a legend checkbox sent along (as the dialog does), and Standard comes back without wedges.
        s.execute("graph.setType", &json!({"pieLegend": "none", "legend": true})).unwrap();
        assert!(!spec(&s, id).legend);
        s.execute("graph.setType", &json!({"legend": true})).unwrap();
        assert!(!spec(&s, id).pie_legend_in_wedges);
        assert!(s.execute("graph.setType", &json!({"pieLegend": "inside"})).is_err());
        let path = vectorcraft_testkit::temp_dir("graph-pie").join("pie.vectorcraft");
        s.execute("graph.setType", &json!({"pieLegend": "wedges", "piePosition": "stacked", "pieSort": "all"})).unwrap();
        s.execute("document.save", &json!({"path": path})).unwrap();
        s.execute("document.open", &json!({"path": path})).unwrap();
        let g = spec(&s, id);
        assert_eq!(
            (g.pie_legend_in_wedges, g.pie_position, g.pie_sort),
            (true, vectorcraft_doc::PiePosition::Stacked, vectorcraft_doc::PieSort::All)
        );
        let v = serde_json::to_value(GraphSpec::default()).unwrap();
        for k in ["pieLegendInWedges", "piePosition", "pieSort"] {
            assert!(v.get(k).is_none(), "{k}");
        }
    }

    #[test]
    fn stacked_pies_are_rings_so_none_covers_another() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
        let id = make(&mut s, "pie", ",a,b\nSmall,5,5\nBig,20,20\nSame,20,20");
        s.execute("graph.setType", &json!({"piePosition": "stacked"})).unwrap();
        let n = s.doc().unwrap().doc.node(id).unwrap().clone();
        // Series b's wedges: of the two equal big pies one is left out, the other a ring outside the small pie
        // (clear of the centre), and the small pie's whole.
        let b: Vec<_> = series(&n, 1).children().unwrap().iter().map(|c| (**c).clone()).collect();
        assert_eq!(b.len(), 3, "two wedges and the swatch");
        let small = b[1].geometric_bounds().unwrap();
        let NodeKind::Path { path, .. } = &b[0].kind else { panic!() };
        let c = small.center();
        let near: Vec<f64> = path.to_bezpath().elements().iter().filter_map(|e| e.end_point()).map(|p| (p - c).hypot()).collect();
        let min = near.iter().copied().fold(f64::MAX, f64::min);
        assert!(min > 1.0, "the ring keeps clear of the centre: {near:?}");
        // The names are drawn over the pies: after every series group.
        let kids = n.children().unwrap();
        let last_series = kids.iter().rposition(|c| c.series_index.is_some()).unwrap();
        let names: Vec<usize> = kids.iter().enumerate().filter(|(_, c)| matches!(c.kind, NodeKind::Text(_))).map(|(i, _)| i).collect();
        assert_eq!(names.len(), 2, "of two equal pies, one has no ring and no name");
        assert!(names.iter().all(|i| *i > last_series), "{names:?} after {last_series}");
    }

    #[test]
    fn transpose_keeps_within_the_series_cap_and_drops_stale_axis_series() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
        let rows: Vec<Vec<f64>> = (0..300).map(|i| vec![i as f64]).collect();
        let id = NodeId(
            s.execute("graph.create", &json!({"type": "column", "x": 0, "y": 0, "width": 300, "height": 200, "rows": rows})).unwrap()["id"]
                .as_u64()
                .unwrap(),
        );
        assert!(s.execute("graph.setData", &json!({"transpose": true})).is_err(), "300 categories can't become series");
        assert_eq!(spec(&s, id).cells().len(), 300);
        let g = make(&mut s, "column", ",a,b,c\nQ1,1,2,3");
        s.execute("graph.setType", &json!({"seriesIndexes": [2], "valueAxis": "right"})).unwrap();
        s.execute("graph.setData", &json!({"transpose": true})).unwrap();
        assert!(spec(&s, g).right_series.is_empty(), "one series left");
        // An all-blank series becomes no category.
        let h = make(&mut s, "column", ",a,b\nQ1,1,\nQ2,2,");
        s.execute("graph.setData", &json!({"transpose": true})).unwrap();
        assert_eq!(spec(&s, h).categories, ["a", "b"], "b keeps its label");
        assert!(s.execute("graph.setData", &json!({"id": h.0, "transpose": false})).unwrap().get("csv").is_some(), "no change: the query");
    }

    #[test]
    fn switch_xy_moves_the_labels_with_their_columns() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
        let id = make(&mut s, "scatter", ",y1,x1,y2\nP,1,10,2,20");
        s.execute("graph.setData", &json!({"switchXY": true})).unwrap();
        let g = spec(&s, id);
        assert_eq!(g.series, ["x1", "y1", "", "y2"]);
        assert_eq!(g.cells(), [[Some(10.0), Some(1.0), Some(20.0), Some(2.0)]]);
    }

    #[test]
    fn a_label_of_only_bars_is_blank_and_non_pie_graphs_list_no_pie_options() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
        let id = make(&mut s, "column", ",||,b\nQ1,1,2");
        let n = s.doc().unwrap().doc.node(id).unwrap().clone();
        assert_eq!(group(&n, "Legend").children().unwrap().len(), 1, "only b");
        let v = s.execute("graph.setType", &json!({})).unwrap();
        assert!(v.get("pieLegend").is_none() && v.get("legend").is_some());
        // Bar and radar labels with a `|` centre their lines on the category.
        for ty in ["bar", "radar"] {
            let g = make(&mut s, ty, ",a\nOne|Two|Three,1\nQ2,2\nQ3,3");
            assert!(axis_texts(&s, g).contains(&"One\nTwo\nThree".to_string()), "{ty}");
        }
    }

    #[test]
    fn drop_shadows_sit_behind_the_columns_lines_and_wedges() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
        let id = graph(&mut s, "column");
        s.execute("graph.setType", &json!({"dropShadow": true})).unwrap();
        let n = s.doc().unwrap().doc.node(id).unwrap().clone();
        let kids = n.children().unwrap();
        let at = kids.iter().position(|c| c.name.as_deref() == Some("Drop Shadow")).unwrap();
        assert!(kids.iter().position(|c| c.series_index.is_some()).unwrap() > at, "behind the series");
        let shadow = &kids[at];
        assert!((shadow.opacity - 0.35).abs() < 1e-6);
        let copies: Vec<_> = shadow.children().unwrap().iter().filter_map(|c| c.geometric_bounds()).collect();
        let cols: Vec<_> = (0..2).flat_map(|i| marks(&s, id, i)).collect();
        assert_eq!(copies.len(), cols.len(), "one per column, none for the swatches");
        for c in &cols {
            assert!(copies.iter().any(|b| (b.x0 - c.x0 - 2.0).abs() < 1e-6 && (b.y0 - c.y0 - 2.0).abs() < 1e-6), "{c:?}");
        }
        let black = shadow.children().unwrap().iter().all(|c| c.appearance.fill_paint().color().map(|c| c.to_hex()) == Some("#000000".into()));
        assert!(black);
        // Lines cast one too (not their markers); pies one per wedge.
        let line = graph(&mut s, "line");
        s.execute("graph.setType", &json!({"dropShadow": true})).unwrap();
        let n = s.doc().unwrap().doc.node(line).unwrap().clone();
        assert_eq!(group(&n, "Drop Shadow").children().unwrap().len(), 2, "one per series line");
        let pie = graph(&mut s, "pie");
        s.execute("graph.setType", &json!({"dropShadow": true})).unwrap();
        let n = s.doc().unwrap().doc.node(pie).unwrap().clone();
        assert_eq!(group(&n, "Drop Shadow").children().unwrap().len(), 5, "the wedges with a value above 0");
        assert_eq!(s.execute("graph.setType", &json!({})).unwrap()["dropShadow"], true);
        s.execute("graph.setType", &json!({"dropShadow": false})).unwrap();
        let n = s.doc().unwrap().doc.node(pie).unwrap().clone();
        assert!(n.children().unwrap().iter().all(|c| c.name.as_deref() != Some("Drop Shadow")));
    }

    #[test]
    fn the_legend_across_top_runs_in_rows_above_the_plot() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
        let id = graph(&mut s, "column");
        s.execute("graph.setType", &json!({"legendAcrossTop": true})).unwrap();
        let swatch = |s: &Session, i: u32| series_parts(s, id, i).last().unwrap().geometric_bounds().unwrap();
        let (a, b) = (swatch(&s, 0), swatch(&s, 1));
        assert!(a.y1 <= 100.0 - 8.0 + 1e-6 && (a.y0 - b.y0).abs() < 1e-6 && b.x0 > a.x1 && (a.x0 - 100.0).abs() < 1e-6, "{a:?} {b:?}");
        assert_eq!(s.execute("graph.setType", &json!({})).unwrap()["legendAcrossTop"], true);
        // Long labels wrap into rows within the plot's width, the last row just above it.
        let labels: Vec<String> = (0..6).map(|i| format!("A rather long series name {i}")).collect();
        let csv = format!(",{}\nQ1,1,2,3,4,5,6", labels.join(","));
        s.execute("graph.setData", &json!({"id": id.0, "csv": csv})).unwrap();
        let ys: Vec<f64> = (0..6).map(|i| swatch(&s, i).y0).collect();
        assert!(ys.windows(2).any(|w| w[1] > w[0]), "a second row: {ys:?}");
        assert!(ys.iter().all(|y| *y + 8.0 <= 100.0 - 8.0 + 1e-6), "{ys:?}");
        let xs: Vec<f64> = (0..6).map(|i| swatch(&s, i).x1).collect();
        assert!(xs.iter().all(|x| *x <= 400.0), "{xs:?}");
        let path = vectorcraft_testkit::temp_dir("graph-top").join("top.vectorcraft");
        s.execute("graph.setType", &json!({"dropShadow": true})).unwrap();
        s.execute("document.save", &json!({"path": path})).unwrap();
        s.execute("document.open", &json!({"path": path})).unwrap();
        let g = spec(&s, id);
        assert!(g.legend_across_top && g.drop_shadow);
        let v = serde_json::to_value(GraphSpec::default()).unwrap();
        assert!(v.get("dropShadow").is_none() && v.get("legendAcrossTop").is_none());
    }

    #[test]
    fn each_layer_keeps_its_shadows_just_behind_it() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
        let id = make(&mut s, "column", ",a,b,c\nQ1,3,2,1\nQ2,5,,2\nQ3,4,6,3");
        s.execute("graph.setType", &json!({"seriesIndexes": [0], "type": "area"})).unwrap();
        s.execute("graph.setType", &json!({"seriesIndexes": [2], "type": "line", "dropShadow": true})).unwrap();
        let n = s.doc().unwrap().doc.node(id).unwrap().clone();
        let order: Vec<String> =
            n.children().unwrap().iter().map(|c| c.series_index.map_or_else(|| c.name.clone().unwrap_or_default(), |i| format!("s{i}"))).collect();
        // Areas cast none; the columns' shadows sit between the area and the columns, the line's between the columns
        // and the line.
        let at = |k: &str| order.iter().position(|o| o == k).unwrap();
        let shadows: Vec<usize> = order.iter().enumerate().filter(|(_, o)| *o == "Drop Shadow").map(|(i, _)| i).collect();
        assert_eq!(shadows.len(), 2, "{order:?}");
        assert!(at("s0") < shadows[0] && shadows[0] < at("s1") && at("s1") < shadows[1] && shadows[1] < at("s2"), "{order:?}");
        // b's blank leaves two columns; a line with a blank casts a shadow per run.
        let line = make(&mut s, "line", ",a\nQ1,1\nQ2,\nQ3,3\nQ4,4");
        s.execute("graph.setType", &json!({"dropShadow": true})).unwrap();
        let n = s.doc().unwrap().doc.node(line).unwrap().clone();
        assert_eq!(group(&n, "Drop Shadow").children().unwrap().len(), 1, "Q1 alone draws no line; Q3–Q4 one");
        // Scatter and radar lines cast one too; a stacked segment of 0 doesn't.
        for (ty, want) in [("scatter", 1), ("radar", 2)] {
            let g = graph(&mut s, ty);
            s.execute("graph.setType", &json!({"dropShadow": true})).unwrap();
            let n = s.doc().unwrap().doc.node(g).unwrap().clone();
            assert_eq!(group(&n, "Drop Shadow").children().unwrap().len(), want, "{ty}");
        }
        let st = make(&mut s, "stackedColumn", ",a,b\nQ1,0,2");
        s.execute("graph.setType", &json!({"dropShadow": true})).unwrap();
        let n = s.doc().unwrap().doc.node(st).unwrap().clone();
        assert_eq!(group(&n, "Drop Shadow").children().unwrap().len(), 1);
    }

    #[test]
    fn the_legend_across_top_keeps_above_labels_over_the_plot() {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
        let id = make(&mut s, "radar", ",a,b\nTop|of|chart,1,2\nQ2,2,3\nQ3,3,1");
        s.execute("graph.setType", &json!({"legendAcrossTop": true})).unwrap();
        let n = s.doc().unwrap().doc.node(id).unwrap().clone();
        let axes_top = group(&n, "Axes").children().unwrap().iter().filter_map(|c| c.geometric_bounds()).map(|b| b.y0).fold(f64::MAX, f64::min);
        let legend_bottom =
            group(&n, "Legend").children().unwrap().iter().filter_map(|c| c.geometric_bounds()).map(|b| b.y1).fold(f64::MIN, f64::max);
        assert!(legend_bottom < axes_top, "{legend_bottom} above {axes_top}");
        // No legend: nothing across the top either.
        s.execute("graph.setType", &json!({"legend": false})).unwrap();
        let n = s.doc().unwrap().doc.node(id).unwrap().clone();
        assert!(n.children().unwrap().iter().all(|c| c.name.as_deref() != Some("Legend")));
        // Labels of a row other than the last stay within the plot's width.
        let labels: Vec<String> = (0..6).map(|i| format!("A rather long series name {i}")).collect();
        let col = make(&mut s, "column", &format!(",{}\nQ1,1,2,3,4,5,6", labels.join(",")));
        s.execute("graph.setType", &json!({"legendAcrossTop": true})).unwrap();
        let n = s.doc().unwrap().doc.node(col).unwrap().clone();
        let texts: Vec<_> = group(&n, "Legend").children().unwrap().iter().filter_map(|c| c.geometric_bounds()).collect();
        let last_row = texts.iter().map(|b| b.y0).fold(f64::MIN, f64::max);
        assert!(texts.iter().filter(|b| b.y0 < last_row - 1e-6).all(|b| b.x1 <= 400.0 + 1e-6), "{texts:?}");
    }
}
