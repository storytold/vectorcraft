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
use vectorcraft_doc::{Appearance, CharStyle, Document, GraphKind, GraphSpec, Justify, Node, NodeId, NodeKind, TextObject};
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
            "{id?, series?, categories?, rows?, csv?: first row = series labels (first cell empty), then one row per category: label, values…; an empty cell or a null is a blank value, a quoted number a label} replace the graph's data; no data → the current {csv, series, categories, rows}",
            has_selection,
            set_data
        ),
        cmd!(
            "graph.setType",
            "Type…",
            ["Object", "Graph"],
            None,
            "{id?, type?, seriesIndexes?: [index], columnWidth?: %, clusterWidth?: %, legend?: bool, markPoints?: bool, connectPoints?: bool, edgeToEdge?: bool (line graphs: true runs the lines across the whole plot, false puts the points at the centres of their categories), ticks?: n, axisMin?, axisMax?} change the graph type and options; with `seriesIndexes`, or (no `id`) with only series selected with Group Selection, `type` goes to those series only (Combine different graph types: column, stacked column, line and area mix, and so do bar and stacked bar; a series given the graph's type follows the graph again) (axisMin and axisMax together override the calculated value axis: exactly that range in `ticks` divisions, 5 when 0); no options → the current ones",
            has_selection,
            set_type
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
    // Drop rows with neither a value nor a label; a label-only row stays, its values blank.
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
    changed
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

/// A "nice" axis: (min, max, step).
fn nice_axis(lo: f64, hi: f64, ticks: usize) -> (f64, f64, f64) {
    let (lo, hi) = if (hi - lo).abs() < 1e-12 { (lo.min(0.0), lo.max(0.0) + 1.0) } else { (lo, hi) };
    let n = if ticks == 0 { 5 } else { ticks.clamp(1, 100) } as f64;
    let raw = (hi - lo) / n;
    let mag = 10f64.powf(raw.log10().floor());
    let step = [1.0, 2.0, 2.5, 5.0, 10.0].iter().map(|m| m * mag).find(|s| *s >= raw - 1e-12).unwrap_or(10.0 * mag);
    ((lo / step).floor() * step, (hi / step).ceil() * step, step)
}

/// The value axis: with both `axis_min` and `axis_max` set (Graph Type › Tick Values › Override Calculated Values),
/// exactly that range split into `ticks` divisions (5 when automatic); otherwise a nice axis around the data, using
/// whichever bound was given.
fn value_axis(g: &GraphSpec, lo: f64, hi: f64) -> (f64, f64, f64) {
    match (g.axis_min, g.axis_max) {
        (Some(min), Some(max)) if max > min && (max - min).is_finite() => {
            let n = if g.ticks == 0 { 5 } else { g.ticks.clamp(1, 100) } as f64;
            (min, max, (max - min) / n)
        }
        _ => {
            // An override too wide for f64 (or NaN) gives a non-finite axis, which would reach `clamp` with NaN bounds
            // and panic: fall back to the data, then to 0..1.
            let finite = |(a, b, s): (f64, f64, f64)| a.is_finite() && b.is_finite() && s.is_finite() && b > a && s > 0.0;
            [nice_axis(g.axis_min.unwrap_or(lo), g.axis_max.unwrap_or(hi), g.ticks), nice_axis(lo, hi, g.ticks)]
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
    fn group(&mut self, name: &str, children: Vec<Arc<Node>>) -> Arc<Node> {
        let mut g = Node::group(self.d.alloc_id(), children);
        g.name = Some(name.into());
        Arc::new(g)
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

fn marker(p: Point, s: f64) -> PathData {
    shapes::rectangle(Rect::from_center_size(p, (s, s)))
}

/// Build the graph's children.
fn generate(d: &mut Document, g: &GraphSpec) -> Vec<Arc<Node>> {
    let mark = |i: usize, series_fill: bool, stroke: Paint, width: f64| series_marks(g, i, series_fill, stroke, width);
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

    match g.kind {
        GraphKind::Pie => {
            // One pie per row, side by side; a wedge per series, clockwise from 12 o'clock.
            let w = r.width() / ncat as f64;
            let rad = (w.min(r.height()) / 2.0 * 0.9).max(1.0);
            for c in 0..ncat {
                let centre = Point::new(r.x0 + w * (c as f64 + 0.5), r.y0 + r.height() / 2.0);
                let total: f64 = (0..nser).map(|s| val(c, s).max(0.0)).sum();
                let mut a0 = -std::f64::consts::FRAC_PI_2;
                for (s, items) in series.iter_mut().enumerate() {
                    let v = val(c, s).max(0.0);
                    if total <= 0.0 || v <= 0.0 {
                        continue;
                    }
                    let sweep = v / total * std::f64::consts::TAU;
                    let mut bp = BezPath::new();
                    bp.move_to(centre);
                    let arc = vectorcraft_geom::kurbo::Arc::new(centre, (rad, rad), a0, sweep, 0.0);
                    bp.line_to(centre + vectorcraft_geom::Vec2::new(a0.cos(), a0.sin()) * rad);
                    arc.to_cubic_beziers(0.1, |p1, p2, p| bp.curve_to(p1, p2, p));
                    bp.close_path();
                    let (fill, stroke, w) = mark(s, true, Paint::solid(Color::WHITE), 0.5);
                    let n = b.path(PathData::from_bezpath(&bp), fill, stroke, w);
                    items.push(n);
                    a0 += sweep;
                }
                if let Some(cat) = g.categories.get(c).filter(|c| !c.is_empty()) {
                    let t = b.text(Point::new(centre.x, centre.y + rad + LABEL_SIZE * 1.6), cat, Justify::Center);
                    b.out.push(t);
                }
            }
        }
        GraphKind::Radar => {
            let centre = r.center();
            let rad = (r.width().min(r.height()) / 2.0).max(1.0);
            let (lo, hi, step) = value_axis(g, 0.0, g.rows.iter().flatten().copied().fold(0.0, f64::max));
            let at = |c: usize, v: f64| {
                let a = -std::f64::consts::FRAC_PI_2 + std::f64::consts::TAU * c as f64 / ncat as f64;
                centre + vectorcraft_geom::Vec2::new(a.cos(), a.sin()) * (rad * ((v - lo) / (hi - lo)).clamp(0.0, 1.0))
            };
            let mut axes = vec![];
            for c in 0..ncat {
                axes.push(b.line(centre, at(c, hi)));
                if let Some(cat) = g.categories.get(c).filter(|c| !c.is_empty()) {
                    let p = at(c, hi) + (at(c, hi) - centre).normalize() * (LABEL_SIZE * 1.2);
                    axes.push(b.text(Point::new(p.x, p.y + LABEL_SIZE * 0.35), cat, Justify::Center));
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
                    items.push(b.path(polyline(&pts, true), fill, stroke, w));
                }
                if g.mark_points {
                    for p in pts {
                        let (fill, stroke, w) = mark(s, true, Paint::None, 0.0);
                        items.push(b.path(marker(p, 4.0), fill, stroke, w));
                    }
                }
            }
        }
        _ => {
            // Value range (stacked graphs stack positives and negatives separately).
            let (mut lo, mut hi) = (0.0f64, 0.0f64);
            let scatter = g.kind == GraphKind::Scatter;
            let (mut xlo, mut xhi) = (f64::MAX, f64::MIN);
            for c in 0..ncat {
                if !scatter {
                    // Stacked columns (or bars) stack together, and so do areas; other series count one value each.
                    for stack in [[GraphKind::StackedColumn, GraphKind::StackedBar], [GraphKind::Area, GraphKind::Area]] {
                        let members = || (0..nser).filter(|s| stack.contains(&kind(*s)));
                        hi = hi.max(members().map(|s| val(c, s).max(0.0)).sum());
                        lo = lo.min(members().map(|s| val(c, s).min(0.0)).sum());
                    }
                    for v in (0..nser).filter(|s| matches!(kind(*s), GraphKind::Column | GraphKind::Bar | GraphKind::Line)).filter_map(|s| cell(c, s))
                    {
                        hi = hi.max(v);
                        lo = lo.min(v);
                    }
                } else {
                    for s in (0..nser).step_by(2) {
                        if let (Some(y), Some(x)) = (cell(c, s), cell(c, s + 1)) {
                            hi = hi.max(y);
                            lo = lo.min(y);
                            xlo = xlo.min(x);
                            xhi = xhi.max(x);
                        }
                    }
                }
            }
            let (lo, hi, step) = value_axis(g, lo, hi);
            // Value → coordinate along the value axis (y for columns/lines, x for bars).
            let vpos = |v: f64| {
                let t = (v - lo) / (hi - lo);
                if horizontal { r.x0 + t * r.width() } else { r.y1 - t * r.height() }
            };
            let mut axes = vec![];
            // Value axis with ticks and labels.
            for v in tick_values(lo, hi, step) {
                let q = vpos(v);
                if horizontal {
                    axes.push(b.line(Point::new(q, r.y1), Point::new(q, r.y1 + 4.0)));
                    axes.push(b.text(Point::new(q, r.y1 + 4.0 + LABEL_SIZE * 1.1), &fmt_value(v), Justify::Center));
                } else {
                    axes.push(b.line(Point::new(r.x0 - 4.0, q), Point::new(r.x0, q)));
                    axes.push(b.text(Point::new(r.x0 - 6.0, q + LABEL_SIZE * 0.35), &fmt_value(v), Justify::Right));
                }
            }
            if horizontal {
                axes.push(b.line(Point::new(r.x0, r.y1), Point::new(r.x1, r.y1)));
                axes.push(b.line(Point::new(vpos(0.0f64.clamp(lo, hi)), r.y0), Point::new(vpos(0.0f64.clamp(lo, hi)), r.y1)));
            } else {
                axes.push(b.line(Point::new(r.x0, r.y0), Point::new(r.x0, r.y1)));
                axes.push(b.line(Point::new(r.x0, vpos(0.0f64.clamp(lo, hi))), Point::new(r.x1, vpos(0.0f64.clamp(lo, hi)))));
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
                            b.text(Point::new(r.x0 - 6.0, cat_mid(c) + LABEL_SIZE * 0.35), cat, Justify::Right)
                        } else {
                            b.text(Point::new(cat_mid(c), r.y1 + LABEL_SIZE * 1.4), cat, Justify::Center)
                        };
                        axes.push(t);
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
                            (Some(y), Some(x)) => Some(Point::new(r.x0 + (x - xl) / (xh - xl) * r.width(), vpos(y))),
                            _ => None,
                        })
                        .collect();
                    if g.connect_points {
                        for run in runs(&pts) {
                            let (fill, stroke, w) = mark(si, false, default_series_paint(si), 1.0);
                            series[si].push(b.path(polyline(&run, false), fill, stroke, w));
                        }
                    }
                    if g.mark_points {
                        for p in pts.iter().flatten() {
                            let (fill, stroke, w) = mark(si, true, Paint::None, 0.0);
                            series[si].push(b.path(marker(*p, 5.0), fill, stroke, w));
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
            let slots = columns.len() + usize::from(!stacks.is_empty());
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
                    let n = b.path(shapes::rectangle(bar_rect(a, bar, vpos(0.0f64.clamp(lo, hi)), vpos(v))), fill, stroke, w);
                    if let Some(items) = series.get_mut(s) {
                        items.push(n);
                    }
                }
                let (a, w) = if columns.is_empty() {
                    let w = (cat_w * (g.column_width / 100.0).clamp(0.01, 1.0) * (g.cluster_width / 100.0).clamp(0.01, 1.0) * 1.2).min(cat_w);
                    (cat_start(c) + (cat_w - w) / 2.0, w)
                } else {
                    (c0 + slot * columns.len() as f64 + (slot - bar) / 2.0, bar)
                };
                let (mut pos, mut neg) = (0.0, 0.0);
                for &s in &stacks {
                    let Some(v) = cell(c, s) else { continue };
                    let base = if v >= 0.0 { &mut pos } else { &mut neg };
                    let (v0, v1) = (vpos(*base), vpos(*base + v));
                    *base += v;
                    let (fill, stroke, sw) = mark(s, true, Paint::solid(Color::WHITE), 0.25);
                    let n = b.path(shapes::rectangle(bar_rect(a, w, v0, v1)), fill, stroke, sw);
                    if let Some(items) = series.get_mut(s) {
                        items.push(n);
                    }
                }
            }
            // Cumulative area bands, each from the previous area total up to its own.
            let mut below: Vec<f64> = vec![0.0; ncat];
            for s in (0..nser).filter(|s| kind(*s) == GraphKind::Area) {
                let above: Vec<f64> = below.iter().enumerate().map(|(c, b)| b + val(c, s)).collect();
                // Mixed with series at the category centres, areas take their points there too.
                let at = |c: usize, v: &[f64]| Point::new(mid(c, label_edges), vpos(v.get(c).copied().unwrap_or(0.0)));
                let mut pts: Vec<Point> = (0..ncat).map(|c| at(c, &above)).collect();
                pts.extend((0..ncat).rev().map(|c| at(c, &below)));
                let (fill, stroke, w) = mark(s, true, Paint::solid(Color::WHITE), 0.25);
                let n = b.path(polyline(&pts, true), fill, stroke, w);
                if let Some(items) = series.get_mut(s) {
                    items.push(n);
                }
                below = above;
            }
            for s in (0..nser).filter(|s| kind(*s) == GraphKind::Line) {
                let pts: Vec<Option<Point>> =
                    (0..ncat).map(|c| cell(c, s).map(|v| Point::new(mid(c, g.edge_to_edge && label_edges), vpos(v)))).collect();
                if g.connect_points {
                    for run in runs(&pts) {
                        let (fill, stroke, w) = mark(s, false, default_series_paint(s), 1.0);
                        let n = b.path(polyline(&run, false), fill, stroke, w);
                        if let Some(items) = series.get_mut(s) {
                            items.push(n);
                        }
                    }
                }
                if g.mark_points {
                    for p in pts.iter().flatten() {
                        let (fill, stroke, w) = mark(s, true, Paint::None, 0.0);
                        let n = b.path(marker(*p, 5.0), fill, stroke, w);
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
    if g.legend && !g.series.is_empty() {
        let x = r.x1 + 14.0;
        for (s, items) in series.iter_mut().enumerate().take(legend_series) {
            let label = if g.kind == GraphKind::Scatter { series_label(s * 2) } else { series_label(s) };
            if label.is_empty() {
                continue;
            }
            let y = r.y0 + s as f64 * (LABEL_SIZE * 1.8);
            let (fill, stroke, w) = mark(s, true, Paint::None, 0.0);
            items.push(b.path(shapes::rectangle(Rect::new(x, y, x + 8.0, y + 8.0)), fill, stroke, w));
            labels.push(b.text(Point::new(x + 12.0, y + 7.5), &label, Justify::Left));
        }
    }
    // Areas at the back, lines and points in front of columns; one graph type keeps the series order.
    let depth = |s: usize| match kind(s) {
        GraphKind::Area => 0,
        GraphKind::Line => 2,
        _ => 1,
    };
    let mut series: Vec<(usize, Vec<Arc<Node>>)> = series.into_iter().enumerate().collect();
    series.sort_by_key(|(s, _)| depth(*s));
    for (s, items) in series {
        if items.is_empty() {
            continue;
        }
        let name = g.series.get(s).cloned().filter(|n| !n.is_empty()).unwrap_or_else(|| format!("Series {}", s + 1));
        let grp = b.series(&name, s, items);
        b.out.push(grp);
    }
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
    let parent = s.doc()?.insertion_parent();
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
        let count = spec.rows.iter().map(Vec::len).max().unwrap_or(0).max(1);
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
        match members.get(id).filter(|r| r.group == *id).and_then(|_| doc.node(*id)) {
            Some(group) => group.walk(&mut |m| {
                if matches!(m.kind, NodeKind::Path { .. }) {
                    out.push(m.id);
                }
            }),
            None => out.push(*id),
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
            let r = members.get(id)?;
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

fn set_data(s: &mut Session, p: &Value) -> Result<Value> {
    let id = target(s, p, "graph.setData")?;
    let mut spec = spec_of(s, id)?;
    if !apply_data(&mut spec, p) {
        // Blank cells come back as null, so the rows can be edited and sent back as they are.
        return Ok(json!({ "csv": to_csv(&spec), "series": spec.series, "categories": spec.categories, "rows": spec.cells() }));
    }
    s.edit("Graph Data", |d, _| regenerate(d, id, spec))?;
    Ok(json!({ "id": id.0 }))
}

fn set_type(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "graph.setType";
    let id = target(s, p, C)?;
    let mut spec = spec_of(s, id)?;
    // The series to retype: `seriesIndexes`, else, with no `id`, a selection made only of this graph's series (Group
    // Selection). Anything else selected, the graph itself for one, is the whole graph.
    let count = spec.rows.iter().map(Vec::len).max().unwrap_or(0).min(vectorcraft_doc::MAX_GRAPH_SERIES);
    let picked: Vec<usize> = match p.get("seriesIndexes") {
        Some(v) => {
            let a = v.as_array().filter(|a| !a.is_empty()).ok_or_else(|| bad(C, "`seriesIndexes` is a non-empty list of series indexes"))?;
            if p.get("type").is_none() {
                return Err(bad(C, "`seriesIndexes` needs a `type`"));
            }
            a.iter()
                .map(|i| i.as_u64().and_then(|i| usize::try_from(i).ok()).filter(|i| *i < count).ok_or_else(|| bad(C, format!("no series {i}"))))
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
    };
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
    ];
    if !keys.iter().any(|k| p.get(*k).is_some()) {
        // With series picked, the type they share, so the dialog's OK keeps it.
        let kinds: Vec<GraphKind> = picked.iter().map(|i| spec.series_kind(*i)).collect();
        let kind = kinds.first().copied().filter(|k| kinds.iter().all(|x| x == k)).unwrap_or(spec.kind);
        return Ok(json!({
            "type": kind.id(), "columnWidth": spec.column_width, "clusterWidth": spec.cluster_width, "legend": spec.legend,
            "markPoints": spec.mark_points, "connectPoints": spec.connect_points, "edgeToEdge": spec.edge_to_edge, "ticks": spec.ticks,
            "axisMin": spec.axis_min, "axisMax": spec.axis_max,
        }));
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
            for i in picked {
                if let Some(k) = spec.series_kinds.get_mut(i) {
                    *k = (kind != spec.kind).then_some(kind);
                }
            }
            while spec.series_kinds.last().is_some_and(Option::is_none) {
                spec.series_kinds.pop();
            }
        }
    }
    spec.column_width = f64_or(p, "columnWidth", spec.column_width).clamp(1.0, 1000.0);
    spec.cluster_width = f64_or(p, "clusterWidth", spec.cluster_width).clamp(1.0, 100.0);
    spec.legend = bool_or(p, "legend", spec.legend);
    spec.mark_points = bool_or(p, "markPoints", spec.mark_points);
    spec.connect_points = bool_or(p, "connectPoints", spec.connect_points);
    spec.edge_to_edge = bool_or(p, "edgeToEdge", spec.edge_to_edge);
    spec.ticks = p.get("ticks").and_then(Value::as_u64).map_or(spec.ticks, |t| t.min(100) as usize);
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
            let groups: Vec<NodeId> = d
                .node(id)
                .and_then(Node::children)
                .into_iter()
                .flatten()
                .filter(|c| c.series_index.is_some_and(|i| reselect.contains(&i)))
                .map(|c| c.id)
                .collect();
            sel.set(groups);
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
}
