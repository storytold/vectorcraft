use super::axis::{LABEL_SIZE, MAX_CATEGORY_TICKS, MAX_TICK_MARKS, one_line, tick_values, value_axis};
use super::columns::{Allowance, ColumnArt, column_art, column_designs};
use super::data::fmt_value;
use super::labels::{
    FILLED_LINE_BUDGET, Gen, Role, SHADOW_BUDGET, SHADOW_OPACITY, blank_label, label_size, label_width, polyline, recolour_text, runs, shadow_of,
};
use super::markers::{MARKER_BUDGET, marker_cost, marker_designs};
use super::series::series_marks;
use super::{
    Arc, BezPath, Color, Document, GraphKind, GraphSpec, Justify, Node, NodeKind, Paint, PathData, Point, Rect, TickLength, ValueAxisSide, shapes,
};

/// Build the graph's children: with each series' column design (Object › Graph › Column) when the document has it,
/// and with plain columns for all when the designs would draw more than COLUMN_BUDGET objects.
pub(super) fn generate(d: &mut Document, g: &GraphSpec) -> Vec<Arc<Node>> {
    let column_looks = column_designs(d, g);
    match generate_with(d, g, &column_looks) {
        Some(out) => out,
        None => generate_with(d, g, &[]).unwrap_or_default(),
    }
}

/// [`generate`] with the column designs `column_looks`; `None` when they would draw more than COLUMN_BUDGET objects.
pub(super) fn generate_with(d: &mut Document, g: &GraphSpec, column_looks: &[Option<ColumnArt>]) -> Option<Vec<Arc<Node>>> {
    let mut p = Plot::new(d, g, column_looks);
    match g.kind {
        GraphKind::Pie => p.pie(),
        GraphKind::Radar => p.radar(),
        _ => p.cartesian(),
    }
    let labels = p.legend();
    p.finish(labels)
}

/// The `s`th entry of a per-series list of optional designs.
fn nth<T>(list: &[Option<T>], s: usize) -> Option<&T> {
    list.get(s).and_then(Option::as_ref)
}

/// One graph being generated: its data, the nodes drawn so far and what the legend and shadows still need.
struct Plot<'a> {
    g: &'a GraphSpec,
    b: Gen<'a>,
    column_looks: &'a [Option<ColumnArt>],
    /// Each series' marker design (Object › Graph › Marker), when the document has it; default squares for all
    /// when the designs would draw more than MARKER_BUDGET objects.
    designs: Vec<Option<Arc<Node>>>,
    r: Rect,
    nser: usize,
    ncat: usize,
    /// The labels' sizes (9 pt unless formatted), which their spacing follows.
    vs: f64,
    cs: f64,
    ls: f64,
    /// A blank cell (no value) is `None`: columns, bars and stacked segments leave it out, lines and scatter points
    /// break around it and the value axis ignores it. Area, pie and radar draw it as 0, which is what it adds to them.
    cells: Vec<Vec<Option<f64>>>,
    /// Each series' own type (Combine different graph types); one type for the whole graph unless a series has its own.
    kinds: Vec<GraphKind>,
    horizontal: bool,
    /// Where the art right of the plot ends (the right value axis' labels), so the legend starts after it.
    right_edge: f64,
    /// Labels drawn over the series: a pie's Legends in Wedges (in the Legend group) and stacked pies' names.
    wedge_labels: Vec<Arc<Node>>,
    on_top: Vec<Arc<Node>>,
    /// Draw Filled Lines, up to FILLED_LINE_BUDGET data points (the outlines are rebuilt on every change).
    filled: bool,
    /// Where columns and bars reach (past the plot when their widths are over 100%), so the legend keeps clear.
    bars_reach: Option<Rect>,
    /// Add Drop Shadow: the columns, bars, lines and wedges to cast one.
    shadowed: Vec<(usize, Arc<Node>)>,
    series: Vec<Vec<Arc<Node>>>,
}

/// A pie (or the ring of a stacked pie) to draw.
#[derive(Clone, Copy)]
struct Ring {
    centre: Point,
    rad: f64,
    hole: f64,
}

/// The geometry of a plot with axes: the value scales and where the categories sit.
struct Geo {
    r: Rect,
    horizontal: bool,
    ncat: usize,
    /// Which series use the right axis' own scale.
    on_right: Vec<bool>,
    left: (f64, f64, f64),
    right: (f64, f64, f64),
    cat_w: f64,
    label_edges: bool,
    left_side: bool,
    right_side: bool,
}

impl Geo {
    fn axis(&self, s: usize) -> usize {
        usize::from(self.on_right.get(s).copied().unwrap_or(false))
    }

    fn scale(&self, a: usize) -> (f64, f64, f64) {
        if a == 1 { self.right } else { self.left }
    }

    /// Value → coordinate along value axis `a` (y for columns/lines, x for bars).
    fn vpos(&self, a: usize, v: f64) -> f64 {
        let (lo, hi, _) = self.scale(a);
        let t = (v - lo) / (hi - lo);
        if self.horizontal { self.r.x0 + t * self.r.width() } else { self.r.y1 - t * self.r.height() }
    }

    /// That axis' zero (or the end nearest it).
    fn zero(&self, a: usize) -> f64 {
        let (lo, hi, _) = self.scale(a);
        self.vpos(a, 0.0f64.clamp(lo, hi))
    }

    /// Plot points per unit of value along value axis `a`.
    fn per_unit(&self, a: usize) -> f64 {
        let (lo, hi, _) = self.scale(a);
        (if self.horizontal { self.r.width() } else { self.r.height() }) / (hi - lo)
    }

    fn cat_start(&self, c: usize) -> f64 {
        if self.horizontal { self.r.y0 + c as f64 * self.cat_w } else { self.r.x0 + c as f64 * self.cat_w }
    }

    fn mid(&self, c: usize, edges: bool) -> f64 {
        let r = self.r;
        if edges && self.ncat > 1 { r.x0 + r.width() * c as f64 / (self.ncat - 1) as f64 } else { self.cat_start(c) + self.cat_w / 2.0 }
    }

    fn cat_mid(&self, c: usize) -> f64 {
        self.mid(c, self.label_edges)
    }

    /// The ends of a value tick mark at coordinate `q` of axis `a`.
    fn tick_ends(&self, len: TickLength, a: usize, q: f64) -> Option<(Point, Point)> {
        let r = self.r;
        match (len, self.horizontal, a == 1) {
            (TickLength::None, ..) => None,
            (TickLength::Short, true, _) => Some((Point::new(q, r.y1), Point::new(q, r.y1 + 4.0))),
            (TickLength::Short, false, false) => Some((Point::new(r.x0 - 4.0, q), Point::new(r.x0, q))),
            (TickLength::Short, false, true) => Some((Point::new(r.x1, q), Point::new(r.x1 + 4.0, q))),
            (TickLength::Full, true, _) => Some((Point::new(q, r.y0), Point::new(q, r.y1))),
            (TickLength::Full, false, _) => Some((Point::new(r.x0, q), Point::new(r.x1, q))),
        }
    }
}

/// How columns (or bars) and stacks share a category's width.
struct Cluster {
    /// Columns of one category side by side; stacked columns take one slot of that cluster, or, with no plain
    /// columns, a width of their own.
    columns: Vec<usize>,
    stacks: Vec<usize>,
    /// One stack per value axis that has stacked series, each in a slot of its own.
    stack_axes: Vec<usize>,
    own_width: bool,
    /// The widths past 100% make the columns (or clusters) overlap.
    col_frac: f64,
    clus_frac: f64,
    cluster: f64,
    slot: f64,
    bar: f64,
}

impl Cluster {
    /// Where a stack on axis `ax` starts across the category `c` (starting at `c0`), and how wide it is.
    fn place(&self, geo: &Geo, c: usize, c0: f64, ax: usize) -> (f64, f64) {
        if self.own_width {
            // Up to the category's width, or past it when the widths are set past 100%.
            let cat_w = geo.cat_w;
            let w = (cat_w * self.col_frac * self.clus_frac * 1.2).min(cat_w * (self.col_frac * self.clus_frac).max(1.0));
            (geo.cat_start(c) + (cat_w - w) / 2.0, w)
        } else {
            let i = self.columns.len() + self.stack_axes.iter().position(|a| *a == ax).unwrap_or(0);
            (c0 + self.slot * i as f64 + (self.slot - self.bar) / 2.0, self.bar)
        }
    }
}

/// A column, bar or stacked segment to draw.
struct Bar {
    s: usize,
    rect: Rect,
    value: f64,
    /// It grows towards the axis' start.
    down: bool,
    per_unit: f64,
}

fn bar_rect(horizontal: bool, a: f64, w: f64, v0: f64, v1: f64) -> Rect {
    if horizontal { Rect::new(v0.min(v1), a, v0.max(v1), a + w) } else { Rect::new(a, v0.min(v1), a + w, v0.max(v1)) }
}

/// White text on a dark wedge.
fn is_dark(fill: &Paint) -> bool {
    fill.color().is_some_and(|c| {
        let [r, g, b] = c.to_rgb();
        0.2126 * r + 0.7152 * g + 0.0722 * b < 0.5
    })
}

fn dir(a: f64) -> vectorcraft_geom::Vec2 {
    vectorcraft_geom::Vec2::new(a.cos(), a.sin())
}

/// A wedge of `ring` from angle `a0` over `sweep`; a ring segment when the ring has a hole.
fn wedge_path(ring: Ring, a0: f64, sweep: f64) -> BezPath {
    let Ring { centre, rad, hole } = ring;
    let mut bp = BezPath::new();
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
    bp
}

/// The radius of the pie with `total`: one size (Even) or sized by the totals (Ratio, by area).
fn pie_radius(g: &GraphSpec, full: f64, biggest: f64, total: f64) -> f64 {
    match g.pie_position {
        vectorcraft_doc::PiePosition::Even => full,
        _ if biggest > 0.0 && biggest.is_finite() => (full * (total / biggest).sqrt()).max(1.0),
        _ => full,
    }
}

impl<'a> Plot<'a> {
    fn new(d: &'a mut Document, g: &'a GraphSpec, column_looks: &'a [Option<ColumnArt>]) -> Self {
        let mut designs = marker_designs(d, g);
        if marker_cost(g, &designs) > MARKER_BUDGET {
            designs.clear();
        }
        let nser = g.rows.iter().map(Vec::len).max().unwrap_or(0).max(1);
        let kinds: Vec<GraphKind> = (0..nser).map(|s| g.series_kind(s)).collect();
        Plot {
            g,
            b: Gen { d, out: vec![], styles: &g.label_styles, columns: Allowance::new() },
            column_looks,
            designs,
            r: g.rect,
            nser,
            ncat: g.rows.len().max(1),
            vs: label_size(&g.label_styles, Role::Value),
            cs: label_size(&g.label_styles, Role::Category),
            ls: label_size(&g.label_styles, Role::Legend),
            cells: g.cells(),
            kinds,
            horizontal: matches!(g.kind, GraphKind::Bar | GraphKind::StackedBar),
            right_edge: g.rect.x1,
            wedge_labels: vec![],
            on_top: vec![],
            filled: g.filled_lines && g.rows.len().saturating_mul(nser) <= FILLED_LINE_BUDGET,
            bars_reach: None,
            shadowed: vec![],
            series: vec![vec![]; nser],
        }
    }

    fn cell(&self, c: usize, s: usize) -> Option<f64> {
        self.cells.get(c).and_then(|r| r.get(s)).copied().flatten()
    }

    fn val(&self, c: usize, s: usize) -> f64 {
        self.cell(c, s).unwrap_or(0.0)
    }

    fn kind(&self, s: usize) -> GraphKind {
        self.kinds.get(s).copied().unwrap_or(self.g.kind)
    }

    /// Series drawn as columns or bars, which column designs apply to.
    fn columnar(&self, s: usize) -> bool {
        matches!(self.kind(s), GraphKind::Column | GraphKind::StackedColumn | GraphKind::Bar | GraphKind::StackedBar)
    }

    /// The series drawn as `kind`.
    fn series_of(&self, kind: GraphKind) -> Vec<usize> {
        (0..self.nser).filter(|s| self.kind(*s) == kind).collect()
    }

    fn mark(&self, i: usize, series_fill: bool, stroke: Paint, width: f64) -> (Paint, Paint, f64) {
        series_marks(self.g, i, series_fill, stroke, width)
    }

    fn series_label(&self, i: usize) -> String {
        self.g.series.get(i).cloned().unwrap_or_default()
    }

    fn push_series(&mut self, s: usize, n: Arc<Node>) {
        if let Some(items) = self.series.get_mut(s) {
            items.push(n);
        }
    }

    /// Add `n` to series `s` and, for the drop shadow, to the shadowed marks.
    fn push_shadowed(&mut self, s: usize, n: Arc<Node>) {
        self.shadowed.push((s, n.clone()));
        self.push_series(s, n);
    }

    fn reach(&mut self, rect: Rect) {
        self.bars_reach = Some(self.bars_reach.map_or(rect, |r| r.union(rect)));
    }

    // ---- Pie ----

    /// One pie per row, a wedge per series, clockwise from 12 o'clock. Position: side by side at one size
    /// (Even) or sized by their totals (Ratio, by area), or stacked on one centre, the largest at the back.
    fn pie(&mut self) {
        let (g, r, ncat, nser) = (self.g, self.r, self.ncat, self.nser);
        let totals: Vec<f64> = (0..ncat).map(|c| (0..nser).map(|s| self.val(c, s).max(0.0)).sum()).collect();
        let biggest = totals.iter().copied().fold(0.0, f64::max);
        let stacked = g.pie_position == vectorcraft_doc::PiePosition::Stacked;
        let w = if stacked { r.width() } else { r.width() / ncat as f64 };
        let full = (w.min(r.height()) / 2.0 * 0.9).max(1.0);
        let radius = |total: f64| pie_radius(g, full, biggest, total);
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
            let ring = Ring { centre, rad, hole };
            self.pie_wedges(c, ring, total);
            self.pie_name(c, ring, stacked);
        }
    }

    /// Sort: data order, largest first in each pie, or the first pie's largest-first order everywhere.
    fn pie_order(&self, c: usize) -> Vec<usize> {
        let mut o: Vec<usize> = (0..self.nser).collect();
        let by = match self.g.pie_sort {
            vectorcraft_doc::PieSort::None => return o,
            vectorcraft_doc::PieSort::All => c,
            vectorcraft_doc::PieSort::First => 0,
        };
        o.sort_by(|a, b| self.val(by, *b).max(0.0).total_cmp(&self.val(by, *a).max(0.0)));
        o
    }

    fn pie_wedges(&mut self, c: usize, ring: Ring, total: f64) {
        let mut a0 = -std::f64::consts::FRAC_PI_2;
        for s in self.pie_order(c) {
            let v = self.val(c, s).max(0.0);
            if total <= 0.0 || v <= 0.0 {
                continue;
            }
            let sweep = v / total * std::f64::consts::TAU;
            let bp = wedge_path(ring, a0, sweep);
            let (fill, stroke, sw) = self.mark(s, true, Paint::solid(Color::WHITE), 0.5);
            let dark = is_dark(&fill);
            let n = self.b.path(PathData::from_bezpath(&bp), fill, stroke, sw);
            self.push_shadowed(s, n);
            // Legends in Wedges: the series label inside its wedge, centred two thirds of the way out.
            if self.g.legend && self.g.pie_legend_in_wedges {
                self.wedge_label(s, ring, a0 + sweep / 2.0, dark);
            }
            a0 += sweep;
        }
    }

    fn wedge_label(&mut self, s: usize, ring: Ring, mid: f64, dark: bool) {
        let label = self.series_label(s);
        if blank_label(&label) {
            return;
        }
        // Two thirds of the way out (of the ring, for a stacked pie).
        let at = ring.centre + dir(mid) * (ring.hole + (ring.rad - ring.hole) * 0.62);
        let t = self.b.label(Role::Legend, Point::new(at.x, at.y + self.ls * 0.35), &label, Justify::Center, true);
        // White on a dark wedge, unless the legend labels were given a colour of their own.
        let own_colour = self.g.label_styles.legend.as_ref().is_some_and(|st| st.fill != Paint::solid(Color::BLACK));
        self.wedge_labels.push(if dark && !own_colour { recolour_text(&t, Color::WHITE) } else { t });
    }

    /// The pie's category name.
    fn pie_name(&mut self, c: usize, ring: Ring, stacked: bool) {
        let Some(cat) = self.g.categories.get(c).filter(|c| !c.is_empty()) else { return };
        let (centre, rad, cs) = (ring.centre, ring.rad, self.cs);
        if stacked {
            // Just above its own outline, all its lines, and drawn over the pies.
            let lines = cat.split('|').count() as f64;
            let at = Point::new(centre.x, centre.y - rad - cs * 0.4 - (lines - 1.0) * cs * 1.2);
            let t = self.b.label(Role::Category, at, cat, Justify::Center, false);
            self.on_top.push(t);
        } else {
            let t = self.b.label(Role::Category, Point::new(centre.x, centre.y + rad + cs * 1.6), cat, Justify::Center, false);
            self.b.out.push(t);
        }
    }

    // ---- Radar ----

    fn radar(&mut self) {
        let (g, r, ncat, cs) = (self.g, self.r, self.ncat, self.cs);
        let centre = r.center();
        let rad = (r.width().min(r.height()) / 2.0).max(1.0);
        let (lo, hi, step) = value_axis(g.ticks, g.axis_min, g.axis_max, 0.0, g.rows.iter().flatten().copied().fold(0.0, f64::max));
        let at = |c: usize, v: f64| {
            let a = -std::f64::consts::FRAC_PI_2 + std::f64::consts::TAU * c as f64 / ncat as f64;
            centre + vectorcraft_geom::Vec2::new(a.cos(), a.sin()) * (rad * ((v - lo) / (hi - lo)).clamp(0.0, 1.0))
        };
        let mut axes = vec![];
        for c in 0..ncat {
            axes.push(self.b.line(centre, at(c, hi)));
            if let Some(cat) = g.categories.get(c).filter(|c| !c.is_empty()) {
                let p = at(c, hi) + (at(c, hi) - centre).normalize() * (cs * 1.2);
                axes.push(self.b.label(Role::Category, Point::new(p.x, p.y + cs * 0.35), cat, Justify::Center, true));
            }
        }
        for v in tick_values(lo, hi, step).into_iter().skip(1) {
            let ring: Vec<Point> = (0..ncat).map(|c| at(c, v)).collect();
            axes.push(self.b.path(polyline(&ring, true), Paint::None, Paint::solid(Color::rgb(0.6, 0.6, 0.6)), 0.25));
        }
        let ax = self.b.group("Axes", axes);
        self.b.out.push(ax);
        for s in 0..self.series.len() {
            let pts: Vec<Point> = (0..ncat).map(|c| at(c, self.val(c, s))).collect();
            if g.connect_points && pts.len() > 1 {
                let n = self.b.data_line(g, s, polyline(&pts, true), self.filled);
                self.push_shadowed(s, n);
            }
            if g.mark_points {
                for p in pts {
                    let paint = self.mark(s, true, Paint::None, 0.0);
                    let n = self.b.marker(nth(&self.designs, s), Rect::from_center_size(p, (4.0, 4.0)), paint);
                    self.push_series(s, n);
                }
            }
        }
    }

    // ---- Columns, bars, lines, areas and scatter ----

    fn cartesian(&mut self) {
        let g = self.g;
        let scatter = g.kind == GraphKind::Scatter;
        // Separate Scales: with the value axis on both sides, the series assigned to the right axis get a scale
        // of their own there (axis 1); every other series, and every series without separate scales, uses axis 0.
        let separate = g.separate_scales && g.value_axis == ValueAxisSide::Both && !self.horizontal && !scatter;
        let mut on_right = vec![false; self.nser];
        if separate {
            for i in &g.right_series {
                if let Some(x) = on_right.get_mut(*i) {
                    *x = true;
                }
            }
        }
        let mut geo = self.geo(on_right);
        let (lo, hi, xlo, xhi) = self.value_range(&geo, scatter);
        // An axis with no series of its own shows the other one's scale.
        let left = value_axis(g.ticks, g.axis_min, g.axis_max, lo[0], hi[0]);
        let used = |a: usize| (0..self.nser).any(|s| geo.axis(s) == a);
        let (left, right) = if !separate || !used(1) {
            (left, left)
        } else {
            let right = value_axis(g.right_ticks, g.right_axis_min, g.right_axis_max, lo[1], hi[1]);
            if used(0) { (left, right) } else { (right, right) }
        };
        (geo.left, geo.right) = (left, right);
        let mut axes = vec![];
        self.value_axes(&geo, &mut axes);
        if scatter {
            self.scatter(&geo, (xlo, xhi), &mut axes);
        } else {
            self.category_axis(&geo, &mut axes);
        }
        let ax = self.b.group("Axes", axes);
        self.b.out.push(ax);
        self.columns(&geo);
        self.areas(&geo);
        self.lines(&geo);
    }

    /// The plot's geometry, its value scales still to come.
    fn geo(&self, on_right: Vec<bool>) -> Geo {
        let (g, r) = (self.g, self.r);
        // Value axes with ticks and labels: along the bottom for bar graphs, else on the chosen sides (the right
        // one shows the right scale).
        let (left_side, right_side) =
            if self.horizontal { (true, false) } else { (g.value_axis != ValueAxisSide::Right, g.value_axis != ValueAxisSide::Left) };
        let span = if self.horizontal { r.height() } else { r.width() };
        // Area bands always span the plot; lines only with Edge-to-Edge Lines. Category labels sit where every
        // series puts its points: at the edges when all of them span the plot, else at the category centres.
        let spans = |k: GraphKind| k == GraphKind::Area || (k == GraphKind::Line && g.edge_to_edge);
        Geo {
            r,
            horizontal: self.horizontal,
            ncat: self.ncat,
            on_right,
            left: (0.0, 1.0, 1.0),
            right: (0.0, 1.0, 1.0),
            cat_w: span / self.ncat as f64,
            label_edges: self.kinds.iter().all(|k| spans(*k)),
            left_side,
            right_side,
        }
    }

    /// The value range per axis (stacked graphs stack positives and negatives separately), and a scatter's x range.
    fn value_range(&self, geo: &Geo, scatter: bool) -> ([f64; 2], [f64; 2], f64, f64) {
        let (mut lo, mut hi) = ([0.0f64; 2], [0.0f64; 2]);
        let (mut xlo, mut xhi) = (f64::MAX, f64::MIN);
        for c in 0..self.ncat {
            if !scatter {
                for (a, (lo, hi)) in lo.iter_mut().zip(hi.iter_mut()).enumerate() {
                    self.extend_range(geo, c, a, (lo, hi));
                }
            } else {
                for s in (0..self.nser).step_by(2) {
                    if let (Some(y), Some(x)) = (self.cell(c, s), self.cell(c, s + 1)) {
                        hi[0] = hi[0].max(y);
                        lo[0] = lo[0].min(y);
                        xlo = xlo.min(x);
                        xhi = xhi.max(x);
                    }
                }
            }
        }
        (lo, hi, xlo, xhi)
    }

    /// Widen the range of axis `a` by category `c`'s values.
    fn extend_range(&self, geo: &Geo, c: usize, a: usize, (lo, hi): (&mut f64, &mut f64)) {
        let nser = self.nser;
        // Stacked columns (or bars) stack together, and so do areas; other series count one value each.
        for stack in [[GraphKind::StackedColumn, GraphKind::StackedBar], [GraphKind::Area, GraphKind::Area]] {
            let members = || (0..nser).filter(|s| geo.axis(*s) == a && stack.contains(&self.kind(*s)));
            *hi = hi.max(members().map(|s| self.val(c, s).max(0.0)).sum());
            *lo = lo.min(members().map(|s| self.val(c, s).min(0.0)).sum());
        }
        let single = |s: &usize| geo.axis(*s) == a && matches!(self.kind(*s), GraphKind::Column | GraphKind::Bar | GraphKind::Line);
        for v in (0..nser).filter(single).filter_map(|s| self.cell(c, s)) {
            *hi = hi.max(v);
            *lo = lo.min(v);
        }
    }

    /// The value axes with ticks and labels, the legend's start after them, and the axis lines.
    fn value_axes(&mut self, geo: &Geo, axes: &mut Vec<Arc<Node>>) {
        let r = self.r;
        if geo.left_side {
            let (lo, hi, step) = geo.left;
            for v in tick_values(lo, hi, step) {
                self.value_tick(geo, axes, 0, v, (step, hi));
            }
        }
        if geo.right_side {
            let (lo, hi, step) = geo.right;
            for v in tick_values(lo, hi, step) {
                self.value_tick(geo, axes, 1, v, (step, hi));
            }
        }
        // The legend starts after the value labels right of the plot: the right axis' (14 points on), or a bar
        // graph's last bottom label running past the plot (6 points on; a short number stays within the gap).
        for t in axes.iter() {
            if matches!(t.kind, NodeKind::Text(_))
                && let Some(bb) = t.geometric_bounds()
                && bb.x1 > r.x1
            {
                self.right_edge = self.right_edge.max(if bb.x0 > r.x1 { bb.x1 } else { bb.x1 - 8.0 });
            }
        }
        if geo.horizontal {
            axes.push(self.b.line(Point::new(r.x0, r.y1), Point::new(r.x1, r.y1)));
            axes.push(self.b.line(Point::new(geo.zero(0), r.y0), Point::new(geo.zero(0), r.y1)));
        } else {
            if geo.left_side {
                axes.push(self.b.line(Point::new(r.x0, r.y0), Point::new(r.x0, r.y1)));
            }
            if geo.right_side {
                axes.push(self.b.line(Point::new(r.x1, r.y0), Point::new(r.x1, r.y1)));
            }
            axes.push(self.b.line(Point::new(r.x0, geo.zero(0)), Point::new(r.x1, geo.zero(0))));
            // The right scale's zero, where its columns start, when it isn't the left one's.
            if (geo.zero(1) - geo.zero(0)).abs() > 1e-9 {
                axes.push(self.b.line(Point::new(r.x0, geo.zero(1)), Point::new(r.x1, geo.zero(1))));
            }
        }
    }

    /// Each label's tick mark, then the extra tick marks of its division (Tick Marks per division).
    fn value_tick(&mut self, geo: &Geo, axes: &mut Vec<Arc<Node>>, a: usize, v: f64, (step, hi): (f64, f64)) {
        let g = self.g;
        let (len, marks) = if a == 1 { (g.right_tick_length, g.right_tick_marks) } else { (g.tick_length, g.tick_marks) };
        let n = marks.clamp(1, MAX_TICK_MARKS);
        for k in 0..n {
            let w = v + step * k as f64 / n as f64;
            if k > 0 && w >= hi - step * 1e-9 {
                break;
            }
            let q = geo.vpos(a, w);
            // A full-length tick on the zero line, or repeating the left side's on one shared scale, adds nothing.
            let repeat = a == 1 && geo.left_side && g.tick_length == TickLength::Full && geo.right == geo.left;
            let len = if len == TickLength::Full && ((q - geo.zero(a)).abs() < 1e-9 || repeat) { TickLength::None } else { len };
            if let Some((p0, p1)) = geo.tick_ends(len, a, q) {
                axes.push(self.b.line(p0, p1));
            }
            if k == 0 {
                axes.push(self.value_label(geo, a, q, v));
            }
        }
    }

    /// The label after a tick mark: with the Add Labels text of its axis, kept to one short line whatever the file holds.
    fn value_label(&mut self, geo: &Geo, a: usize, q: f64, v: f64) -> Arc<Node> {
        let (r, vs) = (self.r, self.vs);
        let (prefix, suffix) = if a == 1 { (&self.g.right_prefix, &self.g.right_suffix) } else { (&self.g.prefix, &self.g.suffix) };
        let label = format!("{}{}{}", one_line(prefix), fmt_value(v), one_line(suffix));
        if geo.horizontal {
            self.b.text(Role::Value, Point::new(q, r.y1 + 4.0 + vs * 1.1), &label, Justify::Center)
        } else if a == 1 {
            self.b.text(Role::Value, Point::new(r.x1 + 6.0, q + vs * 0.35), &label, Justify::Left)
        } else {
            self.b.text(Role::Value, Point::new(r.x0 - 6.0, q + vs * 0.35), &label, Justify::Right)
        }
    }

    /// Category labels and tick marks.
    fn category_axis(&mut self, geo: &Geo, axes: &mut Vec<Arc<Node>>) {
        let (r, cs) = (self.r, self.cs);
        for c in 0..self.ncat {
            if let Some(cat) = self.g.categories.get(c).filter(|c| !c.is_empty()) {
                let t = if geo.horizontal {
                    self.b.label(Role::Category, Point::new(r.x0 - 6.0, geo.cat_mid(c) + cs * 0.35), cat, Justify::Right, true)
                } else {
                    self.b.label(Role::Category, Point::new(geo.cat_mid(c), r.y1 + cs * 1.4), cat, Justify::Center, false)
                };
                axes.push(t);
            }
        }
        // Category tick marks (none by default): at the labels, or between them (at the category edges),
        // with the extra tick marks of each division between those.
        let len = self.g.category_tick_length;
        if len == TickLength::None {
            return;
        }
        let at = self.category_tick_positions(geo);
        // A full-length tick on a drawn axis line adds nothing.
        let on_axis_line = |q: f64| {
            if geo.horizontal {
                (q - r.y1).abs() < 1e-9
            } else {
                ((q - r.x0).abs() < 1e-9 && geo.left_side) || ((q - r.x1).abs() < 1e-9 && geo.right_side)
            }
        };
        for q in at.into_iter().take(MAX_CATEGORY_TICKS) {
            if len == TickLength::Full && on_axis_line(q) {
                continue;
            }
            let (p0, p1) = match (len, geo.horizontal) {
                (TickLength::Full, false) => (Point::new(q, r.y0), Point::new(q, r.y1)),
                (TickLength::Full, true) => (Point::new(r.x0, q), Point::new(r.x1, q)),
                (_, false) => (Point::new(q, r.y1), Point::new(q, r.y1 + 4.0)),
                (_, true) => (Point::new(r.x0 - 4.0, q), Point::new(r.x0, q)),
            };
            axes.push(self.b.line(p0, p1));
        }
    }

    /// Where the category tick marks go: at the labels, or between them, each division with its extra marks.
    fn category_tick_positions(&self, geo: &Geo) -> Vec<f64> {
        let shown = self.ncat.min(MAX_CATEGORY_TICKS);
        let base: Vec<f64> = match (self.g.ticks_between_labels, geo.label_edges) {
            // Labels at the plot's edges: halfway between neighbouring labels.
            (true, true) => (1..shown).map(|c| (geo.cat_mid(c - 1) + geo.cat_mid(c)) / 2.0).collect(),
            (true, false) => (0..=shown).map(|c| geo.cat_start(c)).collect(),
            (false, _) => (0..shown).map(|c| geo.cat_mid(c)).collect(),
        };
        // Fewer per division on a long axis, so the cap still ticks all of it.
        let n = self.g.category_tick_marks.clamp(1, MAX_TICK_MARKS).min(MAX_CATEGORY_TICKS / base.len().max(1)).max(1);
        base.iter()
            .enumerate()
            .flat_map(|(i, q)| {
                let next = base.get(i + 1).copied();
                (0..n).filter_map(move |k| if k == 0 { Some(*q) } else { next.map(|x| q + (x - q) * k as f64 / n as f64) })
            })
            .collect()
    }

    /// Scatter: a horizontal value axis too, and the series' points.
    fn scatter(&mut self, geo: &Geo, (xlo, xhi): (f64, f64), axes: &mut Vec<Arc<Node>>) {
        let (g, r, vs) = (self.g, self.r, self.vs);
        // Like the value axis: finite, and a counted (capped) list of ticks, so data near f64::MAX can't loop forever.
        let x_lo = if xlo == f64::MAX { 0.0 } else { xlo.min(0.0) };
        let x_hi = if xhi == f64::MIN { 1.0 } else { xhi };
        let (xl, xh, xs) = value_axis(g.ticks, None, None, x_lo, x_hi);
        for x in tick_values(xl, xh, xs) {
            let q = r.x0 + (x - xl) / (xh - xl) * r.width();
            axes.push(self.b.line(Point::new(q, r.y1), Point::new(q, r.y1 + 4.0)));
            axes.push(self.b.text(Role::Value, Point::new(q, r.y1 + 4.0 + vs * 1.1), &fmt_value(x), Justify::Center));
        }
        for (si, s) in (0..self.nser).step_by(2).enumerate() {
            let pts: Vec<Option<Point>> = (0..self.ncat)
                .map(|c| match (self.cell(c, s), self.cell(c, s + 1)) {
                    (Some(y), Some(x)) => Some(Point::new(r.x0 + (x - xl) / (xh - xl) * r.width(), geo.vpos(0, y))),
                    _ => None,
                })
                .collect();
            if g.connect_points {
                for run in runs(&pts) {
                    let n = self.b.data_line(g, si, polyline(&run, false), self.filled);
                    self.shadowed.push((si, n.clone()));
                    self.series[si].push(n);
                }
            }
            self.point_markers(si, &pts);
        }
    }

    /// A marker on each point of series `s`, when Mark Data Points is on.
    fn point_markers(&mut self, s: usize, pts: &[Option<Point>]) {
        if !self.g.mark_points {
            return;
        }
        for p in pts.iter().flatten() {
            let paint = self.mark(s, true, Paint::None, 0.0);
            let n = self.b.marker(nth(&self.designs, s), Rect::from_center_size(*p, (5.0, 5.0)), paint);
            self.push_series(s, n);
        }
    }

    /// A column, bar or stacked segment: with its column design when it has one (which casts no shadow), else a plain
    /// rectangle in `paint`, shadowed when `shadow`.
    fn bar_node(&mut self, bar: Bar, paint: (Paint, Paint, f64), shadow: bool) -> Arc<Node> {
        let Bar { s, rect, value, down, per_unit } = bar;
        self.reach(rect);
        let horizontal = self.horizontal;
        let art = nth(self.column_looks, s).and_then(|c| column_art(self.b.d, c, (rect, value), (horizontal, down), per_unit, &mut self.b.columns));
        match art {
            Some(n) => {
                // A design can reach past its column (Uniformly Scaled), so the legend keeps clear of it too.
                if let Some(art) = n.geometric_bounds() {
                    self.reach(art);
                }
                n
            }
            None => {
                let (fill, stroke, w) = paint;
                let n = self.b.path(shapes::rectangle(rect), fill, stroke, w);
                if shadow {
                    self.shadowed.push((s, n.clone()));
                }
                n
            }
        }
    }

    /// Columns (or bars) of one category side by side, and the stacks.
    fn columns(&mut self, geo: &Geo) {
        let g = self.g;
        let nser = self.nser;
        let columns: Vec<usize> = (0..nser).filter(|s| matches!(self.kind(*s), GraphKind::Column | GraphKind::Bar)).collect();
        let stacks: Vec<usize> = (0..nser).filter(|s| matches!(self.kind(*s), GraphKind::StackedColumn | GraphKind::StackedBar)).collect();
        let stack_axes: Vec<usize> = (0..2).filter(|a| stacks.iter().any(|s| geo.axis(*s) == *a)).collect();
        let col_frac = (g.column_width / 100.0).clamp(0.01, 10.0);
        let clus_frac = (g.cluster_width / 100.0).clamp(0.01, 10.0);
        let cluster = geo.cat_w * (g.cluster_width / 100.0).clamp(0.01, 10.0);
        let slot = cluster / (columns.len() + stack_axes.len()).max(1) as f64;
        let cl = Cluster {
            own_width: columns.is_empty() && stack_axes.len() < 2,
            columns,
            stacks,
            stack_axes,
            col_frac,
            clus_frac,
            cluster,
            slot,
            bar: slot * col_frac,
        };
        // First Row in Front: the first category drawn last, over the next.
        let cats: Vec<usize> = if g.first_row_in_front { (0..self.ncat).rev().collect() } else { (0..self.ncat).collect() };
        for c in cats {
            let c0 = geo.cat_start(c) + (geo.cat_w - cl.cluster) / 2.0;
            self.cluster_columns(geo, &cl, c, c0);
            self.cluster_stacks(geo, &cl, c, c0);
        }
    }

    fn cluster_columns(&mut self, geo: &Geo, cl: &Cluster, c: usize, c0: f64) {
        for (i, &s) in cl.columns.iter().enumerate() {
            let Some(v) = self.cell(c, s) else { continue };
            let a = c0 + cl.slot * i as f64 + (cl.slot - cl.bar) / 2.0;
            let paint = self.mark(s, true, Paint::None, 0.0);
            let (v0, v1) = (geo.zero(geo.axis(s)), geo.vpos(geo.axis(s), v));
            let rect = bar_rect(geo.horizontal, a, cl.bar, v0, v1);
            let n = self.bar_node(Bar { s, rect, value: v, down: v1 < v0, per_unit: geo.per_unit(geo.axis(s)) }, paint, true);
            self.push_series(s, n);
        }
    }

    fn cluster_stacks(&mut self, geo: &Geo, cl: &Cluster, c: usize, c0: f64) {
        // Each axis stacks its own series.
        let (mut pos, mut neg) = ([0.0f64; 2], [0.0f64; 2]);
        for &s in &cl.stacks {
            let Some(v) = self.cell(c, s) else { continue };
            let ax = geo.axis(s);
            let (a, w) = cl.place(geo, c, c0, ax);
            let Some(base) = (if v >= 0.0 { pos.get_mut(ax) } else { neg.get_mut(ax) }) else { continue };
            let (v0, v1) = (geo.vpos(ax, *base), geo.vpos(ax, *base + v));
            *base += v;
            let paint = self.mark(s, true, Paint::solid(Color::WHITE), 0.25);
            let rect = bar_rect(geo.horizontal, a, w, v0, v1);
            // A segment of 0 has no area to shade.
            let n = self.bar_node(Bar { s, rect, value: v, down: v1 < v0, per_unit: geo.per_unit(ax) }, paint, v != 0.0);
            self.push_series(s, n);
        }
    }

    /// Cumulative area bands, each from the previous area total on its axis up to its own.
    fn areas(&mut self, geo: &Geo) {
        let ncat = self.ncat;
        let mut belows: [Vec<f64>; 2] = [vec![0.0; ncat], vec![0.0; ncat]];
        for s in self.series_of(GraphKind::Area) {
            let a = geo.axis(s);
            let Some(below) = belows.get_mut(a) else { continue };
            let above: Vec<f64> = below.iter().enumerate().map(|(c, b)| b + self.val(c, s)).collect();
            // Mixed with series at the category centres, areas take their points there too.
            let at = |c: usize, v: &[f64]| Point::new(geo.cat_mid(c), geo.vpos(a, v.get(c).copied().unwrap_or(0.0)));
            let mut pts: Vec<Point> = (0..ncat).map(|c| at(c, &above)).collect();
            pts.extend((0..ncat).rev().map(|c| at(c, below)));
            let (fill, stroke, w) = self.mark(s, true, Paint::solid(Color::WHITE), 0.25);
            let n = self.b.path(polyline(&pts, true), fill, stroke, w);
            self.push_series(s, n);
            *below = above;
        }
    }

    fn lines(&mut self, geo: &Geo) {
        let g = self.g;
        for s in self.series_of(GraphKind::Line) {
            let pts: Vec<Option<Point>> = (0..self.ncat)
                .map(|c| self.cell(c, s).map(|v| Point::new(geo.mid(c, g.edge_to_edge && geo.label_edges), geo.vpos(geo.axis(s), v))))
                .collect();
            if g.connect_points {
                for run in runs(&pts) {
                    let n = self.b.data_line(g, s, polyline(&run, false), self.filled);
                    self.push_shadowed(s, n);
                }
            }
            self.point_markers(s, &pts);
        }
    }

    // ---- Legend and layering ----

    /// Legend: a swatch + label per series, right of the plot; the swatch joins its series group. Returns the labels.
    fn legend(&mut self) -> Vec<Arc<Node>> {
        let g = self.g;
        let legend_on = g.legend && !g.series.is_empty() && !(g.kind == GraphKind::Pie && g.pie_legend_in_wedges);
        if !legend_on {
            return vec![];
        }
        if g.legend_across_top { self.legend_across_top() } else { self.legend_beside() }
    }

    fn legend_series(&self) -> usize {
        if self.g.kind == GraphKind::Scatter { self.nser.div_ceil(2) } else { self.nser }
    }

    fn legend_text(&self, s: usize) -> String {
        if self.g.kind == GraphKind::Scatter { self.series_label(s * 2) } else { self.series_label(s) }
    }

    /// The swatch of series `s` at `(x, y)`: series drawn with markers show their marker design in the legend, and
    /// series drawn with a column design that design.
    fn legend_swatch(&mut self, s: usize, (x, y): (f64, f64)) {
        let marked = matches!(self.g.kind, GraphKind::Scatter | GraphKind::Radar) || self.kind(s) == GraphKind::Line;
        let paint = self.mark(s, true, Paint::None, 0.0);
        let column = nth(self.column_looks, s).filter(|_| self.columnar(s));
        let swatch = self.b.swatch(column, nth(&self.designs, s).filter(|_| marked), Rect::new(x, y, x + 8.0, y + 8.0), paint);
        self.push_series(s, swatch);
    }

    fn legend_label(&mut self, (x, y): (f64, f64), label: &str) -> Arc<Node> {
        let ls = self.ls;
        self.b.label(Role::Legend, Point::new(x + 12.0, y + 7.5 + (ls - LABEL_SIZE) * 0.35), label, Justify::Left, false)
    }

    /// Add Legend Across Top: swatch and label after swatch and label, in rows as wide as the plot, the last row
    /// just above it.
    fn legend_across_top(&mut self) -> Vec<Arc<Node>> {
        let (g, r, ls) = (self.g, self.r, self.ls);
        let entries: Vec<(usize, String)> = (0..self.legend_series()).map(|s| (s, self.legend_text(s))).filter(|(_, l)| !blank_label(l)).collect();
        let mut rows: Vec<Vec<(usize, String, f64)>> = vec![vec![]];
        let mut used = 0.0;
        for (s, label) in entries {
            let w = 12.0 + label_width(&g.label_styles, &label) + 12.0;
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
        let height = |row: &[(usize, String, f64)]| ls * 1.8 * row.iter().map(|(_, l, _)| l.split('|').count()).max().unwrap_or(1) as f64;
        // Above the plot and anything drawn over it (radar and stacked-pie labels).
        let top = self.b.out.iter().chain(self.on_top.iter()).filter_map(|n| n.geometric_bounds()).map(|bb| bb.y0).fold(r.y0, f64::min);
        let top = self.bars_reach.map_or(top, |b| top.min(b.y0));
        let mut y = top - 8.0 - rows.iter().map(|row| height(row)).sum::<f64>();
        let mut labels = vec![];
        for row in &rows {
            let mut x = r.x0;
            for (s, label, w) in row {
                self.legend_swatch(*s, (x, y));
                labels.push(self.legend_label((x, y), label));
                x += w;
            }
            y += height(row);
        }
        labels
    }

    fn legend_beside(&mut self) -> Vec<Arc<Node>> {
        let (r, ls) = (self.r, self.ls);
        let x = self.bars_reach.map_or(self.right_edge, |b| self.right_edge.max(b.x1)) + 14.0;
        // Each row as tall as its label's lines (a `|` in a label starts a new line).
        let mut y = r.y0;
        let mut labels = vec![];
        for s in 0..self.legend_series() {
            let label = self.legend_text(s);
            let row_y = y;
            if blank_label(&label) {
                y += ls * 1.8;
                continue;
            }
            y += ls * 1.8 * label.split('|').count() as f64;
            self.legend_swatch(s, (x, row_y));
            labels.push(self.legend_label((x, row_y), &label));
        }
        labels
    }

    /// Areas at the back, lines and points in front of columns; one graph type keeps the series order.
    fn depth(&self, s: usize) -> u8 {
        match self.kind(s) {
            GraphKind::Area => 0,
            GraphKind::Line => 2,
            _ => 1,
        }
    }

    /// Add Drop Shadow: a translucent black copy of each column, bar, line and wedge, a little down and right, just
    /// behind the series of its layer (areas, columns, lines), so a layer in front doesn't hide the shadows of the one
    /// behind. Left out past SHADOW_BUDGET marks.
    fn finish(mut self, mut labels: Vec<Arc<Node>>) -> Option<Vec<Arc<Node>>> {
        let g = self.g;
        if !g.drop_shadow || self.shadowed.len() > SHADOW_BUDGET {
            self.shadowed.clear();
        }
        let mut series: Vec<(usize, Vec<Arc<Node>>)> = std::mem::take(&mut self.series).into_iter().enumerate().collect();
        // First Column in Front: the first series drawn last in its layer, over the others.
        if g.first_column_in_front {
            series.reverse();
        }
        series.sort_by_key(|(s, _)| self.depth(*s));
        let mut layer = None;
        for (s, items) in series {
            if items.is_empty() {
                continue;
            }
            let depth = self.depth(s);
            if layer != Some(depth) {
                layer = Some(depth);
                let casting: Vec<Arc<Node>> = self.shadowed.iter().filter(|(t, _)| self.depth(*t) == depth).map(|(_, n)| n.clone()).collect();
                let copies: Vec<Arc<Node>> = casting.iter().map(|n| shadow_of(self.b.d, n)).collect();
                if !copies.is_empty() {
                    let mut grp = Node::group(self.b.d.alloc_id(), copies);
                    grp.name = Some("Drop Shadow".into());
                    grp.opacity = SHADOW_OPACITY;
                    self.b.out.push(Arc::new(grp));
                }
            }
            let name = g.series.get(s).cloned().filter(|n| !n.is_empty()).unwrap_or_else(|| format!("Series {}", s + 1));
            let grp = self.b.series(&name, s, items);
            self.b.out.push(grp);
        }
        self.b.out.append(&mut self.on_top);
        labels.append(&mut self.wedge_labels);
        if !labels.is_empty() {
            let grp = self.b.group("Legend", labels);
            self.b.out.push(grp);
        }
        (!self.b.columns.short).then_some(self.b.out)
    }
}

pub(super) fn children_bounds(children: &[Arc<Node>]) -> Option<Rect> {
    children.iter().filter_map(|c| c.geometric_bounds()).reduce(|a, b| a.union(b))
}
