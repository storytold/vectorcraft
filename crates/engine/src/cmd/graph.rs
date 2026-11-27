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
use vectorcraft_doc::{
    Appearance, CharStyle, ColumnDesign, ColumnType, Document, Fractions, GraphDesign, GraphKind, GraphSpec, Justify, Node, NodeId, NodeKind,
    TextObject, TickLength, ValueAxisSide,
};
use vectorcraft_geom::{Affine, BezPath, FillRule, PathData, Point, Rect, shapes};

use super::typecmd::refresh_bounds;
use super::{
    CommandSpec, EngineError, Result, Session, bad, cmd, f64_req, has_doc, has_selection, named_id, number_or, ok, str_param, switch_or, text_param,
};

mod axis;
mod columns;
mod commands;
mod data;
mod designs;
mod edit;
mod generate;
mod labels;
mod markers;
mod series;

#[cfg(test)]
use axis::{nice_axis, tick_values};
#[cfg(test)]
use columns::{fill_totals, total_text};
pub use commands::specs;
pub use data::{MAX_GRAPH_DATA_FILE, graph_cell_text, graph_data_from_grid, graph_data_text, graph_grid_from_text};
pub(crate) use data::{apply_csv, graph_csv_from_file, parse_csv};
#[cfg(test)]
use data::{fmt_value, to_csv};
pub(crate) use designs::{holds_designs, redraw_detached, redraw_symbols_with_designs, redraw_with_designs, refresh_graphs};
pub use edit::{GraphTypeSubject, graph_type_subject};
pub(crate) use edit::{edit_graph_art, graphs_holding, keep_placed};
pub use generate::calculated_axes;
#[cfg(test)]
use generate::generate;
#[cfg(test)]
use series::default_series_color;
pub(crate) use series::{
    LabelTextChange, RowWrites, capture_label_anti_alias, capture_label_changes, capture_label_kind_rows, capture_label_object_opacity,
    capture_label_paragraphs, capture_label_rows, capture_label_styles, capture_label_styles_given, capture_label_text, capture_line_changes,
    capture_line_fill_stroke_rows, capture_line_kind_rows, capture_line_paint_writes, capture_line_row_writes, capture_line_rows,
    capture_series_object_opacity, capture_series_opacity, capture_series_top_paints, follow_wedge_shades, graph_part_members, is_design_art,
    is_graph_line, label_object_opacity_differs, label_rows, labels_in, paint_targets, plain_labels, series_group_marks, series_group_marks_where,
    series_groups_among, series_marks_in, series_object_opacity_differs, shown_paint, wedge_shades,
};

#[cfg(test)]
mod tests;
