use super::axis::LABEL_SIZE;
use super::columns::{Allowance, ColumnArt};
use super::markers::fit_design;
use super::series::{default_series_paint, series_marks};
use super::*;

pub(super) struct Gen<'a> {
    pub(super) d: &'a mut Document,
    pub(super) out: Vec<Arc<Node>>,
    /// The labels' kept character styles.
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
        self.path(PathData::from_bezpath(&bp), Paint::None, Paint::solid(Color::BLACK), 0.5)
    }
    pub(super) fn text(&mut self, role: Role, at: Point, s: &str, justify: Justify) -> Arc<Node> {
        let style = label_style(self.styles, role);
        let mut t = TextObject::point(at, s, style);
        t.para.justify = justify;
        t.system_layout = self.d.setup.system_layout;
        refresh_bounds(&mut t);
        Arc::new(Node::new(self.d.alloc_id(), NodeKind::Text(Box::new(t))))
    }
    /// A category or series label: a `|` in it starts a new line (Add graph labels). `middle`: the lines are centred
    /// on `at` (labels beside an axis or in a wedge) instead of running down from it.
    pub(super) fn label(&mut self, role: Role, at: Point, s: &str, justify: Justify, middle: bool) -> Arc<Node> {
        let lines = s.split('|').count();
        let at = if middle { Point::new(at.x, at.y - (lines - 1) as f64 * label_size(self.styles, role) * 1.2 / 2.0) } else { at };
        self.text(role, at, &s.replace('|', "\n"), justify)
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
            // The band takes the series' colour, set as a fill or, on a thin line, as its stroke; a thin black outline.
            let slot = g.series_paints.get(s);
            let fill = slot.and_then(|p| p.fill.clone().or_else(|| p.stroke.clone())).unwrap_or_else(|| default_series_paint(s));
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

pub(super) const FILLED_LINE_BUDGET: usize = 20_000;

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

/// How wide label `s` is set (its widest line), without adding it to a document.
pub(super) fn label_width(styles: &vectorcraft_doc::LabelStyles, s: &str) -> f64 {
    let style = label_style(styles, Role::Legend);
    let mut t = TextObject::point(Point::ZERO, &s.replace('|', "\n"), style);
    refresh_bounds(&mut t);
    Node::new(NodeId(0), NodeKind::Text(Box::new(t))).geometric_bounds().map_or(0.0, |b| b.width())
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
