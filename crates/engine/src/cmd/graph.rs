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
pub(crate) use data::apply_csv;
#[cfg(test)]
use data::{fmt_value, parse_csv, to_csv};
#[cfg(test)]
use generate::generate;
#[cfg(test)]
use series::default_series_color;
pub(crate) use series::{
    capture_label_anti_alias, capture_label_paragraphs, capture_label_styles, capture_series_object_opacity, capture_series_opacity,
    capture_series_paints, capture_series_top_paints, paint_targets, series_object_opacity_differs, shown_paint,
};

#[cfg(test)]
mod tests;
