//! Graphs (Illustrator's graph tools): a group whose art is generated from a [`GraphSpec`].

use serde::{Deserialize, Serialize};
use vectorcraft_geom::Rect;

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
    /// The plot rectangle (the area the graph tool dragged), document points.
    pub rect: Rect,
    pub series: Vec<String>,
    pub categories: Vec<String>,
    pub rows: Vec<Vec<f64>>,
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
    /// Line graphs: mark data points, connect them, fill (area-like) the lines.
    pub mark_points: bool,
    pub connect_points: bool,
    /// Bounds of the art when it was last generated (detects moves/scales of the graph group).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub placed: Option<Rect>,
}

impl Default for GraphSpec {
    fn default() -> Self {
        Self {
            series_paints: vec![],
            kind: GraphKind::Column,
            rect: Rect::new(0.0, 0.0, 200.0, 150.0),
            series: vec![],
            categories: vec![],
            rows: vec![vec![1.0]],
            column_width: 90.0,
            cluster_width: 80.0,
            legend: true,
            ticks: 0,
            axis_min: None,
            axis_max: None,
            mark_points: true,
            connect_points: true,
            placed: None,
        }
    }
}
