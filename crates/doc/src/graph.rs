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
            mark_points: true,
            connect_points: true,
            edge_to_edge: false,
            placed: None,
        }
    }
}
