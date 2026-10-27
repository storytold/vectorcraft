use super::axis::{LABEL_SIZE, MAX_CATEGORY_TICKS, MAX_TICK_MARKS, one_line, tick_values, value_axis};
use super::columns::{Allowance, ColumnArt, column_art, column_designs};
use super::data::fmt_value;
use super::labels::{
    FILLED_LINE_BUDGET, Gen, Role, SHADOW_BUDGET, SHADOW_OPACITY, blank_label, label_size, label_width, polyline, recolour_text, runs, shadow_of,
};
use super::markers::{MARKER_BUDGET, marker_cost, marker_designs};
use super::series::series_marks;
use super::*;

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
    let mark = |i: usize, series_fill: bool, stroke: Paint, width: f64| series_marks(g, i, series_fill, stroke, width);
    // Each series' marker design (Object › Graph › Marker), when the document has it; default squares for all
    // when the designs would draw more than MARKER_BUDGET objects.
    let mut designs = marker_designs(d, g);
    if marker_cost(g, &designs) > MARKER_BUDGET {
        designs.clear();
    }
    let design = |s: usize| designs.get(s).and_then(Option::as_ref);
    let column_look = |s: usize| column_looks.get(s).and_then(Option::as_ref);
    let mut b = Gen { d, out: vec![], styles: &g.label_styles, columns: Allowance::new() };
    // The labels' sizes (9 pt unless formatted), which their spacing follows.
    let (vs, cs, ls) =
        (label_size(&g.label_styles, Role::Value), label_size(&g.label_styles, Role::Category), label_size(&g.label_styles, Role::Legend));
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
    // Series drawn as columns or bars, which column designs apply to.
    let columnar = |s: usize| matches!(kind(s), GraphKind::Column | GraphKind::StackedColumn | GraphKind::Bar | GraphKind::StackedBar);
    let horizontal = matches!(g.kind, GraphKind::Bar | GraphKind::StackedBar);
    // Where the art right of the plot ends (the right value axis' labels), so the legend starts after it.
    let mut right_edge = r.x1;

    // Labels drawn over the series: a pie's Legends in Wedges (in the Legend group) and stacked pies' names.
    let mut wedge_labels = vec![];
    let mut on_top = vec![];
    // Draw Filled Lines, up to FILLED_LINE_BUDGET data points (the outlines are rebuilt on every change).
    let filled = g.filled_lines && g.rows.len().saturating_mul(nser) <= FILLED_LINE_BUDGET;
    // Where columns and bars reach (past the plot when their widths are over 100%), so the legend keeps clear.
    let mut bars_reach: Option<Rect> = None;
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
                            let t = b.label(Role::Legend, Point::new(at.x, at.y + ls * 0.35), &label, Justify::Center, true);
                            // White on a dark wedge, unless the legend labels were given a colour of their own.
                            let own_colour = g.label_styles.legend.as_ref().is_some_and(|st| st.fill != Paint::solid(Color::BLACK));
                            wedge_labels.push(if dark && !own_colour { recolour_text(&t, Color::WHITE) } else { t });
                        }
                    }
                    a0 += sweep;
                }
                if let Some(cat) = g.categories.get(c).filter(|c| !c.is_empty()) {
                    if stacked {
                        // Just above its own outline, all its lines, and drawn over the pies.
                        let lines = cat.split('|').count() as f64;
                        let at = Point::new(centre.x, centre.y - rad - cs * 0.4 - (lines - 1.0) * cs * 1.2);
                        let t = b.label(Role::Category, at, cat, Justify::Center, false);
                        on_top.push(t);
                    } else {
                        let t = b.label(Role::Category, Point::new(centre.x, centre.y + rad + cs * 1.6), cat, Justify::Center, false);
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
                    let p = at(c, hi) + (at(c, hi) - centre).normalize() * (cs * 1.2);
                    axes.push(b.label(Role::Category, Point::new(p.x, p.y + cs * 0.35), cat, Justify::Center, true));
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
                    let n = b.data_line(g, s, polyline(&pts, true), filled);
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
            // Plot points per unit of value along value axis `a`.
            let per_unit = |a: usize| {
                let (lo, hi, _) = scale(a);
                (if horizontal { r.width() } else { r.height() }) / (hi - lo)
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
                            b.text(Role::Value, Point::new(q, r.y1 + 4.0 + vs * 1.1), &label, Justify::Center)
                        } else if a == 1 {
                            b.text(Role::Value, Point::new(r.x1 + 6.0, q + vs * 0.35), &label, Justify::Left)
                        } else {
                            b.text(Role::Value, Point::new(r.x0 - 6.0, q + vs * 0.35), &label, Justify::Right)
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
                            b.label(Role::Category, Point::new(r.x0 - 6.0, cat_mid(c) + cs * 0.35), cat, Justify::Right, true)
                        } else {
                            b.label(Role::Category, Point::new(cat_mid(c), r.y1 + cs * 1.4), cat, Justify::Center, false)
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
                // Like the value axis: finite, and a counted (capped) list of ticks, so data near f64::MAX can't loop forever.
                let x_lo = if xlo == f64::MAX { 0.0 } else { xlo.min(0.0) };
                let x_hi = if xhi == f64::MIN { 1.0 } else { xhi };
                let (xl, xh, xs) = value_axis(g.ticks, None, None, x_lo, x_hi);
                for x in tick_values(xl, xh, xs) {
                    let q = r.x0 + (x - xl) / (xh - xl) * r.width();
                    axes.push(b.line(Point::new(q, r.y1), Point::new(q, r.y1 + 4.0)));
                    axes.push(b.text(Role::Value, Point::new(q, r.y1 + 4.0 + vs * 1.1), &fmt_value(x), Justify::Center));
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
                            let n = b.data_line(g, si, polyline(&run, false), filled);
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
            // Widths past 100% make the columns (or clusters) overlap.
            let cluster = cat_w * (g.cluster_width / 100.0).clamp(0.01, 10.0);
            let slot = cluster / slots.max(1) as f64;
            let bar = slot * (g.column_width / 100.0).clamp(0.01, 10.0);
            let bar_rect = |a: f64, w: f64, v0: f64, v1: f64| {
                if horizontal { Rect::new(v0.min(v1), a, v0.max(v1), a + w) } else { Rect::new(a, v0.min(v1), a + w, v0.max(v1)) }
            };
            // First Row in Front: the first category drawn last, over the next.
            let cats: Vec<usize> = if g.first_row_in_front { (0..ncat).rev().collect() } else { (0..ncat).collect() };
            for c in cats {
                let c0 = cat_start(c) + (cat_w - cluster) / 2.0;
                for (i, &s) in columns.iter().enumerate() {
                    let Some(v) = cell(c, s) else { continue };
                    let a = c0 + slot * i as f64 + (slot - bar) / 2.0;
                    let (fill, stroke, w) = mark(s, true, Paint::None, 0.0);
                    let (v0, v1) = (zero(axis(s)), vpos(axis(s), v));
                    let rect = bar_rect(a, bar, v0, v1);
                    bars_reach = Some(bars_reach.map_or(rect, |r| r.union(rect)));
                    // A column drawn with a design casts no shadow.
                    let n = match column_look(s).and_then(|c| column_art(b.d, c, (rect, v), (horizontal, v1 < v0), per_unit(axis(s)), &mut b.columns))
                    {
                        Some(n) => {
                            // A design can reach past its column (Uniformly Scaled), so the legend keeps clear of it too.
                            if let Some(art) = n.geometric_bounds() {
                                bars_reach = Some(bars_reach.map_or(art, |r| r.union(art)));
                            }
                            n
                        }
                        None => {
                            let n = b.path(shapes::rectangle(rect), fill, stroke, w);
                            shadowed.push((s, n.clone()));
                            n
                        }
                    };
                    if let Some(items) = series.get_mut(s) {
                        items.push(n);
                    }
                }
                let place = |ax: usize| {
                    if own_width {
                        // Up to the category's width, or past it when the widths are set past 100%.
                        let (col, clus) = ((g.column_width / 100.0).clamp(0.01, 10.0), (g.cluster_width / 100.0).clamp(0.01, 10.0));
                        let w = (cat_w * col * clus * 1.2).min(cat_w * (col * clus).max(1.0));
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
                    let rect = bar_rect(a, w, v0, v1);
                    bars_reach = Some(bars_reach.map_or(rect, |r| r.union(rect)));
                    let n = match column_look(s).and_then(|c| column_art(b.d, c, (rect, v), (horizontal, v1 < v0), per_unit(ax), &mut b.columns)) {
                        Some(n) => {
                            if let Some(art) = n.geometric_bounds() {
                                bars_reach = Some(bars_reach.map_or(art, |r| r.union(art)));
                            }
                            n
                        }
                        None => {
                            let n = b.path(shapes::rectangle(rect), fill, stroke, sw);
                            // A segment of 0 has no area to shade.
                            if v != 0.0 {
                                shadowed.push((s, n.clone()));
                            }
                            n
                        }
                    };
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
                        let n = b.data_line(g, s, polyline(&run, false), filled);
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
        let top = b.out.iter().chain(on_top.iter()).filter_map(|n| n.geometric_bounds()).map(|bb| bb.y0).fold(r.y0, f64::min);
        let top = bars_reach.map_or(top, |b| top.min(b.y0));
        let mut y = top - 8.0 - rows.iter().map(|row| height(row)).sum::<f64>();
        for row in &rows {
            let mut x = r.x0;
            for (s, label, w) in row {
                let marked = matches!(g.kind, GraphKind::Scatter | GraphKind::Radar) || kind(*s) == GraphKind::Line;
                let paint = mark(*s, true, Paint::None, 0.0);
                let swatch =
                    b.swatch(column_look(*s).filter(|_| columnar(*s)), design(*s).filter(|_| marked), Rect::new(x, y, x + 8.0, y + 8.0), paint);
                if let Some(items) = series.get_mut(*s) {
                    items.push(swatch);
                }
                labels.push(b.label(Role::Legend, Point::new(x + 12.0, y + 7.5 + (ls - LABEL_SIZE) * 0.35), label, Justify::Left, false));
                x += w;
            }
            y += height(row);
        }
    } else if legend_on {
        let x = bars_reach.map_or(right_edge, |b| right_edge.max(b.x1)) + 14.0;
        // Each row as tall as its label's lines (a `|` in a label starts a new line).
        let mut y = r.y0;
        for (s, items) in series.iter_mut().enumerate().take(legend_series) {
            let label = if g.kind == GraphKind::Scatter { series_label(s * 2) } else { series_label(s) };
            let row_y = y;
            if blank_label(&label) {
                y += ls * 1.8;
                continue;
            }
            y += ls * 1.8 * label.split('|').count() as f64;
            let y = row_y;
            // Series drawn with markers show their marker design in the legend, and series drawn with a column design
            // that design.
            let marked = matches!(g.kind, GraphKind::Scatter | GraphKind::Radar) || kind(s) == GraphKind::Line;
            let paint = mark(s, true, Paint::None, 0.0);
            items.push(b.swatch(column_look(s).filter(|_| columnar(s)), design(s).filter(|_| marked), Rect::new(x, y, x + 8.0, y + 8.0), paint));
            labels.push(b.label(Role::Legend, Point::new(x + 12.0, y + 7.5 + (ls - LABEL_SIZE) * 0.35), &label, Justify::Left, false));
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
    // First Column in Front: the first series drawn last in its layer, over the others.
    if g.first_column_in_front {
        series.reverse();
    }
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
    (!b.columns.short).then_some(b.out)
}

pub(super) fn children_bounds(children: &[Arc<Node>]) -> Option<Rect> {
    children.iter().filter_map(|c| c.geometric_bounds()).reduce(|a, b| a.union(b))
}
