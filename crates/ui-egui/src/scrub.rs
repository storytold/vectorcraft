//! Scrubbing numeric fields (#400, beyond Illustrator; Photoshop's scrubby labels): a horizontal
//! drag on a numeric field's label (`W:`, `Opacity:`, the Character panel's size icon) steps the
//! field's value by one unit every [`PX_PER_STEP`] points, ten with Shift and a tenth with
//! Ctrl/Cmd, as the arrow keys step a focused field; a click on the label focuses the field. The
//! text box itself never scrubs, and a field without a label doesn't.
//!
//! The panels apply each value the drag passes as they apply a typed one, so the art follows the
//! drag live; the app holds an undo group open meanwhile ([`begin_frame`], [`end_frame`]), so the
//! drag is one undo step and Escape takes it back. Preferences › General › Scrub Numeric Fields by
//! Dragging turns it off.

use std::ops::RangeInclusive;

use egui::{Context, CursorIcon, Id, Key, Rect, Response, Sense, Ui};

use crate::VectorcraftApp;

/// Points of horizontal drag per step.
pub const PX_PER_STEP: f32 = 2.0;
/// The widest gap between a label and the field it names.
const LABEL_GAP: f32 = 64.0;

/// Where the scrub in progress is kept (egui memory).
fn state_id() -> Id {
    Id::unique("numeric-field-scrub")
}

/// Where the last label drawn waits for its field.
fn label_id() -> Id {
    Id::unique("numeric-field-label")
}

/// Temp-memory key for the numeric-field scrub preference.
fn enabled_id() -> Id {
    Id::unique("numeric-field-scrub-enabled")
}

/// Turn scrubbing on or off (General › Scrub Numeric Fields by Dragging; on until set).
pub(crate) fn set_enabled(ctx: &Context, on: bool) {
    ctx.data_mut(|d| d.insert_temp(enabled_id(), on));
}

fn enabled(ctx: &Context) -> bool {
    ctx.data(|d| d.get_temp(enabled_id())).unwrap_or(true)
}

/// The scrub in progress.
#[derive(Clone, Copy, Debug)]
struct Scrub {
    /// The zone dragged: a field or its label.
    zone: Id,
    /// The field's value when the drag started (in its units), which Escape puts back.
    start: f64,
    /// Drag not applied yet, in the field's units.
    pending: f64,
    /// The step `pending` counts in (it changes with Shift and Ctrl/Cmd).
    step: f64,
    /// A value was applied.
    moved: bool,
    /// Escape ended the drag.
    cancelled: bool,
}

/// Where the scrub of a numeric field stands this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Phase {
    Idle,
    Dragging,
    /// Released this frame.
    Released,
    /// Escape ended it this frame.
    Cancelled,
}

pub(crate) fn phase(ctx: &Context) -> Phase {
    let Some(s) = ctx.data(|d| d.get_temp::<Scrub>(state_id())) else { return Phase::Idle };
    if ctx.drag_stopped_id() == Some(s.zone) {
        if s.cancelled { Phase::Cancelled } else { Phase::Released }
    } else if ctx.is_being_dragged(s.zone) {
        Phase::Dragging
    } else {
        Phase::Idle
    }
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

/// A field's label drawn this pass, waiting for the field after it.
#[derive(Clone, Copy, Debug)]
struct Label {
    rect: Rect,
    pass: u64,
}

/// Note the label (text or icon) drawn at `rect`: a numeric field drawn right after it on the
/// same line takes it as its label, which scrubs the field.
pub(crate) fn note_label(ui: &Ui, rect: Rect) {
    let pass = ui.ctx().cumulative_pass_nr();
    ui.data_mut(|d| d.insert_temp(label_id(), Label { rect, pass }));
}

/// The label noted just before the field at `field`, on its line.
fn take_label(ui: &Ui, field: Rect) -> Option<Rect> {
    let l = ui.data_mut(|d| {
        let l = d.get_temp::<Label>(label_id());
        d.remove::<Label>(label_id());
        l
    })?;
    let gap = field.left() - l.rect.right();
    (l.pass == ui.ctx().cumulative_pass_nr() && (-1.0..=LABEL_GAP).contains(&gap) && field.y_range().contains(l.rect.center().y)).then_some(l.rect)
}

/// Scrub numeric field `field`, its box at `rect`, showing `text`: `value` in its units, rounded
/// to `decimals` places and kept within `bounds` (#3370). → The value a drag on the field's label
/// steps it to, or the value it started at when Escape cancels the drag. A click on the label (or a
/// press held without stepping) focuses the field with all of `text` selected. Nothing for a field
/// without a label, disabled or showing no value (objects that differ).
pub(crate) fn field(ui: &Ui, field: Id, rect: Rect, text: &str, value: Option<f64>, decimals: i32, bounds: &RangeInclusive<f64>) -> Option<f64> {
    let label = take_label(ui, rect)?;
    let value = value.filter(|v| v.is_finite())?;
    if !enabled(ui.ctx()) || !ui.is_enabled() {
        return None;
    }
    // On top of the label, which may sense clicks itself (selectable text).
    let resp = ui.interact(label, field.with("scrub-label"), Sense::click_and_drag());
    drag(ui, &resp, field, text, value, decimals).map(|v| v.max(*bounds.start()).min(*bounds.end()))
}

/// What a drag on `resp`, the label of field `field`, does this frame.
fn drag(ui: &Ui, resp: &Response, field: Id, text: &str, value: f64, decimals: i32) -> Option<f64> {
    if resp.hovered() || resp.dragged() {
        ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
    }
    if resp.clicked() {
        crate::widgets::focus_field(ui, field, text);
        return None;
    }
    if resp.drag_started() {
        // Typing elsewhere ends, as a click elsewhere ends it.
        ui.memory_mut(|m| m.stop_text_input());
        let s = Scrub { zone: resp.id, start: value, pending: 0.0, step: 0.0, moved: false, cancelled: false };
        ui.data_mut(|d| d.insert_temp(state_id(), s));
    }
    if !(resp.dragged() || resp.drag_stopped()) {
        return None;
    }
    let mut s = ui.data(|d| d.get_temp::<Scrub>(state_id())).filter(|s| s.zone == resp.id && !s.cancelled)?;
    // egui ends the drag on Escape: the field goes back to where it started.
    let out = if ui.input_mut(|i| i.consume_key(i.modifiers, Key::Escape)) {
        s.cancelled = true;
        s.moved.then_some(s.start)
    } else if resp.drag_stopped() {
        // Held without stepping: a slow click.
        if !s.moved {
            crate::widgets::focus_field(ui, field, text);
        }
        None
    } else {
        step(ui, resp, &mut s, value, decimals)
    };
    ui.data_mut(|d| d.insert_temp(state_id(), s));
    out
}

/// The value this frame's drag steps `value` to, if it makes a whole step.
fn step(ui: &Ui, resp: &Response, s: &mut Scrub, value: f64, decimals: i32) -> Option<f64> {
    let mods = ui.input(|i| i.modifiers);
    let size = if mods.command {
        0.1
    } else if mods.shift {
        10.0
    } else {
        1.0
    };
    // A count steps by whole numbers: Ctrl/Cmd makes it slower instead.
    let step = f64::max(size, 10f64.powi(-decimals));
    if step != s.step {
        (s.step, s.pending) = (step, 0.0);
    }
    s.pending += f64::from(resp.drag_delta().x / PX_PER_STEP) * size;
    // (Nudged off the float error of summed tenths.)
    let n = (s.pending / step + s.pending.signum() * 1e-9).trunc();
    // The drag's first frame only starts it: the app opens its undo group on the next.
    if n == 0.0 || resp.drag_started() {
        return None;
    }
    s.pending -= n * step;
    s.moved = true;
    Some(crate::widgets::round_to(value + n * step, decimals))
}
