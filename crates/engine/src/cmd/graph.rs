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
            "{type: column|stackedColumn|bar|stackedBar|line|area|scatter|pie|radar, x, y, width, height, series?: [..], categories?: [..], rows?: [[..]], csv?} → {id}",
            has_doc,
            create
        ),
        cmd!(
            "graph.setData",
            "Data…",
            ["Object", "Graph"],
            None,
            "{id?, series?, categories?, rows?, csv?: first row = series labels (first cell empty), then one row per category: label, values…} replace the graph's data; no data → the current {csv, series, categories, rows}",
            has_selection,
            set_data
        ),
        cmd!(
            "graph.setType",
            "Type…",
            ["Object", "Graph"],
            None,
            "{id?, type?, columnWidth?: %, clusterWidth?: %, legend?: bool, markPoints?: bool, connectPoints?: bool, ticks?: n, axisMin?, axisMax?} change the graph type and options; no options → the current ones",
            has_selection,
            set_type
        ),
    ]
}

// ---------- data ----------

fn parse_csv(csv: &str) -> (Vec<String>, Vec<String>, Vec<Vec<f64>>) {
    let rows: Vec<Vec<String>> = csv
        .lines()
        .map(|l| l.trim_end_matches('\r'))
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            let sep = if l.contains('\t') { '\t' } else { ',' };
            l.split(sep).map(|c| c.trim().trim_matches('"').to_string()).collect()
        })
        .collect();
    let Some(first) = rows.first() else { return (vec![], vec![], vec![]) };
    // A header row has a non-numeric cell after the first one (or an empty first cell).
    let header = first.first().is_some_and(|c| c.is_empty()) || first.iter().skip(1).any(|c| c.parse::<f64>().is_err());
    let series = if header { first.iter().skip(1).cloned().collect() } else { vec![] };
    let mut categories = vec![];
    let mut values = vec![];
    for r in rows.iter().skip(header as usize) {
        let labelled = r.first().is_some_and(|c| c.parse::<f64>().is_err());
        categories.push(if labelled { r[0].clone() } else { String::new() });
        values.push(r.iter().skip(labelled as usize).map(|c| c.parse::<f64>().unwrap_or(0.0)).collect());
    }
    if categories.iter().all(String::is_empty) {
        categories.clear();
    }
    (series, categories, values)
}

fn to_csv(g: &GraphSpec) -> String {
    let mut out = String::new();
    if !g.series.is_empty() {
        out.push(',');
        out.push_str(&g.series.join(","));
        out.push('\n');
    }
    for (i, r) in g.rows.iter().enumerate() {
        if let Some(c) = g.categories.get(i) {
            out.push_str(c);
            out.push(',');
        }
        out.push_str(&r.iter().map(|v| fmt_value(*v)).collect::<Vec<_>>().join(","));
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
        g.rows = r;
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
        g.rows = rows.iter().map(|r| r.as_array().map(|a| a.iter().map(|v| v.as_f64().unwrap_or(0.0)).collect()).unwrap_or_default()).collect();
        changed = true;
    }
    g.rows.retain(|r| !r.is_empty());
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
    let val = |c: usize, s: usize| g.rows.get(c).and_then(|r| r.get(s)).copied().unwrap_or(0.0);
    let stacked = matches!(g.kind, GraphKind::StackedColumn | GraphKind::StackedBar | GraphKind::Area);
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
            let hi = g.axis_max.unwrap_or_else(|| g.rows.iter().flatten().copied().fold(0.0, f64::max));
            let (lo, hi, step) = nice_axis(g.axis_min.unwrap_or(0.0), hi, g.ticks);
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
            let mut v = lo + step;
            while v <= hi + step * 1e-6 {
                let ring: Vec<Point> = (0..ncat).map(|c| at(c, v)).collect();
                axes.push(b.path(polyline(&ring, true), Paint::None, Paint::solid(Color::rgb(0.6, 0.6, 0.6)), 0.25));
                v += step;
            }
            let ax = b.group("Axes", axes);
            b.out.push(ax);
            for (s, items) in series.iter_mut().enumerate() {
                let pts: Vec<Point> = (0..ncat).map(|c| at(c, val(c, s))).collect();
                let (fill, stroke, w) = mark(s, false, default_series_paint(s), 1.0);
                items.push(b.path(polyline(&pts, true), fill, stroke, w));
                for p in pts {
                    let (fill, stroke, w) = mark(s, true, Paint::None, 0.0);
                    items.push(b.path(marker(p, 4.0), fill, stroke, w));
                }
            }
        }
        _ => {
            // Value range (stacked graphs stack positives and negatives separately).
            let (mut lo, mut hi) = (0.0f64, 0.0f64);
            let scatter = g.kind == GraphKind::Scatter;
            let (mut xlo, mut xhi) = (f64::MAX, f64::MIN);
            for c in 0..ncat {
                if stacked {
                    let pos: f64 = (0..nser).map(|s| val(c, s).max(0.0)).sum();
                    let neg: f64 = (0..nser).map(|s| val(c, s).min(0.0)).sum();
                    hi = hi.max(pos);
                    lo = lo.min(neg);
                } else if scatter {
                    for s in (0..nser).step_by(2) {
                        hi = hi.max(val(c, s));
                        lo = lo.min(val(c, s));
                        xlo = xlo.min(val(c, s + 1));
                        xhi = xhi.max(val(c, s + 1));
                    }
                } else {
                    for s in 0..nser {
                        hi = hi.max(val(c, s));
                        lo = lo.min(val(c, s));
                    }
                }
            }
            let (lo, hi, step) = nice_axis(g.axis_min.unwrap_or(lo), g.axis_max.unwrap_or(hi), g.ticks);
            // Value → coordinate along the value axis (y for columns/lines, x for bars).
            let vpos = |v: f64| {
                let t = (v - lo) / (hi - lo);
                if horizontal { r.x0 + t * r.width() } else { r.y1 - t * r.height() }
            };
            let mut axes = vec![];
            // Value axis with ticks and labels.
            let mut v = lo;
            while v <= hi + step * 1e-6 {
                let q = vpos(v);
                if horizontal {
                    axes.push(b.line(Point::new(q, r.y1), Point::new(q, r.y1 + 4.0)));
                    axes.push(b.text(Point::new(q, r.y1 + 4.0 + LABEL_SIZE * 1.1), &fmt_value(v), Justify::Center));
                } else {
                    axes.push(b.line(Point::new(r.x0 - 4.0, q), Point::new(r.x0, q)));
                    axes.push(b.text(Point::new(r.x0 - 6.0, q + LABEL_SIZE * 0.35), &fmt_value(v), Justify::Right));
                }
                v += step;
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
            let points_mode = matches!(g.kind, GraphKind::Line | GraphKind::Area);
            let cat_mid = |c: usize| {
                if points_mode && ncat > 1 { r.x0 + r.width() * c as f64 / (ncat - 1) as f64 } else { cat_start(c) + cat_w / 2.0 }
            };
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
                    let pts: Vec<Point> =
                        (0..ncat).map(|c| Point::new(r.x0 + (val(c, s + 1) - xl) / (xh - xl) * r.width(), vpos(val(c, s)))).collect();
                    if g.connect_points && pts.len() > 1 {
                        let (fill, stroke, w) = mark(si, false, default_series_paint(si), 1.0);
                        series[si].push(b.path(polyline(&pts, false), fill, stroke, w));
                    }
                    if g.mark_points {
                        for p in &pts {
                            let (fill, stroke, w) = mark(si, true, Paint::None, 0.0);
                            series[si].push(b.path(marker(*p, 5.0), fill, stroke, w));
                        }
                    }
                }
            }
            let ax = b.group("Axes", axes);
            b.out.push(ax);
            match g.kind {
                GraphKind::Column | GraphKind::Bar => {
                    let cluster = cat_w * (g.cluster_width / 100.0).clamp(0.01, 1.0);
                    let slot = cluster / nser as f64;
                    let bar = slot * (g.column_width / 100.0).clamp(0.01, 1.0);
                    for c in 0..ncat {
                        let c0 = cat_start(c) + (cat_w - cluster) / 2.0;
                        for (s, items) in series.iter_mut().enumerate() {
                            let a = c0 + slot * s as f64 + (slot - bar) / 2.0;
                            let (v0, v1) = (vpos(0.0f64.clamp(lo, hi)), vpos(val(c, s)));
                            let rect = if horizontal {
                                Rect::new(v0.min(v1), a, v0.max(v1), a + bar)
                            } else {
                                Rect::new(a, v0.min(v1), a + bar, v0.max(v1))
                            };
                            let (fill, stroke, w) = mark(s, true, Paint::None, 0.0);
                            items.push(b.path(shapes::rectangle(rect), fill, stroke, w));
                        }
                    }
                }
                GraphKind::StackedColumn | GraphKind::StackedBar => {
                    let bar = cat_w * (g.column_width / 100.0).clamp(0.01, 1.0) * (g.cluster_width / 100.0).clamp(0.01, 1.0) * 1.2;
                    let bar = bar.min(cat_w);
                    for c in 0..ncat {
                        let a = cat_start(c) + (cat_w - bar) / 2.0;
                        let (mut pos, mut neg) = (0.0, 0.0);
                        for (s, items) in series.iter_mut().enumerate() {
                            let v = val(c, s);
                            let base = if v >= 0.0 { &mut pos } else { &mut neg };
                            let (v0, v1) = (vpos(*base), vpos(*base + v));
                            *base += v;
                            let rect = if horizontal {
                                Rect::new(v0.min(v1), a, v0.max(v1), a + bar)
                            } else {
                                Rect::new(a, v0.min(v1), a + bar, v0.max(v1))
                            };
                            let (fill, stroke, w) = mark(s, true, Paint::solid(Color::WHITE), 0.25);
                            items.push(b.path(shapes::rectangle(rect), fill, stroke, w));
                        }
                    }
                }
                GraphKind::Line => {
                    for (s, items) in series.iter_mut().enumerate() {
                        let pts: Vec<Point> = (0..ncat).map(|c| Point::new(cat_mid(c), vpos(val(c, s)))).collect();
                        if g.connect_points && pts.len() > 1 {
                            let (fill, stroke, w) = mark(s, false, default_series_paint(s), 1.0);
                            items.push(b.path(polyline(&pts, false), fill, stroke, w));
                        }
                        if g.mark_points {
                            for p in &pts {
                                let (fill, stroke, w) = mark(s, true, Paint::None, 0.0);
                                items.push(b.path(marker(*p, 5.0), fill, stroke, w));
                            }
                        }
                    }
                }
                GraphKind::Area => {
                    // Cumulative bands, each from the previous total up to its own.
                    let mut below: Vec<f64> = vec![0.0; ncat];
                    for (s, items) in series.iter_mut().enumerate() {
                        let above: Vec<f64> = (0..ncat).map(|c| below[c] + val(c, s)).collect();
                        let mut pts: Vec<Point> = (0..ncat).map(|c| Point::new(cat_mid(c), vpos(above[c]))).collect();
                        pts.extend((0..ncat).rev().map(|c| Point::new(cat_mid(c), vpos(below[c]))));
                        let (fill, stroke, w) = mark(s, true, Paint::solid(Color::WHITE), 0.25);
                        items.push(b.path(polyline(&pts, true), fill, stroke, w));
                        below = above;
                    }
                }
                _ => {}
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
    for (s, items) in series.into_iter().enumerate() {
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
        return Ok(json!({ "csv": to_csv(&spec), "series": spec.series, "categories": spec.categories, "rows": spec.rows }));
    }
    s.edit("Graph Data", |d, _| regenerate(d, id, spec))?;
    Ok(json!({ "id": id.0 }))
}

fn set_type(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "graph.setType";
    let id = target(s, p, C)?;
    let mut spec = spec_of(s, id)?;
    let keys = ["type", "columnWidth", "clusterWidth", "legend", "markPoints", "connectPoints", "ticks", "axisMin", "axisMax"];
    if !keys.iter().any(|k| p.get(*k).is_some()) {
        return Ok(json!({
            "type": spec.kind.id(), "columnWidth": spec.column_width, "clusterWidth": spec.cluster_width, "legend": spec.legend,
            "markPoints": spec.mark_points, "connectPoints": spec.connect_points, "ticks": spec.ticks,
        }));
    }
    if let Some(t) = str_param(p, "type") {
        spec.kind = GraphKind::parse(t).ok_or_else(|| bad(C, format!("unknown graph type `{t}`")))?;
    }
    spec.column_width = f64_or(p, "columnWidth", spec.column_width).clamp(1.0, 1000.0);
    spec.cluster_width = f64_or(p, "clusterWidth", spec.cluster_width).clamp(1.0, 100.0);
    spec.legend = bool_or(p, "legend", spec.legend);
    spec.mark_points = bool_or(p, "markPoints", spec.mark_points);
    spec.connect_points = bool_or(p, "connectPoints", spec.connect_points);
    spec.ticks = p.get("ticks").and_then(Value::as_u64).map_or(spec.ticks, |t| t.min(100) as usize);
    if let Some(v) = p.get("axisMin") {
        spec.axis_min = v.as_f64();
    }
    if let Some(v) = p.get("axisMax") {
        spec.axis_max = v.as_f64();
    }
    let label = format!("{} Graph", spec.kind.label());
    s.edit("Graph Type", |d, _| {
        regenerate(d, id, spec)?;
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
        assert_eq!(r, vec![vec![1.0, 2.0], vec![3.0, 4.0]]);
        let (s, c, r) = super::parse_csv("1,2\n3,4");
        assert!(s.is_empty() && c.is_empty());
        assert_eq!(r.len(), 2);
        assert_eq!(super::nice_axis(0.0, 7.0, 0), (0.0, 8.0, 2.0));
    }
}
