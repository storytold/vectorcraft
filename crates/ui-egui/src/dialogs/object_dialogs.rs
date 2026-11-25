//! The object dialogs' shared record, their per-frame pump and the menu routing for them.

use super::path_ops::PathOp;
use super::simplify_slider::StripKind;
use super::{
    blend_options, dimension, flare_options, graph_data, graph_design, graph_type, indexed_color, live_paint_gaps, object_mosaic, option_form,
    path_ops, perspective_grid, perspective_presets, saved_names, shapes, simplify_slider, slices, smooth_slider, step_repeat, transform,
    transform_each, width_point,
};
use crate::VectorcraftApp;
use crate::dialogs::preview::LivePreview;

/// What the object dialogs keep between frames. Held OK records are `app.held`, not here.
#[derive(Default)]
pub(crate) struct ObjectDialogs {
    /// The open Move, Rotate, Scale, Reflect or Shear dialog's preview record.
    pub transform: Option<transform::Open>,
    /// The open Transform Each dialog's preview record.
    pub each: Option<transform_each::Open>,
    /// The open Step and Repeat dialog's preview record.
    pub step: Option<step_repeat::Open>,
    /// The open Average, Average & Join, Offset Path and Simplify dialogs' preview records.
    pub path: Vec<path_ops::Open>,
    /// The open Split Into Grid dialog's record.
    pub split: Option<path_ops::SplitOpen>,
    /// The Simplify or Smooth strip on the canvas.
    pub strip: Option<simplify_slider::Strip>,
    /// The open Flare Tool Options dialog's record.
    pub flare: Option<flare_options::Open>,
    /// The open shape dialog's Constrain state, shared with its link control.
    pub shape_link: bool,
    /// The open Blend Options dialog's live preview.
    pub blend: Option<LivePreview>,
    /// The open Indexed Color dialog's live preview.
    pub indexed: Option<LivePreview>,
    /// The open Gap Options dialog's live preview.
    pub gaps: Option<live_paint_gaps::Open>,
    /// The Properties section's pending colour pick: dialog id, field key and target dimension.
    pub dimension: Option<dimension::Pick>,
}

/// Pushes app data into the open dialogs that read it, so a caller reaching an open dialog sees it current.
pub(crate) fn sync(app: &mut VectorcraftApp) {
    object_mosaic::sync(app);
    width_point::sync(app);
}

/// One frame of every object dialog's own step. Each module takes the host itself.
pub(crate) fn pump(app: &mut VectorcraftApp, ctx: &egui::Context) {
    sync(app);
    transform::pump(app, ctx);
    transform_each::pump(app, ctx);
    step_repeat::pump(app, ctx);
    path_ops::pump(app, ctx);
    shapes::pump(app, ctx);
    flare_options::pump(app, ctx);
    dimension::pump(app, ctx);
    option_form::pump(app, ctx);
    live_paint_gaps::pump(app, ctx);
    graph_type::pump(app, ctx);
    graph_data::pump(app, ctx);
    graph_design::pump(app, ctx);
    perspective_grid::pump(app, ctx);
    perspective_presets::pump(app, ctx);
    slices::pump(app, ctx);
    blend_options::pump(app, ctx);
    saved_names::pump(app, ctx);
    object_mosaic::pump(app, ctx);
    width_point::pump(app, ctx);
    indexed_color::pump(app, ctx);
}

/// Opens the dialog of menu command `id` when it is an object dialog; `false` when `id` is not one.
/// An opener's failure goes to the status line and still counts as handled.
pub(crate) fn open_from_menu(app: &mut VectorcraftApp, id: &str) -> bool {
    if let Some(kind) = transform::Kind::of_command(id) {
        transform::open(app, kind, &serde_json::Value::Null);
        return true;
    }
    match id {
        "object.transformEach" => transform_each::open(app),
        step_repeat::CMD => step_repeat::open(app),
        "path.average" => path_ops::open(app, PathOp::Average),
        "path.averageJoin" => path_ops::open(app, PathOp::AverageJoin),
        "object.path.offsetPath" => path_ops::open(app, PathOp::OffsetPath),
        "object.path.splitIntoGrid" => path_ops::open_split(app),
        "object.path.simplify" if path_ops::retained_simplify(app) => path_ops::open(app, PathOp::Simplify),
        "object.path.simplify" => simplify_slider::open(app, StripKind::Simplify),
        "object.path.smooth" => smooth_slider::open(app),
        _ => return false,
    }
    true
}
