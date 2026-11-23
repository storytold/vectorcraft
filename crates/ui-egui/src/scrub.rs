//! Scrubbing numeric fields (#400): a horizontal
//! drag on a numeric field's label (`W:`, `Opacity:`, the Character panel's size icon) steps the
//! field's value by one unit every [`PX_PER_STEP`] points, ten with Shift and a tenth with
//! Ctrl/Cmd, as the arrow keys step a focused field; a click on the label focuses the field. A
//! label that is a link (the Control bar's `Stroke:` and `Opacity:`, which open their panels;
//! #4088) or opens a panel (its `X:`, `Y:`, `W:` and `H:`, which open the Transform panel) scrubs
//! its field the same way, and a click on it still opens the panel. The text box
//! itself never scrubs, and a field without a label doesn't.
//!
//! The panels apply each value the drag passes as they apply a typed one, so the art follows the
//! drag live; the app holds an undo group open meanwhile ([`begin_frame`], [`end_frame`]), so the
//! drag is one undo step and Escape takes it back. Preferences › General › Scrub Numeric Fields by
//! Dragging turns it off.

use std::ops::RangeInclusive;

pub use craft_ui::numeric::ScrubPhase as Phase;
use craft_ui::numeric::{FieldScrubParams, Scrubber};
use egui::{Context, Id, Rect, Response, Ui};

use crate::VectorcraftApp;

/// Turn scrubbing on or off (General › Scrub Numeric Fields by Dragging; on until set).
pub(crate) fn set_enabled(ctx: &Context, on: bool) {
    Scrubber::set_enabled(ctx, on);
}

pub(crate) fn phase(ctx: &Context) -> Phase {
    Scrubber::phase(ctx)
}

/// Before the panels draw: while a field is scrubbed, the edits its values make are one undo step,
/// or part of the step of an undo group open in the active document
/// ([`vectorcraft_engine::Session::begin_undo_scope`]). (The drag's first frame applies nothing, so
/// the step begins before its first value.)
pub(crate) fn begin_frame(app: &mut VectorcraftApp, ctx: &Context) {
    if !app.scrub_group && matches!(phase(ctx), Phase::Dragging | Phase::Released) {
        app.scrub_scope = app.session.begin_undo_scope();
        app.scrub_group = true;
    }
}

/// After the panels: a finished drag ends its undo step in the document it began in, even when
/// another document is active by then; Escape undoes the drag's edits.
pub(crate) fn end_frame(app: &mut VectorcraftApp, ctx: &Context) {
    let phase = phase(ctx);
    if !app.scrub_group || phase == Phase::Dragging {
        return;
    }
    app.scrub_group = false;
    if let Some(scope) = app.scrub_scope.take() {
        app.session.end_undo_scope(scope, phase == Phase::Cancelled);
    }
    app.sync_views();
}

/// Note the label (text or icon) drawn at `rect`: a numeric field drawn right after it on the
/// same line takes it as its label, which scrubs the field.
pub(crate) fn note_label(ui: &Ui, rect: Rect) {
    Scrubber::note_label(ui, rect);
}

/// A link that labels the numeric field drawn after it (it opens a panel when clicked): a drag on
/// it scrubs the field, and a click stays the link's. Its text can't be selected, which a drag
/// across it would otherwise do.
pub(crate) fn link_label(ui: &mut Ui, text: egui::RichText) -> Response {
    Scrubber::link_label(ui, text)
}

/// A field's label that opens a panel when clicked (the Control bar's X:, Y:, W: and H:, which open
/// the Transform panel): a drag on it scrubs the field after it, and a click stays the label's, as
/// with [`link_label`]. Unlike a link it is not a tab stop of its own, so Tab still moves from one
/// field to the next. The pointing hand shows over it.
pub(crate) fn panel_label(ui: &mut Ui, text: egui::RichText) -> Response {
    Scrubber::panel_label(ui, text)
}

/// Scrub numeric field `field`, its box at `rect`, showing `text`: `value` in its units, rounded
/// to `decimals` places and kept within `bounds` (#3370). → The value a drag on the field's label
/// steps it to, or the value it started at when Escape cancels the drag. A click on the label (or a
/// press held without stepping) focuses the field with all of `text` selected. Nothing for a field
/// without a label, disabled or showing no value (objects that differ).
pub(crate) fn field(ui: &Ui, field: Id, rect: Rect, text: &str, value: Option<f64>, decimals: i32, bounds: &RangeInclusive<f64>) -> Option<f64> {
    Scrubber::field(ui, FieldScrubParams { id: field, rect, text, value, decimals, bounds }, |ui, f, t| {
        crate::widgets::focus_field(ui, f, t);
    })
}
