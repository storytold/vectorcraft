//! A live graph's records, which the legacy `.ai` writer ([`crate::legacy_ai`]) and the editing data
//! writer ([`crate::ai_editing`]) both write around a graph's group (Adobe Illustrator File Format
//! Specification, version 7.0, §17 and appendix A): `%_Gs`, the functional records on `%_` lines
//! (`Gb`, `Gy`, `Gd`, three `Ga` … `GA` axes, `Gz`, `Gc` cells and `GC`), the drawn art in one group
//! with each series' group followed by `%_2 column 0 0 Go`, then `%_GS`.
//!
//! A graph is written so only when the `.ai` reader (`import::ai::read::graph`) reads every record
//! back as the same graph: one of the four graph types it reads (grouped and stacked column, line,
//! area), the settings it keeps, a table it holds, and drawn art it attaches the graph to (each
//! series' group a direct member of the graph's, of paths painted with one solid fill and one plain
//! line, and the plot's left and bottom edges drawn as stroked lines). Any other graph is written as
//! its art, with a warning.

use std::collections::HashMap;
use std::fmt::Write;

use kurbo::{PathSeg, Point};
use vectorcraft_color::{BlendMode, Paint};
use vectorcraft_doc::{
    Appearance, AppearanceItem, GraphKind, GraphSpec, LabelStyles, MAX_GRAPH_CATEGORIES, MAX_GRAPH_SERIES, Node, NodeId, NodeKind, PiePosition,
    PieSort, TickLength, ValueAxisSide,
};
use vectorcraft_effects::stroke;
use vectorcraft_geom::{Affine, Rect};

use crate::ps;

/// Most cells of a graph's table, its label row and column included.
const MAX_CELLS: usize = 2_600_000;

/// Longest cell string, in bytes (appendix A, `Gc`).
const MAX_CELL_TEXT: usize = 255;

/// Most characters of the text before or after a value axis' numbers (appendix A, `Ga`, `GA`).
const MAX_AFFIX: usize = 9;

/// Most tick marks per division the reader keeps.
const MAX_TICK_MARKS: usize = 20;

/// Most divisions of a value axis with set values the reader keeps.
const MAX_DIVISIONS: usize = 100;

/// How far an axis line may be from the plot's edge, in points.
const EDGE: f64 = 0.01;

/// Most values one `Gc` takes.
const CELLS_PER_RECORD: usize = 15;

/// A graph's records: what goes before its group (`%_Gs` to `%_GC`), and the `Go` line that follows
/// each series' group, by the group's id. `%_GS` follows the group.
pub(crate) struct Records {
    pub(crate) head: String,
    pub(crate) tags: HashMap<NodeId, String>,
}

/// The line that ends a graph's records, after its group.
pub(crate) const END: &str = "%_GS";

/// The records of graph `n` (a group holding [`vectorcraft_doc::NodeExtras::graph`]), its art written
/// through `to_art`; `None` when the reader would not read them back as the same graph.
pub(crate) fn records(n: &Node, to_art: Affine) -> Option<Records> {
    let g = n.extras().graph.as_deref()?;
    let NodeKind::Group { children, clip: false } = &n.kind else { return None };
    (n.visible && n.extras().mask.is_none() && all_shown(n)).then_some(())?;
    let ty = kind_code(g.kind)?;
    settings_kept(g).then_some(())?;
    let (left_manual, left_min, left_max, left_step) = axis(g.ticks, g.axis_min, g.axis_max)?;
    let (right_manual, right_min, right_max, right_step) = axis(g.right_ticks, g.right_axis_min, g.right_axis_max)?;
    // The table as the graph draws it: `cells` leaves out rows and series past the model's limits.
    let rows = g.rows.len();
    let width = g.rows.iter().map(Vec::len).max().unwrap_or(0);
    ((1..=MAX_GRAPH_CATEGORIES).contains(&rows) && (1..=MAX_GRAPH_SERIES).contains(&width)).then_some(())?;
    let cells = g.cells();
    cells.iter().flatten().all(|v| v.is_none_or(f64::is_finite)).then_some(())?;
    cells.iter().flatten().any(Option::is_some).then_some(())?;
    let first_row = usize::from(!g.series.is_empty());
    let first_col = usize::from(!g.categories.is_empty());
    (g.series.is_empty() || g.series.len() == width).then_some(())?;
    (g.categories.is_empty() || g.categories.len() == rows).then_some(())?;
    let (total_rows, total_cols) = (rows + first_row, width + first_col);
    total_rows.checked_mul(total_cols).filter(|c| *c <= MAX_CELLS)?;
    g.series.iter().chain(&g.categories).all(|l| l.len() <= MAX_CELL_TEXT && !l.chars().any(char::is_control)).then_some(())?;
    [&g.prefix, &g.suffix, &g.right_prefix, &g.right_suffix]
        .iter()
        .all(|a| a.chars().count() <= MAX_AFFIX && !a.chars().any(char::is_control))
        .then_some(())?;
    // The plot, and the art the reader attaches the graph to.
    let r = g.rect;
    (r.is_finite() && r.width() > 0.0 && r.height() > 0.0).then_some(())?;
    let plot = to_art.transform_rect_bbox(r);
    (plot.is_finite() && plot.width() > 0.0 && plot.height() > 0.0).then_some(())?;
    let tags = series_groups(children, width, first_col, g.kind == GraphKind::Line)?;
    draws_axes(n, r).then_some(())?;
    let placed = n.geometric_bounds()?;
    (r.x0 >= placed.x0 - EDGE && r.y0 >= placed.y0 - EDGE && r.x1 <= placed.x1 + EDGE && r.y1 <= placed.y1 + EDGE).then_some(())?;

    let mut head = String::from("%_Gs\n");
    let _ = writeln!(head, "%_{} {} {} {} Gb", real(plot.x0), real(plot.y1), real(plot.x1), real(plot.y0));
    let flag = |b: bool| u8::from(b);
    let _ = writeln!(
        head,
        "%_{ty} {} {} 14 {} {} 0 {} 6 44 Gy",
        flag(g.drop_shadow),
        flag(g.first_row_in_front),
        flag(g.mark_points),
        flag(g.connect_points),
        flag(g.legend_across_top)
    );
    let _ =
        writeln!(head, "%_10 2 {} 0 {} {} {} Gd", flag(g.first_column_in_front), real(g.column_width), real(g.cluster_width), flag(g.edge_to_edge));
    // Tick marks per division: 0 and 1 both mean one, at the labels, and the reader reads 0 as 1.
    let (category_marks, left_marks, right_marks) = (g.category_tick_marks.max(1), g.tick_marks.max(1), g.right_tick_marks.max(1));
    let _ = writeln!(head, "%_1 () Ga\n%_0 {} 0 0 0 {category_marks} {} () GA", tick(g.category_tick_length), flag(g.ticks_between_labels));
    let _ = writeln!(head, "%_2 {} Ga", ps::bytes(g.prefix.as_bytes()));
    let _ = writeln!(
        head,
        "%_{} {} {} {} {} {} 0 {} GA",
        flag(left_manual),
        tick(g.tick_length),
        real(left_min),
        real(left_max),
        real(left_step),
        left_marks,
        ps::bytes(g.suffix.as_bytes())
    );
    let _ = writeln!(head, "%_4 {} Ga", ps::bytes(g.right_prefix.as_bytes()));
    let _ = writeln!(
        head,
        "%_{} {} {} {} {} {} 0 {} GA",
        flag(right_manual),
        tick(g.right_tick_length),
        real(right_min),
        real(right_max),
        real(right_step),
        right_marks,
        ps::bytes(g.right_suffix.as_bytes())
    );
    let _ = writeln!(head, "%_{total_rows} {total_cols} {first_row} {first_col} Gz");
    let mut table = Table::new(&mut head);
    if first_row == 1 {
        if first_col == 1 {
            table.text("");
        }
        for s in &g.series {
            table.text(s);
        }
    }
    for (i, row) in cells.iter().enumerate() {
        if let Some(c) = g.categories.get(i) {
            table.text(c);
        }
        for v in row {
            match v {
                Some(v) => table.num(*v),
                None => table.text(""),
            }
        }
    }
    table.finish();
    head.push_str("%_GC\n");
    Some(Records { head, tags })
}

/// The `Gc` records of a table, its cells in rows: at most [`CELLS_PER_RECORD`] values each, one
/// string at most.
struct Table<'a> {
    out: &'a mut String,
    values: usize,
    string: bool,
}

impl<'a> Table<'a> {
    fn new(out: &'a mut String) -> Self {
        Self { out, values: 0, string: false }
    }

    fn start(&mut self, string: bool) {
        if self.values == CELLS_PER_RECORD || (string && self.string) {
            self.finish();
        }
        self.out.push_str(if self.values == 0 { "%_" } else { " " });
        self.values += 1;
        self.string |= string;
    }

    fn text(&mut self, t: &str) {
        self.start(true);
        self.out.push_str(&ps::bytes(t.as_bytes()));
    }

    fn num(&mut self, v: f64) {
        self.start(false);
        self.out.push_str(&real(v));
    }

    fn finish(&mut self) {
        if self.values > 0 {
            self.out.push_str(" Gc\n");
        }
        (self.values, self.string) = (0, false);
    }
}

/// `v` as a PostScript number that reads back as the same value.
fn real(v: f64) -> String {
    if v == 0.0 {
        "0".into()
    } else if (1e-4..1e15).contains(&v.abs()) {
        format!("{v}")
    } else {
        format!("{v:e}")
    }
}

/// `graphType`: 5 grouped column, 6 stacked column, 7 line, 10 area; the reader reads no other type.
fn kind_code(k: GraphKind) -> Option<u8> {
    match k {
        GraphKind::Column => Some(5),
        GraphKind::StackedColumn => Some(6),
        GraphKind::Line => Some(7),
        GraphKind::Area => Some(10),
        _ => None,
    }
}

/// `tickMarkType`: 13 none, 14 short, 15 the length of the plot.
fn tick(t: TickLength) -> u8 {
    match t {
        TickLength::None => 13,
        TickLength::Short => 14,
        TickLength::Full => 15,
    }
}

/// Whether `g`'s settings are ones the records give and the reader keeps: a legend beside or across
/// the top, every series of the graph's type, one value axis on the left, lines not filled and no
/// Line Width of their own, no graph
/// designs, label formatting or pie settings, column and cluster widths of 1% to 1000%, and at most
/// [`MAX_TICK_MARKS`] tick marks a division.
fn settings_kept(g: &GraphSpec) -> bool {
    let percent = |p: f64| (1.0..=1000.0).contains(&p);
    g.legend
        && g.series_kinds.iter().all(|k| k.is_none_or(|k| k == g.kind))
        && g.value_axis == ValueAxisSide::Left
        && !g.separate_scales
        && g.right_series.is_empty()
        && !g.filled_lines
        && g.line_width.is_none()
        && g.series_markers.iter().all(Option::is_none)
        && g.series_columns.iter().all(Option::is_none)
        && g.label_styles == LabelStyles::default()
        && !g.pie_legend_in_wedges
        && g.pie_position == PiePosition::Even
        && g.pie_sort == PieSort::None
        && percent(g.column_width)
        && percent(g.cluster_width)
        && [g.tick_marks, g.right_tick_marks, g.category_tick_marks].iter().all(|m| *m <= MAX_TICK_MARKS)
}

/// A value axis' `useManualValues`, lowest and highest value and `betweenValue`: automatic (no tick
/// count and no values), or a whole number of divisions (1 to [`MAX_DIVISIONS`]) between a lowest
/// and a higher highest value, which the reader reads back as the same count.
fn axis(ticks: usize, min: Option<f64>, max: Option<f64>) -> Option<(bool, f64, f64, f64)> {
    match (ticks, min, max) {
        (0, None, None) => Some((false, 0.0, 0.0, 0.0)),
        (1..=MAX_DIVISIONS, Some(lo), Some(hi)) if lo.is_finite() && hi.is_finite() && hi > lo => {
            let step = (hi - lo) / ticks as f64;
            let n = (hi - lo) / step;
            let exact = step > 0.0 && step.is_finite() && (n - n.round()).abs() <= 1e-6 * n.round().max(1.0) && n.round() == ticks as f64;
            exact.then_some((true, lo, hi, step))
        }
        _ => None,
    }
}

/// Whether `n` and everything in it is shown.
fn all_shown(n: &Node) -> bool {
    n.visible && n.children().is_none_or(|c| c.iter().all(|c| all_shown(c)))
}

/// Whether `n` is shown, fully opaque, in Normal mode, not isolated and without an opacity mask or
/// effects: the reader keeps a series' paint only from groups and paths drawn so.
fn as_drawn(n: &Node) -> bool {
    n.visible
        && n.opacity == 1.0
        && n.blend == BlendMode::Normal
        && !n.isolate
        && n.extras().mask.is_none()
        && !n.appearance.effects.iter().any(|e| e.visible)
}

/// The visible fill and line `p` is written with as one path object: a solid fill, and a solid
/// plain line with its weight, each opaque, in Normal mode and without effects of its own. `None`
/// when it has other paint, more than one of either, or isn't one path of one subpath.
#[allow(clippy::type_complexity)]
fn plain_paint(p: &Node) -> Option<(Option<Paint>, Option<(Paint, f64)>)> {
    let NodeKind::Path { path, guide: false, clipping: false, .. } = &p.kind else { return None };
    (path.subpaths.iter().filter(|sp| !sp.anchors.is_empty()).count() == 1).then_some(())?;
    let (mut fill, mut line) = (None, None);
    for item in &p.appearance.items {
        let drawn = match item {
            AppearanceItem::Fill(f) => f.visible && !f.paint.is_none(),
            AppearanceItem::Stroke(s) => s.visible && !s.paint.is_none() && s.width > 0.0 && s.width.is_finite(),
        };
        if !drawn {
            // The writers leave out what draws nothing; the reader reads nothing in its place.
            continue;
        }
        let plain =
            item.opacity() == 1.0 && item.blend() == BlendMode::Normal && item.effects().is_empty() && matches!(item.paint(), Paint::Solid { .. });
        plain.then_some(())?;
        match item {
            AppearanceItem::Fill(f) if fill.is_none() && line.is_none() => fill = Some(f.paint.clone()),
            AppearanceItem::Stroke(s) if line.is_none() && stroke::is_plain(s) => line = Some((s.paint.clone(), s.width)),
            _ => return None,
        }
    }
    Some((fill, line))
}

/// The `Go` line of each series' group among `children`: a group whose series index is one of the
/// `series` the table has, each once, of paths [`as_drawn`] with one fill and one line alike
/// ([`plain_paint`]), every path filled unless the graph draws lines (`line`). Its column is the
/// table's: after the label column when there is one (`first_col`). `None` when a series has no
/// such group.
fn series_groups(children: &[std::sync::Arc<Node>], series: usize, first_col: usize, line: bool) -> Option<HashMap<NodeId, String>> {
    let mut tags = HashMap::new();
    let mut seen = vec![false; series];
    for c in children {
        let Some(s) = c.extras().series_index else { continue };
        let s = usize::try_from(s).ok().filter(|s| *s < series)?;
        let slot = seen.get_mut(s)?;
        (!*slot).then_some(())?;
        *slot = true;
        let NodeKind::Group { children: paths, clip: false } = &c.kind else { return None };
        (as_drawn(c) && *c.appearance == Appearance::default() && !paths.is_empty()).then_some(())?;
        let mut paints = vec![];
        for p in paths {
            as_drawn(p).then_some(())?;
            paints.push(plain_paint(p)?);
        }
        let fill = paints.iter().find_map(|(f, _)| f.clone());
        paints.iter().all(|(f, _)| f.is_none() || *f == fill).then_some(())?;
        (line || paints.iter().all(|(f, _)| f.is_some())).then_some(())?;
        let stroke = paints.first().and_then(|(_, s)| s.clone());
        paints.iter().all(|(_, s)| *s == stroke).then_some(())?;
        tags.insert(c.id, format!("%_2 {} 0 0 Go", s + first_col));
    }
    seen.iter().all(|s| *s).then_some(tags)
}

/// Whether `art` draws plot `plot`'s left edge and its bottom edge (the document's y runs down) as
/// straight lines, each a shown path of one subpath with a plain line ([`plain_paint`]) in a shown
/// group, as the reader finds the axes.
fn draws_axes(art: &Node, plot: Rect) -> bool {
    let near = |a: Point, b: Point| (a - b).hypot() <= EDGE;
    let on = |s: &PathSeg, a: Point, b: Point| match s {
        PathSeg::Line(l) => (near(l.p0, a) && near(l.p1, b)) || (near(l.p0, b) && near(l.p1, a)),
        _ => false,
    };
    let (top_left, bottom_left, bottom_right) = (Point::new(plot.x0, plot.y0), Point::new(plot.x0, plot.y1), Point::new(plot.x1, plot.y1));
    let (mut left, mut bottom) = (false, false);
    let mut stack = vec![art];
    while let Some(n) = stack.pop() {
        if !n.visible || n.opacity <= 0.0 || (left && bottom) {
            continue;
        }
        match &n.kind {
            NodeKind::Group { children, clip: false } => stack.extend(children.iter().map(|c| &**c)),
            NodeKind::Path { path, .. } if plain_paint(n).is_some_and(|(_, line)| line.is_some()) => {
                for s in path.to_bezpath().segments() {
                    left |= on(&s, top_left, bottom_left);
                    bottom |= on(&s, bottom_left, bottom_right);
                }
            }
            _ => {}
        }
    }
    left && bottom
}

/// Graphs built as the graph tools draw them, for the writers' tests.
#[cfg(test)]
pub(crate) mod sample {
    use std::sync::Arc;

    use vectorcraft_color::{Color, Paint};
    use vectorcraft_doc::{Document, GraphKind, GraphSpec, Node, PieSort, TickLength, ValueAxisSide};
    use vectorcraft_geom::{PathData, Point, Rect, shapes};

    use super::Appearance;

    /// A graph of kind `kind`: series East and West, categories Q1 to Q3, West blank in Q2, on the
    /// plot 100 100 300 250.
    pub(crate) fn spec(kind: GraphKind) -> GraphSpec {
        let mut g = GraphSpec {
            kind,
            rect: Rect::new(100.0, 100.0, 300.0, 250.0),
            series: vec!["East".into(), "West".into()],
            categories: vec!["Q1".into(), "Q2".into(), "Q3".into()],
            ..GraphSpec::default()
        };
        g.set_cells(vec![vec![Some(5.0), Some(3.0)], vec![Some(7.0), None], vec![Some(4.0), Some(6.0)]]);
        g
    }

    fn path(doc: &mut Document, p: PathData, fill: Paint, stroke: Paint) -> Arc<Node> {
        Arc::new(Node::path(doc.alloc_id(), p, Appearance::basic(fill, stroke, 1.0)))
    }

    /// Graph `g` drawn as a group: a group of each series' marks (a line graph's series as an unfilled
    /// line, the others' as filled columns), its axes on the plot's left and bottom edges, and a
    /// legend of a swatch for each series; its series groups named after the series.
    pub(crate) fn graph(doc: &mut Document, g: GraphSpec) -> Node {
        let r = g.rect;
        let cells = g.cells();
        let width = cells.first().map_or(0, Vec::len);
        let mut members = vec![];
        let axes = vec![
            path(doc, shapes::line(Point::new(r.x0, r.y0), Point::new(r.x0, r.y1)), Paint::None, Paint::solid(Color::BLACK)),
            path(doc, shapes::line(Point::new(r.x0, r.y1), Point::new(r.x1, r.y1)), Paint::None, Paint::solid(Color::BLACK)),
        ];
        members.push(Arc::new(Node::group(doc.alloc_id(), axes)));
        for s in 0..width {
            let gray = Paint::solid(Color::gray(0.2 + 0.3 * s as f32));
            let points: Vec<Point> = cells
                .iter()
                .enumerate()
                .filter_map(|(c, row)| {
                    row.get(s).copied().flatten().map(|v| Point::new(r.x0 + 20.0 + 60.0 * c as f64 + 15.0 * s as f64, r.y1 - v * 10.0))
                })
                .collect();
            let marks: Vec<Arc<Node>> = if g.kind == GraphKind::Line {
                let mut bp = kurbo::BezPath::new();
                for (i, p) in points.iter().enumerate() {
                    if i == 0 { bp.move_to(*p) } else { bp.line_to(*p) }
                }
                vec![path(doc, PathData::from_bezpath(&bp), Paint::None, gray)]
            } else {
                points.iter().map(|p| path(doc, shapes::rectangle(Rect::new(p.x, p.y, p.x + 10.0, r.y1)), gray.clone(), Paint::None)).collect()
            };
            let mut series = Node::group(doc.alloc_id(), marks);
            series.name = Some(g.series.get(s).cloned().unwrap_or_else(|| format!("Series {}", s + 1)));
            series.extras_mut().series_index = Some(s as u32);
            members.push(Arc::new(series));
        }
        let swatches: Vec<Arc<Node>> = (0..width)
            .map(|s| {
                let y = r.y0 + 12.0 * s as f64;
                path(
                    doc,
                    shapes::rectangle(Rect::new(r.x1 + 10.0, y, r.x1 + 18.0, y + 8.0)),
                    Paint::solid(Color::gray(0.2 + 0.3 * s as f32)),
                    Paint::None,
                )
            })
            .collect();
        members.push(Arc::new(Node::group(doc.alloc_id(), swatches)));
        let mut n = Node::group(doc.alloc_id(), members);
        n.extras_mut().graph = Some(Box::new(g));
        n
    }

    /// A 400 × 300 document holding `g` drawn by [`graph`] on its layer.
    pub(crate) fn document(g: GraphSpec) -> Document {
        let mut doc = Document::new(400.0, 300.0);
        let layer = doc.layers[0].id;
        let n = graph(&mut doc, g);
        doc.insert(Some(layer), usize::MAX, n).unwrap();
        doc
    }

    /// Graphs whose records read back whole: each type the records give, every setting they keep
    /// changed, labels with parentheses, backslashes and non-ASCII characters, a label of 255 bytes,
    /// and a table without labels.
    pub(crate) fn kept() -> Vec<(&'static str, Document)> {
        let mut cases = vec![
            ("column", document(spec(GraphKind::Column))),
            ("stacked column", document(spec(GraphKind::StackedColumn))),
            ("line", document(spec(GraphKind::Line))),
            ("area", document(spec(GraphKind::Area))),
        ];
        let mut g = spec(GraphKind::Column);
        (g.drop_shadow, g.first_row_in_front, g.first_column_in_front, g.legend_across_top) = (true, true, true, true);
        (g.mark_points, g.connect_points, g.edge_to_edge) = (false, false, true);
        (g.column_width, g.cluster_width) = (120.0, 50.0);
        (g.tick_length, g.tick_marks, g.ticks, g.axis_min, g.axis_max) = (TickLength::Full, 3, 4, Some(0.0), Some(20.0));
        (g.right_tick_length, g.right_tick_marks, g.right_ticks, g.right_axis_min, g.right_axis_max) = (TickLength::None, 2, 5, Some(1.0), Some(6.0));
        (g.category_tick_length, g.category_tick_marks, g.ticks_between_labels) = (TickLength::Short, 2, true);
        (g.prefix, g.suffix, g.right_prefix, g.right_suffix) = ("($".into(), "\\)".into(), "é".into(), " u".into());
        cases.push(("settings", document(g)));
        let mut g = spec(GraphKind::Column);
        g.series = vec!["A (1)".into(), "B\\2".into()];
        g.categories = vec![String::new(), "naïve".into(), "a".repeat(255)];
        g.set_cells(vec![vec![Some(1.0 / 3.0), Some(1e-7)], vec![None, Some(2.5)], vec![Some(12.0), None]]);
        cases.push(("labels", document(g)));
        let mut g = spec(GraphKind::Column);
        (g.series, g.categories) = (vec![], vec![]);
        cases.push(("no labels", document(g)));
        cases
    }

    /// Graphs the records can't give as they are, which are written as their art: the five other
    /// types, settings the reader doesn't keep, a table it can't hold, and art it can't attach the
    /// graph to.
    pub(crate) fn unkept() -> Vec<(&'static str, Document)> {
        let with = |f: &dyn Fn(&mut GraphSpec)| {
            let mut g = spec(GraphKind::Column);
            f(&mut g);
            document(g)
        };
        let mut cases: Vec<(&'static str, Document)> = [GraphKind::Bar, GraphKind::StackedBar, GraphKind::Scatter, GraphKind::Pie, GraphKind::Radar]
            .into_iter()
            .map(|k| (k.id(), document(spec(k))))
            .collect();
        cases.extend([
            ("no legend", with(&|g| g.legend = false)),
            ("filled lines", with(&|g| (g.kind, g.filled_lines) = (GraphKind::Line, true))),
            ("a filled line width", with(&|g| (g.kind, g.line_width) = (GraphKind::Line, Some(8.0)))),
            ("a series of another type", with(&|g| g.series_kinds = vec![None, Some(GraphKind::Line)])),
            ("a marker design", with(&|g| g.series_markers = vec![Some("Star".into())])),
            ("a column design", with(&|g| g.series_columns = vec![Some(Default::default())])),
            ("label formatting", with(&|g| g.label_styles.values = Some(Default::default()))),
            ("a value axis on both sides", with(&|g| g.value_axis = ValueAxisSide::Both)),
            ("a series on the right axis", with(&|g| g.right_series = vec![1])),
            ("pie settings", with(&|g| g.pie_sort = PieSort::All)),
            ("a manual axis without its values", with(&|g| g.ticks = 4)),
            ("values without a tick count", with(&|g| (g.axis_min, g.axis_max) = (Some(0.0), Some(10.0)))),
            ("101 divisions", with(&|g| (g.ticks, g.axis_min, g.axis_max) = (101, Some(0.0), Some(101.0)))),
            ("too many tick marks", with(&|g| g.tick_marks = 21)),
            ("a column width past 1000%", with(&|g| g.column_width = 1001.0)),
            ("a long prefix", with(&|g| g.prefix = "0123456789".into())),
            ("a line break in a label", with(&|g| g.series[0] = "East\nside".into())),
            ("a label of 256 bytes", with(&|g| g.categories[0] = "a".repeat(256))),
            ("a value that isn't finite", with(&|g| g.rows[0][0] = f64::NAN)),
            ("only blank cells", with(&|g| g.set_cells(vec![vec![None, None]; 3]))),
            ("fewer labels than series", with(&|g| g.series.truncate(1))),
        ]);
        let drawn = |what: &'static str, f: &dyn Fn(&mut Vec<Arc<Node>>)| {
            let mut doc = Document::new(400.0, 300.0);
            let mut n = graph(&mut doc, spec(GraphKind::Column));
            if let Some(c) = n.children_mut() {
                f(c);
            }
            let layer = doc.layers[0].id;
            doc.insert(Some(layer), usize::MAX, n).unwrap();
            (what, doc)
        };
        cases.extend([
            drawn("a series without its group", &|c| {
                c.remove(2);
            }),
            drawn("no axes", &|c| {
                c.remove(0);
            }),
            drawn("a hidden legend", &|c| {
                if let Some(l) = c.last_mut() {
                    Arc::make_mut(l).visible = false;
                }
            }),
            drawn("a series drawn half opaque", &|c| Arc::make_mut(&mut c[1]).opacity = 0.5),
            drawn("a series mark with a gradient", &|c| {
                let g = vectorcraft_color::Gradient::new(
                    vectorcraft_color::GradientKind::Linear,
                    vec![vectorcraft_color::GradientStop::new(0.0, Color::BLACK), vectorcraft_color::GradientStop::new(1.0, Color::WHITE)],
                );
                let series = Arc::make_mut(&mut c[1]);
                if let Some(marks) = series.children_mut() {
                    let mark = Arc::make_mut(&mut marks[0]);
                    mark.appearance = Appearance::basic(Paint::Gradient(Box::new(vectorcraft_color::GradientPaint::new(g))), Paint::None, 0.0).into();
                }
            }),
        ]);
        cases
    }

    /// The graphs in `doc`, each with the series index of its members.
    pub(crate) fn graphs(doc: &Document) -> Vec<(GraphSpec, Vec<Option<u32>>)> {
        let mut out = vec![];
        doc.walk(|n| {
            if let Some(g) = n.extras().graph.as_deref() {
                out.push((g.clone(), n.children().map(|c| c.iter().map(|c| c.extras().series_index).collect()).unwrap_or_default()));
            }
        });
        out
    }

    /// Whether `read` is the graph `written` was: the same data and settings, the plot in the same
    /// place; its series' paint is the art's and its bounds are the art's as read.
    pub(crate) fn same(written: &GraphSpec, read: &GraphSpec) -> bool {
        let near = |a: Rect, b: Rect| (a.x0 - b.x0).abs() + (a.y0 - b.y0).abs() + (a.x1 - b.x1).abs() + (a.y1 - b.y1).abs() < 1e-6;
        let plain = |g: &GraphSpec| GraphSpec {
            rect: Rect::ZERO,
            rows: vec![],
            blanks: vec![],
            series_paints: vec![],
            placed: None,
            placed_art: None,
            series_kinds: vec![],
            ..g.clone()
        };
        near(written.rect, read.rect) && written.cells() == read.cells() && plain(written) == plain(read)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Numbers are written so they read back the same, small and large ones included.
    #[test]
    fn numbers_read_back_as_written() {
        for v in [0.0, 1.0, -2.5, 1.0 / 3.0, 1e-7, 123_456_789_012_345_678.0, f64::MAX, f64::MIN_POSITIVE] {
            assert_eq!(real(v).parse::<f64>().ok(), Some(v), "{v}");
        }
    }

    /// A graph of `categories` rows of `series` values, drawn by [`sample::graph`].
    fn sized(categories: usize, series: usize) -> Node {
        let mut g = sample::spec(GraphKind::Column);
        (g.series, g.categories) = (vec![], vec![]);
        g.set_cells(vec![vec![Some(1.0); series]; categories]);
        sample::graph(&mut vectorcraft_doc::Document::new(400.0, 300.0), g)
    }

    /// The table the reader holds: up to 10,000 categories and 256 series; past them, the graph is
    /// its art.
    #[test]
    fn tables_up_to_the_readers_limits_are_written() {
        assert!(records(&sized(MAX_GRAPH_CATEGORIES, 1), Affine::IDENTITY).is_some_and(|r| r.head.contains("%_10000 1 0 0 Gz\n")));
        assert!(records(&sized(MAX_GRAPH_CATEGORIES + 1, 1), Affine::IDENTITY).is_none());
        assert!(records(&sized(1, MAX_GRAPH_SERIES), Affine::IDENTITY).is_some_and(|r| r.tags.len() == MAX_GRAPH_SERIES));
        assert!(records(&sized(1, MAX_GRAPH_SERIES + 1), Affine::IDENTITY).is_none());
    }

    /// Tick-mark counts of 0, which mean one tick mark as 1 does, are written as 1, the count the
    /// reader reads back.
    #[test]
    fn tick_mark_counts_of_0_are_written_as_1() {
        let mut g = sample::spec(GraphKind::Column);
        (g.tick_marks, g.right_tick_marks, g.category_tick_marks) = (0, 0, 0);
        let n = sample::graph(&mut vectorcraft_doc::Document::new(400.0, 300.0), g);
        let head = records(&n, Affine::IDENTITY).map(|r| r.head).unwrap_or_default();
        let marks: Vec<&str> = head.lines().filter(|l| l.ends_with(" GA")).map(|l| l.split(' ').nth(5).unwrap_or("")).collect();
        assert_eq!(marks, ["1", "1", "1"], "{head}");
    }

    /// A table's cells go fifteen to a record, with one string at most in each.
    #[test]
    fn cells_go_fifteen_to_a_record_with_one_string_each() {
        let mut out = String::new();
        let mut t = Table::new(&mut out);
        t.text("a");
        t.text("b");
        for v in 0..16 {
            t.num(f64::from(v));
        }
        t.finish();
        let records: Vec<&str> = out.lines().collect();
        assert_eq!(records[0], "%_(a) Gc");
        assert_eq!(records[1], "%_(b) 0 1 2 3 4 5 6 7 8 9 10 11 12 13 Gc");
        assert_eq!(records[2], "%_14 15 Gc");
    }
}
