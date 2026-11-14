//! A graph's records (version 7 specification §17 and appendix A) and the live graph they give.
//!
//! A graph is `%_Gs`, its functional records on `%_` lines (`Gb`, `Gy`, `Gd`, any `Gj`, three
//! `Ga` … `GA` axes, any `Gw`, `Gz`, `Gc` cells and `GC`), any `version Gt` … `GT` customizations, its
//! drawn art in one group (objects followed by `%_… Go`), then `%_GS`. The drawn art is always
//! read as it is. The group also keeps the graph's data and settings ([`GraphSpec`]) when every
//! record has the shape and values appendix A gives and the model holds each setting, the table
//! has data, the art draws the plot's left and bottom edges, and each series' group is tagged, is
//! one of the graph's own groups and paints its paths with one fill and one stroke, which the
//! graph keeps as the series' paint.
//! Any other graph stays plain art, with a note.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use kurbo::{PathSeg, Point};
use vectorcraft_doc::{Appearance, GraphKind, GraphSpec, MAX_GRAPH_CATEGORIES, MAX_GRAPH_SERIES, SeriesPaint, TickLength};

use super::types::{GRAPH_OPS, GRAPH_PLAIN, GRAPH_UNENDED, Kind};
use super::{Affine, AppearanceItem, BlendMode, Node, NodeId, NodeKind, Paint, Reader, Rect, V, obj};

/// Most cells of a graph's table, its label row and column included.
const MAX_CELLS: usize = 2_600_000;

/// Longest cell string (appendix A, `Gc`).
const MAX_CELL_TEXT: usize = 255;

/// Most characters of an axis label's text before or after its numbers (appendix A, `Ga`, `GA`).
const MAX_AFFIX: usize = 9;

/// Most tick marks per division a graph draws.
const MAX_TICK_MARKS: usize = 20;

/// Most divisions of a value axis with set values.
const MAX_DIVISIONS: f64 = 100.0;

/// How far an axis line may be from the plot's edge, in points.
const EDGE: f64 = 0.01;

/// The axes in the order the records give them: bottom, left, right.
const AXES: [i64; 3] = [1, 2, 4];

/// The `Go` targets appendix A lists.
const TARGETS: [i64; 11] = [1, 2, 4, 5, 6, 8, 9, 10, 15, 20, 22];

/// The version of a graph's customizations (appendix A, `Gt`).
const CUSTOM_VERSION: i64 = 2;

/// `Go`'s target for one series, including its legend.
const SERIES: i64 = 2;

/// A graph being read (`%_Gs` … `%_GS`).
#[derive(Debug)]
pub(super) struct Graph {
    /// Between `Gt` and `GT`, where the graph's settings may use paint and type operators on `%_`
    /// lines too.
    pub(super) settings: bool,
    /// How many frames were open at its `Gs`: a layer that ends with fewer ends the graph.
    pub(super) frames: usize,
    /// How many objects the innermost frame held at `Gs`: the drawn art is the one after them.
    siblings: usize,
    /// The records read so far, while they can still give a live graph.
    live: Option<Box<Records>>,
}

/// Where the records are, in the order appendix A gives.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum Stage {
    #[default]
    Bounds,
    Style,
    Dialog,
    /// Before the `Ga` of axis `n`.
    Axis(usize),
    /// Before the `GA` of axis `n`.
    AxisEnd(usize),
    Widths,
    Cells,
    /// After `GC`: customizations, then the drawn art.
    Done,
    /// Between `Gt` and `GT`.
    Custom,
}

/// One axis: `whichAxis beforeString Ga` and `useManualValues tickMarkType minimumValue maximumValue
/// betweenValue smallTicksPerValue drawMarksBetweenLabels afterString GA`.
#[derive(Debug, Default)]
struct Axis {
    before: Vec<u8>,
    manual: bool,
    tick: TickLength,
    min: f64,
    max: f64,
    between: f64,
    small: usize,
    between_labels: bool,
    after: Vec<u8>,
}

/// A cell of the table (`Gc`).
#[derive(Debug)]
enum Cell {
    Num(f64),
    Text(Vec<u8>),
}

/// The records of a graph read so far.
#[derive(Debug, Default)]
struct Records {
    stage: Stage,
    spec: GraphSpec,
    axes: [Axis; 3],
    /// `Gw`: the first column and how many widths.
    widths: Vec<(usize, usize)>,
    /// `Gz`: rows, columns, first data row and first data column.
    size: Option<[usize; 4]>,
    cells: Vec<Cell>,
    /// The groups `Go` tags as one series, and the column each names.
    series: Vec<(NodeId, usize)>,
}

/// A whole number.
fn int(v: &V) -> Option<i64> {
    v.num().filter(|x| x.is_finite() && x.fract() == 0.0 && x.abs() < 1e15).map(|x| x as i64)
}

/// 0 or 1.
fn flag(v: &V) -> Option<bool> {
    match int(v)? {
        0 => Some(false),
        1 => Some(true),
        _ => None,
    }
}

fn finite(v: &V) -> Option<f64> {
    v.num().filter(|x| x.is_finite())
}

/// A count from 0 to `max`.
fn count(v: &V, max: usize) -> Option<usize> {
    usize::try_from(int(v)?).ok().filter(|n| *n <= max)
}

/// `tickMarkType`: 13 none, 14 short, 15 the length of the plot.
fn tick(v: &V) -> Option<TickLength> {
    match int(v)? {
        13 => Some(TickLength::None),
        14 => Some(TickLength::Short),
        15 => Some(TickLength::Full),
        _ => None,
    }
}

/// `graphType`: 5 grouped column, 6 stacked column, 7 line, 10 area. Pie (8) and scatter (9) graphs
/// keep settings the records don't give the model, and other values are not graph types.
fn kind(v: &V) -> Option<GraphKind> {
    match int(v)? {
        5 => Some(GraphKind::Column),
        6 => Some(GraphKind::StackedColumn),
        7 => Some(GraphKind::Line),
        10 => Some(GraphKind::Area),
        _ => None,
    }
}

/// A percentage of the available width, 1 to 1000.
fn percent(v: &V) -> Option<f64> {
    finite(v).filter(|p| (1.0..=1000.0).contains(p))
}

/// The tick count and values of an axis: with set values (`useManualValues` 1), the divisions
/// between its lowest and highest value, a whole number of `betweenValue` steps from the lowest up.
fn values(a: &Axis) -> Option<(usize, Option<f64>, Option<f64>)> {
    if !a.manual {
        return Some((0, None, None));
    }
    let n = (a.max - a.min) / a.between;
    let whole = n.round();
    let exact = (n - whole).abs() <= 1e-6 * whole.max(1.0);
    (a.between > 0.0 && a.max > a.min && exact && (1.0..=MAX_DIVISIONS).contains(&whole)).then_some((whole as usize, Some(a.min), Some(a.max)))
}

impl Records {
    /// Take the record `w` with its operands `vals`; `None` when its shape, values or place aren't
    /// those appendix A gives, or the model doesn't hold a value it sets. `to_doc` takes art space
    /// to the document.
    fn step(&mut self, w: &str, vals: &[V], to_doc: Affine) -> Option<()> {
        match (w, self.stage) {
            ("Gb", Stage::Bounds) => {
                let [l, t, r, b] = vals else { return None };
                let [l, t, r, b] = [finite(l)?, finite(t)?, finite(r)?, finite(b)?];
                let plot = to_doc.transform_rect_bbox(Rect::new(l, b, r, t));
                let sound = r > l && t > b && plot.width() > 0.0 && plot.height() > 0.0 && plot.width().is_finite() && plot.height().is_finite();
                sound.then_some(())?;
                self.spec.rect = plot;
                self.stage = Stage::Style;
            }
            ("Gy", Stage::Style) => {
                self.style(vals)?;
                self.stage = Stage::Dialog;
            }
            ("Gd", Stage::Dialog) => {
                let [col_width, precision, series_order, both_axes, bar, group, edge] = vals else { return None };
                count(col_width, 20).filter(|w| *w >= 3)?;
                count(precision, 10)?;
                // `useBothAxis` copies one axis' values to the other, which the model doesn't do.
                (!flag(both_axes)?).then_some(())?;
                self.spec.first_column_in_front = flag(series_order)?;
                self.spec.column_width = percent(bar)?;
                self.spec.cluster_width = percent(group)?;
                self.spec.edge_to_edge = flag(edge)?;
                self.stage = Stage::Axis(0);
            }
            ("Ga", Stage::Axis(n)) if n < AXES.len() => {
                let [axis, V::Str(before)] = vals else { return None };
                (int(axis)? == *AXES.get(n)?).then_some(())?;
                self.axes.get_mut(n)?.before = before.clone();
                self.stage = Stage::AxisEnd(n);
            }
            ("GA", Stage::AxisEnd(n)) => {
                let [manual, ticks, min, max, between, small, between_labels, V::Str(after)] = vals else { return None };
                let a = self.axes.get_mut(n)?;
                (a.manual, a.tick) = (flag(manual)?, tick(ticks)?);
                (a.min, a.max, a.between) = (finite(min)?, finite(max)?, finite(between)?);
                a.small = count(small, 1000)?;
                a.between_labels = flag(between_labels)?;
                a.after = after.clone();
                self.stage = Stage::Axis(n + 1);
            }
            ("Gw", Stage::Axis(3) | Stage::Widths) => {
                // `col width1 … widthX numParams Gw`: numParams counts `col` and the 1 to 14 widths.
                let (params, rest) = vals.split_last()?;
                let params = count(params, 15).filter(|p| *p >= 2 && *p == rest.len())?;
                let (col, widths) = rest.split_first()?;
                let col = count(col, MAX_CELLS)?;
                widths.iter().all(|w| int(w).is_some_and(|w| w == -1 || (3..=20).contains(&w))).then_some(())?;
                self.widths.push((col, params - 1));
                self.stage = Stage::Widths;
            }
            ("Gz", Stage::Axis(3) | Stage::Widths) => {
                let [rows, cols, first_row, first_col] = vals else { return None };
                let (rows, cols) = (count(rows, MAX_CELLS)?, count(cols, MAX_CELLS)?);
                let (first_row, first_col) = (usize::from(flag(first_row)?), usize::from(flag(first_col)?));
                rows.checked_sub(first_row).filter(|c| (1..=MAX_GRAPH_CATEGORIES).contains(c))?;
                cols.checked_sub(first_col).filter(|s| (1..=MAX_GRAPH_SERIES).contains(s))?;
                rows.checked_mul(cols).filter(|n| *n <= MAX_CELLS)?;
                self.widths.iter().all(|(col, n)| col.checked_add(*n).is_some_and(|end| end <= cols)).then_some(())?;
                self.size = Some([rows, cols, first_row, first_col]);
                self.stage = Stage::Cells;
            }
            ("Gc", Stage::Cells) => {
                // One to fifteen values, at most one of them a string.
                let strings = vals.iter().filter(|v| matches!(v, V::Str(_))).count();
                ((1..=15).contains(&vals.len()) && strings <= 1).then_some(())?;
                let [rows, cols, ..] = self.size?;
                (self.cells.len() + vals.len() <= rows * cols).then_some(())?;
                for v in vals {
                    self.cells.push(match v {
                        V::Str(s) if s.len() <= MAX_CELL_TEXT => Cell::Text(s.clone()),
                        _ => Cell::Num(finite(v)?),
                    });
                }
            }
            ("GC", Stage::Cells) if vals.is_empty() => self.stage = Stage::Done,
            // `version Gt`, of version 2. Customizations are edits made after the graph was built, which
            // the model can't replay: only an empty list keeps the graph live.
            ("Gt", Stage::Done) => {
                let [version] = vals else { return None };
                (int(version)? == CUSTOM_VERSION).then_some(())?;
                self.stage = Stage::Custom;
            }
            ("GT", Stage::Custom) if vals.is_empty() => self.stage = Stage::Done,
            _ => return None,
        }
        Some(())
    }

    /// `graphType shadow dataPointOrder pieLegendStyle drawMarks drawLines drawLinesAsShapes
    /// drawLegendsAcrossTop lineShapeWidth whichAxis [piePercentage piePctDigits] Gy`.
    fn style(&mut self, vals: &[V]) -> Option<()> {
        let (base, pie) = match vals.len() {
            10 => (vals, None),
            12 => vals.split_at_checked(10).map(|(b, p)| (b, Some(p)))?,
            _ => return None,
        };
        let [ty, shadow, order, legend, marks, lines, shapes, across, width, axis] = base else { return None };
        self.spec.kind = kind(ty)?;
        self.spec.drop_shadow = flag(shadow)?;
        self.spec.first_row_in_front = flag(order)?;
        // 14: legends as in bar and line graphs; 15 (in wedges) and 16 (none) are pie styles.
        (int(legend)? == 14).then_some(())?;
        self.spec.legend = true;
        self.spec.mark_points = flag(marks)?;
        self.spec.connect_points = flag(lines)?;
        // Lines drawn as shapes have a width the model doesn't keep.
        (!flag(shapes)?).then_some(())?;
        self.spec.filled_lines = false;
        self.spec.legend_across_top = flag(across)?;
        finite(width).filter(|w| (0.0..=100.0).contains(w))?;
        // 44: the series are measured against the left axis; 45 (right) isn't kept per series.
        (int(axis)? == 44).then_some(())?;
        if let Some([percentage, digits]) = pie {
            // Pie percentages, which only pie graphs show; shown (1) isn't kept.
            (!flag(percentage)?).then_some(())?;
            int(digits).filter(|d| *d >= 0)?;
        }
        Some(())
    }

    /// The graph's data and settings, its strings read as names in `names`; `None` when the table
    /// or an axis holds what the model can't.
    fn finish(&self, names: &mut obj::Names) -> Option<GraphSpec> {
        let [rows, cols, first_row, first_col] = self.size?;
        let mut spec = self.spec.clone();
        let text = |names: &mut obj::Names, b: &[u8], max: usize| {
            let t = names.quiet(b);
            (t.chars().count() <= max && !t.chars().any(char::is_control)).then(|| names.bytes(b))
        };
        // Axes: the bottom one is the category axis, the left one the value axis; the right one keeps
        // its settings for when it is shown.
        let [bottom, left, right] = &self.axes;
        (bottom.before.is_empty() && bottom.after.is_empty() && !bottom.manual).then_some(())?;
        (!left.between_labels && !right.between_labels).then_some(())?;
        (spec.category_tick_length, spec.category_tick_marks) = (bottom.tick, bottom.small.max(1));
        spec.ticks_between_labels = bottom.between_labels;
        (spec.tick_length, spec.tick_marks) = (left.tick, left.small.max(1));
        (spec.right_tick_length, spec.right_tick_marks) = (right.tick, right.small.max(1));
        (spec.category_tick_marks <= MAX_TICK_MARKS && spec.tick_marks <= MAX_TICK_MARKS && spec.right_tick_marks <= MAX_TICK_MARKS).then_some(())?;
        (spec.ticks, spec.axis_min, spec.axis_max) = values(left)?;
        (spec.right_ticks, spec.right_axis_min, spec.right_axis_max) = values(right)?;
        spec.prefix = text(names, &left.before, MAX_AFFIX)?;
        spec.suffix = text(names, &left.after, MAX_AFFIX)?;
        spec.right_prefix = text(names, &right.before, MAX_AFFIX)?;
        spec.right_suffix = text(names, &right.after, MAX_AFFIX)?;
        // The table, row by row; cells left out at its end are empty.
        let cell = |r: usize, c: usize| self.cells.get(r * cols + c);
        let label = |names: &mut obj::Names, r: usize, c: usize| match cell(r, c) {
            None => Some(String::new()),
            Some(Cell::Text(t)) => text(names, t, MAX_CELL_TEXT),
            Some(Cell::Num(_)) => None,
        };
        // The corner of a table with both label row and label column holds nothing.
        let corner_empty = match cell(0, 0) {
            None => true,
            Some(Cell::Text(t)) => t.is_empty(),
            Some(Cell::Num(_)) => false,
        };
        if first_row == 1 && first_col == 1 && !corner_empty {
            return None;
        }
        spec.series = if first_row == 1 { (first_col..cols).map(|c| label(names, 0, c)).collect::<Option<_>>()? } else { vec![] };
        spec.categories = if first_col == 1 { (first_row..rows).map(|r| label(names, r, 0)).collect::<Option<_>>()? } else { vec![] };
        let mut data = Vec::with_capacity(rows - first_row);
        for r in first_row..rows {
            let row = (first_col..cols)
                .map(|c| match cell(r, c) {
                    None => Some(None),
                    Some(Cell::Text(t)) if t.is_empty() => Some(None),
                    Some(Cell::Text(_)) => None,
                    Some(Cell::Num(v)) => Some(Some(*v)),
                })
                .collect::<Option<Vec<_>>>()?;
            data.push(row);
        }
        data.iter().flatten().any(Option::is_some).then_some(())?;
        spec.set_cells(data);
        Some(spec)
    }

    /// The series index of each group `Go` tagged as one series, by id: every series once, its
    /// columns counted in the table or among the series alike. `None` when the tags are missing,
    /// repeated or out of range: the series' paint is read from its group.
    fn series_tags(&self) -> Option<HashMap<NodeId, u32>> {
        let [_, cols, _, first_col] = self.size?;
        let n = cols - first_col;
        let ids: HashSet<NodeId> = self.series.iter().map(|(id, _)| *id).collect();
        (ids.len() == self.series.len() && self.series.len() == n).then_some(())?;
        let indexes = |offset: usize| -> Option<HashMap<NodeId, u32>> {
            let mut seen = HashSet::new();
            let tags: HashMap<NodeId, u32> = self
                .series
                .iter()
                .map(|(id, col)| {
                    let s = col.checked_sub(offset).filter(|s| *s < n && seen.insert(*s))?;
                    Some((*id, u32::try_from(s).ok()?))
                })
                .collect::<Option<_>>()?;
            Some(tags)
        };
        // With a label column, the columns read as the table's or as the series' are told apart by the
        // set they make; without one, they are the same.
        match (indexes(0), indexes(first_col)) {
            (Some(a), Some(b)) if a == b => Some(a),
            (Some(a), None) | (None, Some(a)) => Some(a),
            _ => None,
        }
    }
}

/// Whether `art` draws the plot `plot`'s left edge and its bottom edge (the document's y runs down)
/// as straight stroked lines: the value axis and the category axis, drawn on the plot's edges.
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
        // Only shown objects draw the axes, and a path only with a stroke.
        if !n.visible || n.opacity <= 0.0 || (left && bottom) {
            continue;
        }
        let stroked = n.appearance.items.iter().any(|it| !it.is_fill() && it.visible() && it.opacity() > 0.0 && !it.paint().is_none());
        if let NodeKind::Path { path, .. } = &n.kind
            && stroked
        {
            for s in path.to_bezpath().segments() {
                left |= on(&s, top_left, bottom_left);
                bottom |= on(&s, bottom_left, bottom_right);
            }
        }
        stack.extend(n.children().into_iter().flatten().map(|c| &**c));
    }
    left && bottom
}

/// Give the groups among `n`'s children that `tags` holds their series index.
fn tag_series(n: &mut Node, tags: &HashMap<NodeId, u32>) {
    for c in n.children_mut().into_iter().flatten() {
        if let Some(i) = tags.get(&c.id) {
            Arc::make_mut(c).extras_mut().series_index = Some(*i);
        }
    }
}

/// Whether every node `tags` holds is a group among `n`'s children: the graph draws its series
/// groups again as its own children.
fn tags_groups(n: &Node, tags: &HashMap<NodeId, u32>) -> bool {
    let found = n.children().into_iter().flatten().filter(|c| tags.contains_key(&c.id) && matches!(c.kind, NodeKind::Group { .. })).count();
    found == tags.len()
}

/// The paint of the shown fill (`fill`) or stroke of `p`, with a stroke's weight; `None` when it
/// has none. `Err` when it has more than one, or that fill or stroke has an opacity, a blend mode
/// or effects of its own, which a series' paint doesn't hold.
fn shown(p: &Node, fill: bool) -> Result<Option<(Paint, f64)>, ()> {
    let mut items = p.appearance.items.iter().filter(|it| it.is_fill() == fill && it.visible());
    let Some(item) = items.next() else { return Ok(None) };
    if items.next().is_some() || item.opacity() != 1.0 || item.blend() != BlendMode::Normal || !item.effects().is_empty() {
        return Err(());
    }
    if item.paint().is_none() {
        return Ok(None);
    }
    let width = match item {
        AppearanceItem::Stroke(s) => s.width,
        AppearanceItem::Fill(_) => 0.0,
    };
    Ok(Some((item.paint().clone(), width)))
}

/// Whether `n` is shown, fully opaque, in Normal mode, not isolated and without an opacity mask:
/// the graph draws its series groups and their marks so.
fn as_drawn(n: &Node) -> bool {
    n.visible && n.opacity == 1.0 && n.blend == BlendMode::Normal && !n.isolate && n.extras().mask.is_none()
}

/// Each series' fill and stroke, read from its group (`tags`), which holds paths only, one at
/// least: the paths with a fill share it, and every path has the same stroke or none. Only a line
/// graph (`line`) has paths without a fill: its lines. `None` when they differ, or the group, its
/// own appearance or a path is drawn otherwise than [`as_drawn`].
fn series_paints(art: &Node, tags: &HashMap<NodeId, u32>, line: bool) -> Option<Vec<SeriesPaint>> {
    let mut paints = vec![SeriesPaint::default(); tags.len()];
    for group in art.children().into_iter().flatten() {
        let Some(slot) = tags.get(&group.id).and_then(|i| paints.get_mut(usize::try_from(*i).ok()?)) else { continue };
        (as_drawn(group) && *group.appearance == Appearance::default()).then_some(())?;
        let paths: Vec<&Arc<Node>> = group.children().into_iter().flatten().collect();
        (!paths.is_empty() && paths.iter().all(|p| matches!(p.kind, NodeKind::Path { .. }))).then_some(())?;
        let (mut fills, mut strokes) = (vec![], vec![]);
        for p in &paths {
            as_drawn(p).then_some(())?;
            fills.extend(shown(p, true).ok()?);
            strokes.push(shown(p, false).ok()?);
        }
        let fill = fills.first().map(|(paint, _)| paint.clone());
        fills.iter().all(|(paint, _)| Some(paint) == fill.as_ref()).then_some(())?;
        (line || fills.len() == paths.len()).then_some(())?;
        let stroke = strokes.first().cloned().flatten();
        strokes.iter().all(|s| *s == stroke).then_some(())?;
        slot.fill = fill;
        (slot.stroke, slot.stroke_width) = match stroke {
            Some((paint, width)) => (Some(paint), Some(width)),
            None => (Some(Paint::None), None),
        };
    }
    Some(paints)
}

impl<'a> Reader<'a> {
    /// A graph's records: `%_Gs` starts the graph on a layer, `GS` ends it, and its data and settings
    /// (the operators in [`GRAPH_OPS`], and any operator on a `%_` line between `Gt` and `GT`) are
    /// taken with their operands `vals`. Its drawn art, and anything else, is read as usual. `true`
    /// when `w` was one of them. `ended`: whether a group and a mesh had just ended before `w`, which
    /// a `Go` tag after them leaves so.
    pub(super) fn graph_record(&mut self, w: &str, vals: &[V], hidden: bool, ended: (bool, bool)) -> bool {
        let depth = self.frames.len();
        let Some(g) = self.graph.as_mut() else {
            // Outside a graph, its operators stay ones the reader doesn't know.
            if w == "Gs" && hidden && !self.frames.is_empty() {
                let siblings = self.frames.last().map_or(0, |f| f.children.len());
                // A graph is live only where its art is in the document's space, in a layer or a group.
                let placed = self.pattern.is_none()
                    && self.symbol.is_none()
                    && vals.is_empty()
                    && matches!(self.frames.last().map(|f| &f.kind), Some(Kind::Layer(_) | Kind::Group | Kind::Clip));
                self.graph = Some(Graph { settings: false, frames: depth, siblings, live: placed.then(Box::default) });
                return true;
            }
            return false;
        };
        // The frame the graph started in closed: its art isn't where it was.
        if depth < g.frames {
            self.graph_plain();
        }
        let Some(g) = self.graph.as_mut() else { return false };
        let stage = g.live.as_ref().map(|r| r.stage);
        match w {
            // A graph doesn't start inside another: the one being read goes on.
            "Gs" => {
                self.graph_plain();
                self.warn(GRAPH_UNENDED);
            }
            "GS" => self.graph_end(vals.is_empty() && hidden),
            "Gt" | "GT" => {
                g.settings = w == "Gt";
                self.graph_step(w, vals, hidden);
            }
            "Go" => {
                self.graph_tag(vals, hidden);
                (self.group_ended, self.mesh_ended) = ended;
            }
            _ if GRAPH_OPS.contains(&w) => self.graph_step(w, vals, hidden),
            _ if hidden && g.settings => self.graph_plain(),
            _ => {
                // Before its table ends, a graph has only its records.
                if !matches!(stage, None | Some(Stage::Done)) {
                    self.graph_plain();
                }
                return false;
            }
        }
        true
    }

    /// The graph being read can't be live: its art stays plain, with a note.
    pub(super) fn graph_plain(&mut self) {
        if let Some(g) = self.graph.as_mut() {
            g.live = None;
        }
        self.warn(GRAPH_PLAIN);
    }

    /// Take a functional record or customization on a `%_` line.
    fn graph_step(&mut self, w: &str, vals: &[V], hidden: bool) {
        if self.graph.as_ref().is_none_or(|g| g.live.is_none()) {
            return;
        }
        let to_doc = if w == "Gb" { self.space() } else { Affine::IDENTITY };
        let ok = hidden && self.graph.as_mut().and_then(|g| g.live.as_mut()).and_then(|r| r.step(w, vals, to_doc)).is_some();
        if !ok {
            self.graph_plain();
        }
    }

    /// `target column row whichAxis Go` after an object of the drawn art: a group tagged as one
    /// series is kept with the column it names.
    fn graph_tag(&mut self, vals: &[V], hidden: bool) {
        let depth = self.frames.len();
        let last = self
            .after_object
            .then(|| self.frames.last().and_then(|f| f.children.last()))
            .flatten()
            .map(|n| (n.id, matches!(n.kind, NodeKind::Group { .. })));
        let Some(g) = self.graph.as_mut() else { return };
        let inside = depth > g.frames;
        let Some(r) = g.live.as_mut() else { return };
        let tag = match vals {
            [target, column, row, axis] if hidden && r.stage == Stage::Done => {
                let (row, axis) = (int(row), int(axis));
                // A series is the whole column, measured on the left axis (2) or with no axis given (0):
                // the model keeps no series on a row or on the right axis.
                let whole_column = row == Some(0) && matches!(axis, Some(0 | 2));
                int(target).filter(|t| TARGETS.contains(t) && row.is_some() && axis.is_some()).zip(int(column)).map(|(t, c)| (t, c, whole_column))
            }
            _ => None,
        };
        let ok = match (tag, last) {
            (Some((SERIES, column, true)), Some((id, true))) if inside => usize::try_from(column).ok().map(|c| r.series.push((id, c))).is_some(),
            (Some((SERIES, ..)), _) => false,
            (Some(_), Some(_)) => true,
            _ => false,
        };
        if !ok {
            self.graph_plain();
        }
    }

    /// `GS`: the graph ends. Its drawn art is the one object its frame gained, a group; it keeps the
    /// graph's data and settings when they read whole (`whole`) and the art is registered on its plot.
    fn graph_end(&mut self, whole: bool) {
        let Some(g) = self.graph.take() else { return };
        let depth = self.frames.len();
        let live = g.live.filter(|r| whole && r.stage == Stage::Done && depth == g.frames);
        let spec = live.as_ref().and_then(|r| r.finish(&mut self.names).zip(r.series_tags()));
        let attached = spec.is_some_and(|(mut spec, tags)| {
            let Some(art) = self.frames.last_mut().filter(|f| f.children.len() == g.siblings + 1).and_then(|f| f.children.last_mut()) else {
                return false;
            };
            if !matches!(art.kind, NodeKind::Group { clip: false, .. }) || !tags_groups(art, &tags) || !draws_axes(art, spec.rect) {
                return false;
            }
            let Some(placed) = art.geometric_bounds().filter(|b| b.width() > 0.0 && b.height() > 0.0) else { return false };
            let r = spec.rect;
            if r.x0 < placed.x0 - EDGE || r.y0 < placed.y0 - EDGE || r.x1 > placed.x1 + EDGE || r.y1 > placed.y1 + EDGE {
                return false;
            }
            let Some(paints) = series_paints(art, &tags, spec.kind == GraphKind::Line) else { return false };
            spec.series_paints = paints;
            spec.placed = Some(placed);
            tag_series(art, &tags);
            art.extras_mut().graph = Some(Box::new(spec));
            true
        });
        if !attached {
            self.warn(GRAPH_PLAIN);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_color::Color;
    use vectorcraft_geom::shapes;

    /// A graph's art: one series group (id 2) holding one mark painted `ap`, the group changed by
    /// `group`.
    fn art(ap: Appearance, group: impl Fn(&mut Node)) -> Node {
        let mark = Arc::new(Node::path(NodeId(3), shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0)), ap));
        let mut series = Node::group(NodeId(2), vec![mark]);
        group(&mut series);
        Node::group(NodeId(1), vec![Arc::new(series)])
    }

    /// A series whose mark has two fills has no one paint, so the graph can't draw it again.
    #[test]
    fn a_series_mark_with_two_fills_has_no_series_paint() {
        let red = Paint::solid(Color::rgb(1.0, 0.0, 0.0));
        let one = Appearance::basic(red.clone(), Paint::None, 0.0);
        let mut two = one.clone();
        two.items.extend(one.items.iter().find(|i| i.is_fill()).cloned());
        let tags = HashMap::from([(NodeId(2), 0)]);
        let kept = series_paints(&art(one, |_| {}), &tags, false).expect("one fill");
        assert_eq!((kept[0].fill.as_ref(), kept[0].stroke.as_ref()), (Some(&red), Some(&Paint::None)));
        assert!(series_paints(&art(two, |_| {}), &tags, false).is_none());
    }

    /// A series group that is hidden, see-through or in another blending mode is not drawn again so.
    #[test]
    fn a_series_group_drawn_otherwise_has_no_series_paint() {
        let ap = || Appearance::basic(Paint::solid(Color::rgb(1.0, 0.0, 0.0)), Paint::None, 0.0);
        let tags = HashMap::from([(NodeId(2), 0)]);
        assert!(series_paints(&art(ap(), |g| g.visible = false), &tags, false).is_none(), "hidden");
        assert!(series_paints(&art(ap(), |g| g.opacity = 0.5), &tags, false).is_none(), "50% opaque");
        assert!(series_paints(&art(ap(), |g| g.blend = BlendMode::Multiply), &tags, false).is_none(), "Multiply");
    }
}
