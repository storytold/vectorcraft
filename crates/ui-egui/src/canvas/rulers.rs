//! The rulers of a canvas body: what they read (units, zero point) and the document edits their
//! gestures make (guides dragged out of a strip, the zero point dragged out of the corner).

use craft::behavior::{Axis, EditEvent, GesturePhase, Mixed, RulerUnits, TickLabels, Ticks};
use craft::ui::canvas::{CanvasRulers, GuideDrag, RulerText, Rulers, RulersOutput};
use craft::ui::contract::Bind;
use craft::ui::runtime::env::{LengthUnits, UnitKind};
use egui::Ui;
use serde_json::{Value, json};
use vectorcraft_doc::Unit;
use vectorcraft_geom::Point;
use vectorcraft_tools::{Mods, PointerEvent, PointerKind};

use super::BodyKind;
use crate::VectorcraftApp;
use crate::doc_windows::Win;

#[cfg(test)]
mod tests;

/// Pixels per inch that sizes a `Pixels` length: a pixel is a point.
const PIXELS_PER_INCH: f64 = 72.0;

/// The units a right click on the rulers offers, in the order of the document's units.
const UNITS: [UnitKind; 9] = [
    UnitKind::Points,
    UnitKind::Picas,
    UnitKind::Inches,
    UnitKind::Millimetres,
    UnitKind::Centimetres,
    UnitKind::Pixels,
    UnitKind::Feet,
    UnitKind::Metres,
    UnitKind::Yards,
];

/// The framework's unit kind of a document unit. Feet and Feet-Inches both count feet.
pub(crate) fn unit_kind(unit: Unit) -> UnitKind {
    match unit {
        Unit::Points => UnitKind::Points,
        Unit::Picas => UnitKind::Picas,
        Unit::Inches => UnitKind::Inches,
        Unit::Millimeters => UnitKind::Millimetres,
        Unit::Centimeters => UnitKind::Centimetres,
        Unit::Pixels => UnitKind::Pixels,
        Unit::FeetInches | Unit::Feet => UnitKind::Feet,
        Unit::Yards => UnitKind::Yards,
        Unit::Meters => UnitKind::Metres,
        Unit::Q => UnitKind::Q,
        Unit::Ha => UnitKind::Ha,
    }
}

/// The document unit a ruler menu row stands for. Searched from the end so Feet, not Feet-Inches, answers Feet.
fn unit_of(kind: UnitKind) -> Option<Unit> {
    Unit::ALL.iter().rev().copied().find(|unit| unit_kind(*unit) == kind)
}

/// The units the rulers' menu lists.
pub(crate) fn units() -> &'static [UnitKind] {
    &UNITS
}

/// Lengths stored in points and shown in `unit`, a bare number read as Numbers Without Units Are Points says.
fn lengths(app: &VectorcraftApp, unit: Unit) -> LengthUnits {
    crate::fields::bare_points(
        LengthUnits::new(UnitKind::Points, unit_kind(unit), PIXELS_PER_INCH),
        app.session.prefs.numbers_without_units_are_points,
    )
}

/// The document's General unit as the lengths its fields show.
pub(crate) fn general_lengths(app: &VectorcraftApp) -> LengthUnits {
    lengths(app, app.session.general_unit())
}

/// The Units ▸ Type unit as the lengths type fields show.
pub(crate) fn type_lengths(app: &VectorcraftApp) -> LengthUnits {
    lengths(app, app.session.type_unit())
}

/// The Units ▸ Stroke unit as the lengths stroke fields show.
pub(crate) fn stroke_lengths(app: &VectorcraftApp) -> LengthUnits {
    lengths(app, app.session.stroke_unit())
}

/// One axis counting `unit`, `per_doc` of its units to a document point, labelled with the signed position.
fn axis(unit: Unit, per_doc: f64) -> RulerText {
    RulerText::new(RulerUnits::new(per_doc, Ticks::decimal(unit == Unit::Pixels)).with_labels(TickLabels::Signed), unit_kind(unit))
}

/// Both axes in the General unit, from the ruler origin.
fn document(app: &VectorcraftApp) -> (RulerText, RulerText, Point) {
    let unit = app.session.general_unit();
    let text = axis(unit, 1.0 / unit.points());
    (text, text, app.ruler_origin())
}

/// The rulers of the document's General unit, for the fields that count positions as the rulers do.
pub(crate) fn general_rulers(app: &VectorcraftApp) -> CanvasRulers {
    let (x, y, zero) = document(app);
    CanvasRulers::new(x, y, zero)
}

/// What the rulers count: the active artboard in video pixels from its top-left corner with Video Rulers on (a pixel
/// as wide as the artboard's Video Ruler Pixel Aspect Ratio, a point tall), else the General unit from the ruler
/// origin.
fn scales(app: &VectorcraftApp) -> (RulerText, RulerText, Point) {
    let board = app.ui.view.video_rulers.then(|| crate::dialogs::artboard_options::active_index(app));
    match board.and_then(|i| app.session.active()?.doc.artboards.get(i)) {
        Some(ab) => {
            // A ratio read from a file may be anything.
            let ratio = if ab.video_ruler_pixel_aspect_ratio.is_finite() && ab.video_ruler_pixel_aspect_ratio > 0.0 {
                ab.video_ruler_pixel_aspect_ratio
            } else {
                1.0
            };
            (axis(Unit::Pixels, 1.0 / ratio), axis(Unit::Pixels, 1.0), Point::new(ab.rect.x0, ab.rect.y0))
        }
        None => document(app),
    }
}

/// The rulers' text for `Canvas::rulers`: the x axis, the y axis and the zero point of the window's document.
// PLAN: the zero point is the active document's; a pane of another document reads its own through canvas-w29.
pub(crate) fn texts(app: &VectorcraftApp, _win: Win) -> (RulerText, RulerText, Point) {
    scales(app)
}

/// The rulers are hidden: a zero-point drag they were showing for ends, so its snapping guides do not stay.
pub(super) fn hidden(app: &mut VectorcraftApp) {
    app.session.end_ruler_origin_drag();
}

/// The strips of a body's rulers, counting from `zero`. A pane's strips only paint. The zero point is given: the
/// document keeps it, and a drag edits it once, at its release ([`apply`]).
pub(super) fn strips(zero: Point, kind: BodyKind) -> Rulers<'static> {
    Rulers::new(tl!("Rulers"), Bind::given(zero)).units(units()).interactive(kind == BodyKind::Live)
}

/// A drag in progress, kept between passes: what it started in, and whether it no longer counts.
#[derive(Clone, Copy)]
struct Drag {
    /// The document it started in (`Document::uid`).
    doc: Option<u64>,
    /// Whether View › Trim View was on when it started.
    trim_view: bool,
    /// Latched when it stopped counting: it makes nothing, whatever follows.
    cancelled: bool,
}

/// The key of the zero-point drag's state.
const ZERO: &str = "ruler-zero-drag";
/// The key of the guide drag's state.
const GUIDE: &str = "ruler-guide-drag";

impl Drag {
    fn begin(app: &VectorcraftApp) -> Self {
        Self { doc: app.session.active().map(|d| d.uid), trim_view: app.ui.view.trim_view, cancelled: false }
    }

    /// Whether the drag can no longer edit: cancelled, or the document or Trim View changed since it began. The snap
    /// targets gathered at its start hold the guides or leave them out, so the origin could snap to a guide nobody sees.
    fn stale(&self, app: &VectorcraftApp) -> bool {
        self.cancelled || self.doc != app.session.active().map(|d| d.uid) || self.trim_view != app.ui.view.trim_view
    }
}

fn drag_state(ui: &Ui, key: &str) -> Option<Drag> {
    ui.data(|d| d.get_temp::<Drag>(egui::Id::unique(key)))
}

fn set_drag(ui: &Ui, key: &str, state: Option<Drag>) {
    let id = egui::Id::unique(key);
    ui.data_mut(|d| match state {
        Some(state) => {
            d.insert_temp(id, state);
        }
        None => d.remove::<Drag>(id),
    });
}

/// Runs a command; a failure is shown in the status line.
fn run(app: &mut VectorcraftApp, command: &str, params: Value) {
    if let Err(e) = app.run(command, params) {
        app.status(e.to_string());
    }
}

/// The artboard the zero point belongs to.
fn active_artboard(app: &VectorcraftApp) -> usize {
    app.view().map_or(0, |v| v.artboard)
}

/// Applies what a live body's rulers reported in its pass.
pub(super) fn apply(app: &mut VectorcraftApp, ui: &Ui, out: &RulersOutput) {
    if let Some(drag) = &out.guide {
        guide(app, ui, drag);
    }
    for event in out.zero.events.iter() {
        match event {
            EditEvent::Begin { .. } => zero_begin(app, ui),
            EditEvent::Update { value, .. } => zero_update(app, ui, *value),
            // A release where it began (a click, or dropped back on the corner) moves nothing.
            EditEvent::Commit { before: Mixed::Same(start), after, .. } if start == after => zero_cancel(app, ui),
            EditEvent::Commit { after, .. } => zero_commit(app, ui, *after),
            EditEvent::Cancel { .. } => zero_cancel(app, ui),
            EditEvent::Set { after, .. } => zero_set(app, *after),
            _ => {}
        }
    }
    if let Some(unit) = out.unit_picked.and_then(unit_of) {
        run(app, "document.setUnits", json!({ "units": unit.label() }));
    }
}

fn zero_begin(app: &mut VectorcraftApp, ui: &Ui) {
    set_drag(ui, ZERO, Some(Drag::begin(app)));
}

/// The pointer is at `at`: the engine shows the zero point snapped there as a drawn point is. Nothing is edited before
/// the release.
fn zero_update(app: &mut VectorcraftApp, ui: &Ui, at: Point) {
    let mut state = drag_state(ui, ZERO).unwrap_or_else(|| Drag::begin(app));
    if state.stale(app) {
        state.cancelled = true;
        set_drag(ui, ZERO, Some(state));
        app.session.end_ruler_origin_drag();
        return;
    }
    set_drag(ui, ZERO, Some(state));
    let view = app.view_info();
    app.session.ruler_origin_drag(Some(at), view);
}

/// The release: the one edit of the drag, at the snapped point.
fn zero_commit(app: &mut VectorcraftApp, ui: &Ui, at: Point) {
    let stale = drag_state(ui, ZERO).is_some_and(|d| d.stale(app));
    set_drag(ui, ZERO, None);
    if stale {
        app.session.end_ruler_origin_drag();
        return;
    }
    let view = app.view_info();
    let at = app.session.ruler_origin_drag(Some(at), view).unwrap_or(at);
    app.session.end_ruler_origin_drag();
    run(app, "document.setRulerOrigin", json!({"x": at.x, "y": at.y, "artboard": active_artboard(app)}));
}

fn zero_cancel(app: &mut VectorcraftApp, ui: &Ui) {
    set_drag(ui, ZERO, None);
    app.session.end_ruler_origin_drag();
}

/// A zero point set in one step: the document's origin is the reset, any other point a nudge.
fn zero_set(app: &mut VectorcraftApp, at: Point) {
    let artboard = active_artboard(app);
    if at == Point::ZERO {
        run(app, "document.resetRulerOrigin", json!({ "artboard": artboard }));
    } else {
        run(app, "document.setRulerOrigin", json!({"x": at.x, "y": at.y, "artboard": artboard}));
    }
}

/// Sends the engine one event of the guide being dragged out of a strip, a guide along `vertical` at `at` (document
/// units across it; none off the canvas).
fn engine_guide(app: &mut VectorcraftApp, vertical: bool, kind: PointerKind, at: Option<f64>, mods: Mods) {
    let pos = at.map_or(Point::ZERO, |a| if vertical { Point::new(a, 0.0) } else { Point::new(0.0, a) });
    let ev = PointerEvent { kind, pos, mods, backquote: false, pressure: 1.0, pen: None };
    if at.is_some() {
        // The new guide shows even if guides were hidden.
        app.ui.view.guides = true;
    }
    let view = app.view_info();
    if let Err(e) = app.session.ruler_guide(vertical, &ev, at.is_some(), view) {
        app.status(e.to_string());
    }
}

/// One pass of the guide a strip is placing. Trim View hides guides (#8363), so a drag it is on for at any point makes
/// none, and stays cancelled until it ends. A `Commit` with no drag running is a strip's double press: one guide
/// where it was pressed, one undo step.
fn guide(app: &mut VectorcraftApp, ui: &Ui, drag: &GuideDrag) {
    let vertical = matches!(drag.along, Axis::Y);
    let mut state = match drag.phase {
        GesturePhase::Begin => Drag::begin(app),
        _ => drag_state(ui, GUIDE).unwrap_or_else(|| Drag::begin(app)),
    };
    if app.ui.view.trim_view && !state.cancelled {
        state.cancelled = true;
        app.status(crate::menus::TRIM_VIEW_HIDES.to_string());
    }
    let running = app.session.ruler_guide_dragging();
    // The framework already turned the axis with Alt, so the engine is not given Alt to turn it again.
    let mods = Mods { shift: drag.mods.shift, alt: false, cmd: drag.mods.cmd, ctrl: drag.mods.ctrl, space: false };
    if state.cancelled {
        // A guide already dragged ends as a release off the canvas does: no guide, no undo step.
        if running {
            engine_guide(app, vertical, PointerKind::Up, None, Mods::default());
        }
    } else {
        match drag.phase {
            GesturePhase::Commit if !running => {
                if let Some(pos) = drag.at {
                    let mut params = json!({"vertical": vertical, "pos": pos});
                    if let Some(ab) = app.session.ruler_guide_artboard() {
                        params["artboard"] = json!(ab);
                    }
                    app.ui.view.guides = true;
                    run(app, "guide.add", params);
                }
            }
            GesturePhase::Commit => engine_guide(app, vertical, PointerKind::Up, drag.at, mods),
            GesturePhase::Cancel => engine_guide(app, vertical, PointerKind::Up, None, mods),
            _ => engine_guide(app, vertical, PointerKind::Drag, drag.at, mods),
        }
    }
    let ends = matches!(drag.phase, GesturePhase::Commit | GesturePhase::Cancel);
    set_drag(ui, GUIDE, (!ends).then_some(state));
}
