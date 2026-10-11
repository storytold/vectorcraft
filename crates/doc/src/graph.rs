//! Graphs (Illustrator's graph tools): a group whose art is generated from a [`GraphSpec`].

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use vectorcraft_geom::Rect;

/// Most categories (rows) and series (columns) a graph's data grid holds: the grid is built in full for drawing and
/// for the data window, so a file can't make it allocate without bound.
pub const MAX_GRAPH_CATEGORIES: usize = 10_000;
pub const MAX_GRAPH_SERIES: usize = 256;

/// The nine graph types, in the Graph tool group's order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GraphKind {
    #[default]
    Column,
    StackedColumn,
    Bar,
    StackedBar,
    Line,
    Area,
    Scatter,
    Pie,
    Radar,
}

impl GraphKind {
    pub const ALL: [GraphKind; 9] = [
        GraphKind::Column,
        GraphKind::StackedColumn,
        GraphKind::Bar,
        GraphKind::StackedBar,
        GraphKind::Line,
        GraphKind::Area,
        GraphKind::Scatter,
        GraphKind::Pie,
        GraphKind::Radar,
    ];
    pub fn id(self) -> &'static str {
        match self {
            GraphKind::Column => "column",
            GraphKind::StackedColumn => "stackedColumn",
            GraphKind::Bar => "bar",
            GraphKind::StackedBar => "stackedBar",
            GraphKind::Line => "line",
            GraphKind::Area => "area",
            GraphKind::Scatter => "scatter",
            GraphKind::Pie => "pie",
            GraphKind::Radar => "radar",
        }
    }
    /// Parse an id, also accepting the tool ids (`columnGraph`, `stackedBarGraph`…).
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.strip_suffix("Graph").unwrap_or(s);
        Self::ALL.into_iter().find(|k| k.id().eq_ignore_ascii_case(s))
    }
    pub fn label(self) -> &'static str {
        match self {
            GraphKind::Column => "Column",
            GraphKind::StackedColumn => "Stacked Column",
            GraphKind::Bar => "Bar",
            GraphKind::StackedBar => "Stacked Bar",
            GraphKind::Line => "Line",
            GraphKind::Area => "Area",
            GraphKind::Scatter => "Scatter",
            GraphKind::Pie => "Pie",
            GraphKind::Radar => "Radar",
        }
    }
    /// Whether a series of this type can be drawn in a graph of type `graph` (Combine different graph types): the
    /// column, stacked column, line and area types share a vertical value axis, the bar types a horizontal one.
    /// Scatter, pie and radar lay their data out on their own and take no other types.
    pub fn combines_with(self, graph: GraphKind) -> bool {
        use GraphKind::*;
        let vertical = |k| matches!(k, Column | StackedColumn | Line | Area);
        let horizontal = |k| matches!(k, Bar | StackedBar);
        self == graph || (vertical(self) && vertical(graph)) || (horizontal(self) && horizontal(graph))
    }
}

/// A graph design (Object › Graph › Design…): named art that graphs draw in place of their default marks.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GraphDesign {
    pub name: String,
    pub art: std::sync::Arc<crate::Node>,
}

impl GraphDesign {
    /// Most designs a document keeps, objects in one design's art, and characters in a design's name.
    pub const MAX: usize = 1000;
    pub const MAX_NODES: usize = 2000;
    pub const MAX_NAME: usize = 64;

    /// A design name cleaned up: trimmed, without control characters, at most [`Self::MAX_NAME`] long; `None` when
    /// nothing is left.
    pub fn clean_name(name: &str) -> Option<String> {
        let n: String = name.trim().chars().filter(|c| !c.is_control()).take(Self::MAX_NAME).collect();
        Some(n.trim().to_string()).filter(|n| !n.is_empty())
    }

    /// `art` as plain design art: a graph inside it becomes the plain group it draws as, so a marker never holds a
    /// graph of its own (that its commands would then edit).
    pub fn plain_art(art: &crate::Node) -> crate::Node {
        let mut n = art.clone();
        n.graph = None;
        n.series_index = None;
        if let Some(ch) = n.children_mut() {
            let old = std::mem::take(ch);
            *ch = old.iter().map(|c| std::sync::Arc::new(Self::plain_art(c))).collect();
        }
        n
    }
}

impl crate::Document {
    /// The graph designs a file gave, made safe to list and draw: at most [`GraphDesign::MAX`], each with a clean,
    /// unique name, art of at most [`GraphDesign::MAX_NODES`] objects and no graph inside.
    pub fn tidy_graph_designs(&mut self) {
        let mut kept: Vec<GraphDesign> = vec![];
        for d in std::mem::take(&mut self.graph_designs) {
            if kept.len() >= GraphDesign::MAX {
                break;
            }
            let Some(name) = GraphDesign::clean_name(&d.name) else { continue };
            if kept.iter().any(|k| k.name == name) || d.art.count() > GraphDesign::MAX_NODES {
                continue;
            }
            kept.push(GraphDesign { name, art: std::sync::Arc::new(GraphDesign::plain_art(&d.art)) });
        }
        self.graph_designs = kept;
    }
}

/// Graph Type › Value Axis: which side of the plot the value axis is drawn on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ValueAxisSide {
    #[default]
    Left,
    Right,
    Both,
}

impl ValueAxisSide {
    pub fn id(self) -> &'static str {
        match self {
            ValueAxisSide::Left => "left",
            ValueAxisSide::Right => "right",
            ValueAxisSide::Both => "both",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        [ValueAxisSide::Left, ValueAxisSide::Right, ValueAxisSide::Both].into_iter().find(|k| k.id().eq_ignore_ascii_case(s))
    }
    fn is_left(&self) -> bool {
        *self == ValueAxisSide::Left
    }
}

/// Graph Type › Tick Marks › Length: no tick marks, short ones outside the axis, or lines across the whole plot.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TickLength {
    None,
    #[default]
    Short,
    Full,
}

impl TickLength {
    pub fn id(self) -> &'static str {
        match self {
            TickLength::None => "none",
            TickLength::Short => "short",
            TickLength::Full => "full",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        [TickLength::None, TickLength::Short, TickLength::Full].into_iter().find(|k| k.id().eq_ignore_ascii_case(s))
    }
    fn is_short(&self) -> bool {
        *self == TickLength::Short
    }
    fn is_none(&self) -> bool {
        *self == TickLength::None
    }
}

/// The fill and stroke captured from one series' generated marks. An absent paint keeps whatever
/// the generator draws for that part (greyscale for a series fill, the part's own stroke otherwise).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SeriesPaint {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fill: Option<vectorcraft_color::Paint>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stroke: Option<vectorcraft_color::Paint>,
    /// Stroke weight in points, kept with [`Self::stroke`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stroke_width: Option<f64>,
}

/// Graph data and options. `rows` are categories (one per row of the Graph Data window), each
/// holding one value per series; `series` are the legend labels (the first row of the window).
/// Scatter graphs read each series as (y, x) column pairs, like Illustrator.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct GraphSpec {
    /// Paint taken from the series' marks (Group Selection). One entry per series index; missing
    /// entries keep the generator's own fill and stroke.
    pub series_paints: Vec<SeriesPaint>,
    pub kind: GraphKind,
    /// Graph types chosen for single series (Combine different graph types), by series index; `None` and missing
    /// entries draw as [`Self::kind`].
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub series_kinds: Vec<Option<GraphKind>>,
    /// The plot rectangle (the area the graph tool dragged), document points.
    pub rect: Rect,
    pub series: Vec<String>,
    pub categories: Vec<String>,
    pub rows: Vec<Vec<f64>>,
    /// Blank cells as `[category, series]`: their `rows` entry is a 0 placeholder, so a file still reads (as zeros)
    /// in a version that doesn't know blanks. Cells past the end of a short row are blank too.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub blanks: Vec<[usize; 2]>,
    /// Column/bar width as a % of the available category width (90).
    pub column_width: f64,
    /// Cluster width as a % (80).
    pub cluster_width: f64,
    /// Legend on the right (Graph Type options: Add Legend Across Top = false).
    pub legend: bool,
    /// Value axis: tick count override (0 = automatic), min/max override.
    pub ticks: usize,
    pub axis_min: Option<f64>,
    pub axis_max: Option<f64>,
    /// Graph Type › Value Axis side. Column, stacked column, line, area and scatter graphs (the bar graphs' value axis
    /// runs along the bottom and stays there).
    #[serde(skip_serializing_if = "ValueAxisSide::is_left")]
    pub value_axis: ValueAxisSide,
    /// Graph Type › Separate Scales: with the value axis on both sides, the series on the right axis
    /// ([`Self::right_series`]) get a scale of their own there.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub separate_scales: bool,
    /// Indexes of the series assigned to the right value axis.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub right_series: Vec<usize>,
    /// The right value axis' tick values, used with separate scales: tick count (0 = automatic) and min/max override.
    #[serde(skip_serializing_if = "is_zero")]
    pub right_ticks: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub right_axis_min: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub right_axis_max: Option<f64>,
    /// Object › Graph › Marker: the graph design each series' data points (and legend swatch) are drawn with, by
    /// series index, for line, scatter and radar series; `None` and missing entries draw the default square.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub series_markers: Vec<Option<String>>,
    /// Graph Type › Tick Marks for the value axis (the left one, or the bottom one of bar graphs) and the right one:
    /// length, and tick marks per division (0 and 1 = one, at the labels).
    #[serde(skip_serializing_if = "TickLength::is_short")]
    pub tick_length: TickLength,
    #[serde(skip_serializing_if = "at_most_one")]
    pub tick_marks: usize,
    #[serde(skip_serializing_if = "TickLength::is_short")]
    pub right_tick_length: TickLength,
    #[serde(skip_serializing_if = "at_most_one")]
    pub right_tick_marks: usize,
    /// Tick Marks for the category axis (none by default), and Draw Tick Marks Between Labels.
    #[serde(skip_serializing_if = "TickLength::is_none")]
    pub category_tick_length: TickLength,
    #[serde(skip_serializing_if = "at_most_one")]
    pub category_tick_marks: usize,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub ticks_between_labels: bool,
    /// Graph Type › Add Labels: text before and after the value axes' numbers.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub prefix: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub suffix: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub right_prefix: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub right_suffix: String,
    /// Line, scatter and radar graphs: mark data points, connect them.
    pub mark_points: bool,
    pub connect_points: bool,
    /// Line graphs: run the lines from edge to edge of the plot; off (the default for new graphs), the points sit
    /// at the centres of their categories, like the columns of a column graph. Graphs saved before the option
    /// existed were drawn edge to edge and keep it.
    #[serde(default = "edge_to_edge_before_the_option")]
    pub edge_to_edge: bool,
    /// Bounds of the art when it was last generated (detects moves/scales of the graph group).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub placed: Option<Rect>,
}

fn at_most_one(n: &usize) -> bool {
    *n <= 1
}

fn is_zero(n: &usize) -> bool {
    *n == 0
}

/// Edge-to-Edge Lines for a graph saved before the option existed: on, as it was drawn then.
fn edge_to_edge_before_the_option() -> bool {
    true
}

impl GraphSpec {
    /// The type series `s` is drawn as: its own type when it combines with the graph's, else the graph's.
    pub fn series_kind(&self, s: usize) -> GraphKind {
        self.series_kinds.get(s).copied().flatten().filter(|k| k.combines_with(self.kind)).unwrap_or(self.kind)
    }

    /// The data with blank cells as `None`, every row as long as the longest (at most [`MAX_GRAPH_CATEGORIES`] rows
    /// of [`MAX_GRAPH_SERIES`] cells).
    pub fn cells(&self) -> Vec<Vec<Option<f64>>> {
        let width = self.rows.iter().map(Vec::len).max().unwrap_or(0).min(MAX_GRAPH_SERIES);
        let blank: HashSet<[usize; 2]> = self.blanks.iter().copied().collect();
        self.rows
            .iter()
            .take(MAX_GRAPH_CATEGORIES)
            .enumerate()
            .map(|(c, r)| (0..width).map(|s| r.get(s).copied().filter(|_| !blank.contains(&[c, s]))).collect())
            .collect()
    }

    /// Store `cells`, a blank (`None`) as a 0 placeholder listed in `blanks`.
    pub fn set_cells(&mut self, cells: Vec<Vec<Option<f64>>>) {
        self.blanks =
            cells.iter().enumerate().flat_map(|(c, r)| r.iter().enumerate().filter(|(_, v)| v.is_none()).map(move |(s, _)| [c, s])).collect();
        self.rows = cells.into_iter().map(|r| r.into_iter().map(|v| v.unwrap_or(0.0)).collect()).collect();
    }
}

impl Default for GraphSpec {
    fn default() -> Self {
        Self {
            series_paints: vec![],
            kind: GraphKind::Column,
            series_kinds: vec![],
            rect: Rect::new(0.0, 0.0, 200.0, 150.0),
            series: vec![],
            categories: vec![],
            rows: vec![vec![1.0]],
            blanks: vec![],
            column_width: 90.0,
            cluster_width: 80.0,
            legend: true,
            ticks: 0,
            axis_min: None,
            axis_max: None,
            value_axis: ValueAxisSide::Left,
            separate_scales: false,
            right_series: vec![],
            right_ticks: 0,
            right_axis_min: None,
            right_axis_max: None,
            series_markers: vec![],
            tick_length: TickLength::Short,
            tick_marks: 1,
            right_tick_length: TickLength::Short,
            right_tick_marks: 1,
            category_tick_length: TickLength::None,
            category_tick_marks: 1,
            ticks_between_labels: false,
            prefix: String::new(),
            suffix: String::new(),
            right_prefix: String::new(),
            right_suffix: String::new(),
            mark_points: true,
            connect_points: true,
            edge_to_edge: false,
            placed: None,
        }
    }
}
