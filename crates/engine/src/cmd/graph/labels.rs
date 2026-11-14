use super::axis::LABEL_SIZE;
use super::columns::{Allowance, ColumnArt};
use super::markers::fit_design;
use super::series::{default_series_paint, series_marks};
use super::{
    Affine, Appearance, Arc, BezPath, CharStyle, Color, Document, GraphSpec, Justify, Node, NodeId, NodeKind, Paint, PathData, Point, Rect,
    TextObject, refresh_bounds, shapes,
};

pub(super) struct Gen<'a> {
    pub(super) d: &'a mut Document,
    pub(super) out: Vec<Arc<Node>>,
    /// The labels' kept character styles and paragraph attributes.
    pub(super) styles: &'a vectorcraft_doc::LabelStyles,
    /// What the column designs may still add.
    pub(super) columns: Allowance,
}

/// What a graph label shows, for its kept character style.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Role {
    Value,
    Category,
    Legend,
}

/// The character style labels of `role` are set in: the kept one, else 9 pt black.
pub(super) fn label_style(styles: &vectorcraft_doc::LabelStyles, role: Role) -> CharStyle {
    let kept = match role {
        Role::Value => &styles.values,
        Role::Category => &styles.categories,
        Role::Legend => &styles.legend,
    };
    let mut st = kept.clone().unwrap_or_else(|| CharStyle { size: LABEL_SIZE, fill: Paint::solid(Color::BLACK), ..CharStyle::default() });
    // A file's style within the limits the Character controls keep to (it is copied onto every label).
    let within = |v: f64, lo: f64, hi: f64, or: f64| if v.is_finite() { v.clamp(lo, hi) } else { or };
    st.size = within(st.size, 0.1, 1296.0, LABEL_SIZE);
    st.leading = st.leading.map(|l| within(l, 0.1, 5000.0, LABEL_SIZE * 1.2));
    st.tracking = within(st.tracking, -1000.0, 10000.0, 0.0);
    st.h_scale = within(st.h_scale, 1.0, 10000.0, 100.0);
    st.v_scale = within(st.v_scale, 1.0, 10000.0, 100.0);
    st.stroke_width = within(st.stroke_width, 0.0, 1000.0, 0.0);
    st.font_family = st.font_family.chars().take(256).collect();
    st.font_style = st.font_style.chars().take(256).collect();
    st
}

/// The paragraph attributes labels of `role` are set with: the kept ones, else the defaults. The caller sets the
/// alignment.
pub(super) fn label_para(styles: &vectorcraft_doc::LabelStyles, role: Role) -> vectorcraft_doc::ParaStyle {
    let mut para = kept_para(styles, role).cloned().unwrap_or_default();
    // A file's attributes within the limits the Paragraph controls keep to (they are copied onto every label).
    para.keep_spacing_in_range();
    para
}

/// The paragraph attributes labels of `role` keep, if any.
fn kept_para(styles: &vectorcraft_doc::LabelStyles, role: Role) -> Option<&vectorcraft_doc::ParaStyle> {
    match role {
        Role::Value => styles.values_para.as_ref(),
        Role::Category => styles.categories_para.as_ref(),
        Role::Legend => styles.legend_para.as_ref(),
    }
}

/// The anti-aliasing method labels of `role` are drawn with: the kept one, else Sharp.
pub(super) fn label_anti_alias(styles: &vectorcraft_doc::LabelStyles, role: Role) -> vectorcraft_doc::text::TypeAntiAlias {
    let kept = match role {
        Role::Value => styles.values_anti_alias,
        Role::Category => styles.categories_anti_alias,
        Role::Legend => styles.legend_anti_alias,
    };
    kept.unwrap_or_default()
}

/// The object rows labels of `role` keep, if any.
fn kept_rows(styles: &vectorcraft_doc::LabelStyles, role: Role) -> Option<&vectorcraft_doc::LabelRows> {
    match role {
        Role::Value => styles.values_rows.as_ref(),
        Role::Category => styles.categories_rows.as_ref(),
        Role::Legend => styles.legend_rows.as_ref(),
    }
}

/// What a graph line is, for the rows it keeps: an axis line or tick mark, or a ring of a radar graph.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Line {
    Axis,
    Ring,
}

impl Line {
    /// The stroke and weight the graph draws lines of this kind with when they keep no rows. They have no fill.
    fn default_stroke(self) -> (Paint, f64) {
        match self {
            Line::Axis => (Paint::solid(Color::BLACK), 0.5),
            Line::Ring => (Paint::solid(Color::rgb(0.6, 0.6, 0.6)), 0.25),
        }
    }

    /// The rows ([`super::series::label_rows`]) a line of this kind shows as the graph draws it with no kept rows.
    pub(super) fn default_rows(self) -> vectorcraft_doc::LabelRows {
        let (stroke, width) = self.default_stroke();
        let paint = vectorcraft_doc::SeriesPaint { fill: Some(Paint::None), stroke: Some(stroke), stroke_width: Some(width), ..Default::default() };
        vectorcraft_doc::LabelRows { paint, ..Default::default() }
    }
}

/// The rows lines of kind `line` keep, if any.
pub(super) fn kept_line_rows(styles: &vectorcraft_doc::LabelStyles, line: Line) -> Option<&vectorcraft_doc::LabelRows> {
    match line {
        Line::Axis => styles.axis_rows.as_ref(),
        Line::Ring => styles.ring_rows.as_ref(),
    }
}

/// The slot of `styles` that holds the rows lines of kind `line` keep.
pub(super) fn line_slot(styles: &mut vectorcraft_doc::LabelStyles, line: Line) -> &mut Option<vectorcraft_doc::LabelRows> {
    match line {
        Line::Axis => &mut styles.axis_rows,
        Line::Ring => &mut styles.ring_rows,
    }
}

/// Give label or graph line `n` the object rows `kept` keeps: a fill row and a stroke row for the kept paints, in their
/// kept order and place relative to the contents, and the object opacity and blend mode. An opacity read from a file is
/// held to 0..1 and a weight to 0..1000 pt; one that is not a number is left out (a stroke then takes 1 pt).
fn apply_label_rows(n: &mut Node, kept: &vectorcraft_doc::LabelRows) {
    use vectorcraft_doc::{AppearanceItem, FillLayer, StrokeLayer};
    let rows = &kept.paint;
    let unit = |o: Option<f32>| o.filter(|o| o.is_finite()).map(|o| o.clamp(0.0, 1.0));
    let fill = rows.fill.as_ref().map(|paint| {
        let mut f = FillLayer::new(paint.clone());
        f.opacity = unit(rows.fill_opacity).unwrap_or(1.0);
        f.blend = rows.fill_blend.unwrap_or_default();
        AppearanceItem::Fill(f)
    });
    let stroke = rows.stroke.as_ref().map(|paint| {
        let width = rows.stroke_width.filter(|w| w.is_finite()).map_or(1.0, |w| w.clamp(0.0, 1000.0));
        let mut st = StrokeLayer::new(paint.clone(), width);
        st.opacity = unit(rows.stroke_opacity).unwrap_or(1.0);
        st.blend = rows.stroke_blend.unwrap_or_default();
        AppearanceItem::Stroke(st)
    });
    let items: Vec<AppearanceItem> = if kept.stroke_under { [stroke, fill] } else { [fill, stroke] }.into_iter().flatten().collect();
    let mut ap = Appearance::clone(&n.appearance);
    ap.items = items;
    ap.set_contents_at(usize::from(kept.below.unwrap_or(0)));
    n.appearance = ap.into();
    n.opacity = unit(rows.opacity).unwrap_or(1.0);
    n.blend = rows.blend.unwrap_or_default();
}

/// How big labels of `role` are set: a kept style's size, else 9 pt. The labels' spacing follows it.
pub(super) fn label_size(styles: &vectorcraft_doc::LabelStyles, role: Role) -> f64 {
    label_style(styles, role).size
}

impl Gen<'_> {
    pub(super) fn path(&mut self, pd: PathData, fill: Paint, stroke: Paint, w: f64) -> Arc<Node> {
        Arc::new(Node::path(self.d.alloc_id(), pd, Appearance::basic(fill, stroke, w)))
    }
    pub(super) fn line(&mut self, a: Point, b: Point) -> Arc<Node> {
        let mut bp = BezPath::new();
        bp.move_to(a);
        bp.line_to(b);
        self.kept_line(Line::Axis, PathData::from_bezpath(&bp))
    }
    /// A ring of a radar graph through `pd`.
    pub(super) fn ring(&mut self, pd: PathData) -> Arc<Node> {
        self.kept_line(Line::Ring, pd)
    }
    /// A graph line of kind `line` through `pd`, with the rows lines of its kind keep, else its default stroke.
    fn kept_line(&mut self, line: Line, pd: PathData) -> Arc<Node> {
        let (stroke, width) = line.default_stroke();
        let mut n = Node::path(self.d.alloc_id(), pd, Appearance::basic(Paint::None, stroke, width));
        if let Some(rows) = kept_line_rows(self.styles, line) {
            apply_label_rows(&mut n, rows);
        }
        Arc::new(n)
    }
    pub(super) fn text(&mut self, role: Role, at: Point, s: &str, justify: Justify) -> Arc<Node> {
        let t = self.text_object(role, at, s, justify);
        self.label_node(role, t)
    }
    /// Label text `t` of `role` as a node, with the object rows its kind keeps.
    fn label_node(&mut self, role: Role, t: TextObject) -> Arc<Node> {
        let mut n = Node::new(self.d.alloc_id(), NodeKind::Text(Box::new(t)));
        if let Some(rows) = kept_rows(self.styles, role) {
            apply_label_rows(&mut n, rows);
        }
        Arc::new(n)
    }
    /// Label `s` of `role` set at `at` and laid out, not yet in the document.
    fn text_object(&self, role: Role, at: Point, s: &str, justify: Justify) -> TextObject {
        let style = label_style(self.styles, role);
        let mut t = TextObject::point(at, s, style);
        t.para = label_para(self.styles, role);
        t.anti_alias = label_anti_alias(self.styles, role);
        t.para.justify = justify;
        t.system_layout = self.d.setup.system_layout;
        refresh_bounds(&mut t);
        t
    }
    /// A category or series label: a `|` in it starts a new line (Add graph labels). `middle`: the lines are centred
    /// on `at` (labels beside an axis or in a wedge) instead of running down from it.
    pub(super) fn label(&mut self, role: Role, at: Point, s: &str, justify: Justify, middle: bool) -> Arc<Node> {
        let lines = s.split('|').count();
        // Default leading puts the first and last baselines this far above and below `at`.
        let half = (lines - 1) as f64 * label_size(self.styles, role) * 1.2 / 2.0;
        let start = if middle { Point::new(at.x, at.y - half) } else { at };
        let mut t = self.text_object(role, start, &s.replace('|', "\n"), justify);
        // Kept leading or paragraph spacing moves the lines: put the middle of the first and last baselines on `at`. An
        // empty line has no baseline: an empty first or last line leaves the label where the default leading puts it.
        if middle && kept_para(self.styles, role).is_some() {
            let ys: Vec<f64> = t.baselines().map(|(a, _)| a.y).collect();
            let ends_set = s.split('|').next().is_some_and(|l| !l.is_empty()) && s.split('|').next_back().is_some_and(|l| !l.is_empty());
            let set = s.split('|').filter(|l| !l.is_empty()).count();
            if let (Some(first), Some(last), true) = (ys.first(), ys.last(), ends_set && ys.len() == set) {
                t.xf = Affine::translate((0.0, half - (first + last) / 2.0)) * t.xf;
            }
        }
        self.label_node(role, t)
    }
    pub(super) fn group(&mut self, name: &str, children: Vec<Arc<Node>>) -> Arc<Node> {
        let mut g = Node::group(self.d.alloc_id(), children);
        g.name = Some(name.into());
        Arc::new(g)
    }
    /// A series' line joining its data points (Connect Data Points): a 1 pt stroke in the series colour, or with Draw
    /// Filled Lines a band FILLED_LINE_WIDTH wide filled with it and outlined in black.
    pub(super) fn data_line(&mut self, g: &GraphSpec, s: usize, pd: PathData, filled: bool) -> Arc<Node> {
        let band = if filled {
            vectorcraft_pathops::outline_stroke(&pd, FILLED_LINE_WIDTH, vectorcraft_pathops::Cap::Butt, vectorcraft_pathops::Join::Miter, 4.0)
        } else {
            PathData::default()
        };
        // A run the outline can't be made for (its points all in one place) stays a thin line.
        if !band.is_empty() {
            // The band takes the series' color, set as a fill or, on a thin line, as a stroke with paint; a thin black outline.
            let slot = g.series_paints.get(s);
            let fill =
                slot.and_then(|p| p.fill.clone().or_else(|| p.stroke.clone().filter(|st| !st.is_none()))).unwrap_or_else(|| default_series_paint(s));
            return self.path(band, fill, Paint::solid(Color::BLACK), 0.25);
        }
        let (fill, stroke, w) = series_marks(g, s, false, default_series_paint(s), 1.0);
        self.path(pd, fill, stroke, w)
    }
    /// A data point's marker filling `cell`: the series' marker design, else the default square in the series' paint.
    pub(super) fn marker(&mut self, design: Option<&Arc<Node>>, cell: Rect, (fill, stroke, w): (Paint, Paint, f64)) -> Arc<Node> {
        match design.and_then(|a| fit_design(self.d, a, cell)) {
            Some(n) => self.design_copy(n),
            None => self.path(shapes::rectangle(cell), fill, stroke, w),
        }
    }
    /// A legend swatch filling `cell`: the series' column design with its totals left out, else, and when the design
    /// draws nothing else, its [`Self::marker`].
    pub(super) fn swatch(&mut self, column: Option<&ColumnArt>, marker: Option<&Arc<Node>>, cell: Rect, paint: (Paint, Paint, f64)) -> Arc<Node> {
        let fitted = match column.and_then(|c| Some((c, c.legend.as_ref()?))) {
            Some((c, legend)) if self.columns.take(legend.count()) => c.fit(self.d, legend, cell),
            _ => None,
        };
        match fitted {
            Some(n) => self.design_copy(n),
            None => self.marker(marker, cell, paint),
        }
    }
    /// A copy of design art drawn in a series: a single path gets a group of its own, as painting a series paints the
    /// paths directly inside its group and the design keeps its own paint.
    fn design_copy(&mut self, n: Arc<Node>) -> Arc<Node> {
        if matches!(n.kind, NodeKind::Path { .. }) { Arc::new(Node::group(self.d.alloc_id(), vec![n])) } else { n }
    }
    /// A series group. Empty series are omitted by the caller; the index is stored on the group
    /// so later paint edits find the series without counting siblings.
    pub(super) fn series(&mut self, name: &str, index: usize, children: Vec<Arc<Node>>) -> Arc<Node> {
        let mut g = Node::group(self.d.alloc_id(), children);
        g.name = Some(name.into());
        g.extras_mut().series_index = u32::try_from(index).ok();
        Arc::new(g)
    }
}

/// Add Drop Shadow: how far the shadow sits down and right of its mark, and how dark it is.
const SHADOW_OFFSET: f64 = 2.0;

pub(super) const SHADOW_OPACITY: f32 = 0.35;

/// Draw Filled Lines: how wide a filled line is.
const FILLED_LINE_WIDTH: f64 = 3.0;

const FILLED_LINE_BUDGET: usize = 20_000;

/// Does graph `g`, with `series` series, draw Filled Lines (up to FILLED_LINE_BUDGET data points)?
pub(super) fn filled_lines_on(g: &GraphSpec, series: usize) -> bool {
    g.filled_lines && g.rows.len().saturating_mul(series) <= FILLED_LINE_BUDGET
}

/// Most marks a graph draws shadows for (each is a copy).
pub(super) const SHADOW_BUDGET: usize = 200_000;

/// Mark `n`'s shadow: a black copy moved down and right, its fill and stroke (whichever it has) black.
pub(super) fn shadow_of(d: &mut Document, n: &Node) -> Arc<Node> {
    // Generated marks carry only paint and a stroke width, so that's all a shadow keeps.
    let mut c = n.clone();
    c.transform(Affine::translate((SHADOW_OFFSET, SHADOW_OFFSET)), false);
    let fill = if c.appearance.fill_paint().is_none() { Paint::None } else { Paint::solid(Color::BLACK) };
    let stroke = if c.appearance.stroke_paint().is_none() { Paint::None } else { Paint::solid(Color::BLACK) };
    c.appearance = Appearance::basic(fill, stroke, c.appearance.stroke_width()).into();
    Arc::new(d.reid(&c))
}

/// The bounds of legend label `s` set from the origin with paragraph attributes `para` and the document's System
/// Layout setting `system_layout`, as [`Gen::text`] sets it, without adding it to a document.
fn legend_label_bounds(styles: &vectorcraft_doc::LabelStyles, s: &str, para: vectorcraft_doc::ParaStyle, system_layout: bool) -> Option<Rect> {
    let style = label_style(styles, Role::Legend);
    let mut t = TextObject::point(Point::ZERO, &s.replace('|', "\n"), style);
    t.para = para;
    t.para.justify = Justify::Left;
    t.system_layout = system_layout;
    refresh_bounds(&mut t);
    Node::new(NodeId(0), NodeKind::Text(Box::new(t))).geometric_bounds()
}

/// How far label `s` reaches right of where it starts (its widest line and any indent), after the legend moves it right
/// by [`label_shift`], without adding it to a document.
pub(super) fn label_width(styles: &vectorcraft_doc::LabelStyles, s: &str, system_layout: bool) -> f64 {
    legend_label_bounds(styles, s, label_para(styles, Role::Legend), system_layout)
        .map_or(0.0, |b| (b.x1 + label_shift(styles, s, system_layout).0).max(0.0))
}

/// The bounds of legend label `s` set with its kept paragraph attributes and with the defaults, when it has kept ones.
fn kept_and_default_bounds(styles: &vectorcraft_doc::LabelStyles, s: &str, system_layout: bool) -> Option<(Rect, Rect)> {
    styles.legend_para.as_ref()?;
    Some((
        legend_label_bounds(styles, s, label_para(styles, Role::Legend), system_layout)?,
        legend_label_bounds(styles, s, vectorcraft_doc::ParaStyle::default(), system_layout)?,
    ))
}

/// How much further left and further up legend label `s` reaches with its kept paragraph attributes than with the
/// defaults (a negative indent or negative spacing), so the legend moves it right and down by that much and it stays
/// clear of its swatch and of the entries before it.
pub(super) fn label_shift(styles: &vectorcraft_doc::LabelStyles, s: &str, system_layout: bool) -> (f64, f64) {
    kept_and_default_bounds(styles, s, system_layout)
        .map_or((0.0, 0.0), |(kept, default)| ((default.x0 - kept.x0).max(0.0), (default.y0 - kept.y0).max(0.0)))
}

/// How much further down legend label `s` reaches with its kept paragraph attributes than with the defaults (more
/// leading, or space before or after), after the legend moves it down by [`label_shift`], so the legend makes room
/// for it.
pub(super) fn label_extra_height(styles: &vectorcraft_doc::LabelStyles, s: &str, system_layout: bool) -> f64 {
    kept_and_default_bounds(styles, s, system_layout).map_or(0.0, |(kept, default)| (kept.y1 + (default.y0 - kept.y0).max(0.0) - default.y1).max(0.0))
}

/// A label with nothing to show (empty, or only `|` line breaks).
pub(super) fn blank_label(s: &str) -> bool {
    s.chars().all(|c| c == '|')
}

/// Text node `t` with its characters painted `colour`.
pub(super) fn recolour_text(t: &Arc<Node>, colour: Color) -> Arc<Node> {
    let mut n = (**t).clone();
    if let NodeKind::Text(text) = &mut n.kind {
        for run in &mut text.runs {
            run.style.fill = Paint::solid(colour);
        }
    }
    Arc::new(n)
}

/// The stretches of two or more points between blank cells, which a line connects.
pub(super) fn runs(pts: &[Option<Point>]) -> Vec<Vec<Point>> {
    pts.split(Option::is_none).map(|r| r.iter().flatten().copied().collect::<Vec<_>>()).filter(|r| r.len() > 1).collect()
}

pub(super) fn polyline(pts: &[Point], closed: bool) -> PathData {
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
