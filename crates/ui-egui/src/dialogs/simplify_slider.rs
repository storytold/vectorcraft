//! Object › Path › Simplify's slider: a popover under the selection with Auto-Simplify, a slider from
//! the fewest anchor points (left) to the closest fit to the original (right), and More Options,
//! which opens the Simplify dialog at the slider's tolerance. While the strip is open, the canvas
//! previews `object.path.simplify {level}`; Enter or a click outside the strip keeps the result as
//! one undo step, and Escape puts the paths back. The slider opens at Auto-Simplify's level for
//! the selection (`vectorcraft_engine::auto_simplify_level`), so the canvas first shows the
//! auto-simplified paths, as the Simplify help page describes
//! (https://helpx.adobe.com/illustrator/desktop/draw-shapes-and-paths/modify-paths/manually-simplify-paths.html).
//! `vectorcraft_pathops::simplify_level_tolerance` maps levels to tolerances.
//!
//! The strip ([`Strip`]) is shared with Object › Path › Smooth's slider (`smooth_slider`) through
//! [`StripKind`]. The canvas body calls [`show`] once per frame.

// PLAN: the framework paths below (`craft_ui_widgets`, `craft_ui_contract`, `craft_behavior`) and
// `LivePreview`'s module are assumed; no app file imports them yet.
use craft::behavior::overlay::{Align, Side};
use craft::behavior::{CanvasView, IconId};
use craft::ui::buttons::IconButton;
use craft::ui::contract::overlay::{Anchor, Dismiss, Outside, Place};
use craft::ui::contract::{Bind, Props as _, Show as _, ShowContainer as _};
use craft::ui::layout_components::Row;
use craft::ui::popover::Popover;
use craft::ui::primitives::IconSpec;
use craft::ui::runtime::focus::Initial;
use craft::ui::runtime::overlay::CloseReason;
use craft::ui::value_drag::Slider;
use egui::{Pos2, Rect as ScreenRect};
use serde_json::{Value, json};
use vectorcraft_engine::{SIMPLIFY_LEVEL, SMOOTH_AUTO_LEVEL, auto_simplify_level, simplify_level_tolerance};
use vectorcraft_geom::Rect;

use super::preview::LivePreview;
use crate::VectorcraftApp;

/// Which command a slider strip previews.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StripKind {
    /// Object › Path › Simplify.
    Simplify,
    /// Object › Path › Smooth.
    Smooth,
}

impl StripKind {
    /// The command previewed.
    fn cmd(self) -> &'static str {
        match self {
            StripKind::Simplify => "object.path.simplify",
            StripKind::Smooth => "object.path.smooth",
        }
    }

    /// The undo step's name, and the slider's.
    fn label(self) -> &'static str {
        match self {
            StripKind::Simplify => "Simplify",
            StripKind::Smooth => "Smooth",
        }
    }

    /// The auto button's tip.
    fn auto_tip(self) -> &'static str {
        match self {
            StripKind::Simplify => "Auto-Simplify",
            StripKind::Smooth => "Auto-Smooth",
        }
    }

    /// Whether the strip has a More Options button.
    fn has_more_options(self) -> bool {
        self == StripKind::Simplify
    }

    /// The level when no number is available.
    fn default_level(self) -> f64 {
        match self {
            StripKind::Simplify => SIMPLIFY_LEVEL,
            StripKind::Smooth => SMOOTH_AUTO_LEVEL,
        }
    }
}

/// An open slider strip: the level the canvas previews, the selection's bounds from before any
/// preview (where the strip hangs), and the preview itself.
pub(crate) struct Strip {
    /// Which command the strip previews.
    pub kind: StripKind,
    /// The drawing whose selection and preview the strip uses.
    document: Option<u64>,
    /// The slider's value, 0 to 100.
    pub level: f64,
    /// The selection's document bounds when the strip opened, so the strip stays put while the
    /// preview changes the paths.
    pub bounds: Option<Rect>,
    /// Whether the popover is open.
    pub open: bool,
    /// The level the auto button picked: while the slider stays there, the canvas previews `{auto: true}`.
    pub auto: Option<f64>,
    /// The canvas preview of `kind`'s command.
    pub preview: LivePreview,
}

impl Strip {
    fn in_source(&self, app: &VectorcraftApp) -> bool {
        self.document == app.session.active().map(|doc| doc.uid)
    }

    fn cancel(self, app: &mut VectorcraftApp) {
        if self.in_source(app)
            && app.session.active().and_then(|doc| doc.interaction.as_ref()?.preview.as_ref()).is_some_and(|(cmd, _)| cmd == self.kind.cmd())
        {
            self.preview.cancel(app);
        }
    }

    /// The slider's level, a whole number from 0 to 100.
    fn level(&self) -> f64 {
        if self.level.is_finite() { self.level.round().clamp(0.0, 100.0) } else { self.kind.default_level() }
    }

    fn params(&self) -> Value {
        match self.auto {
            Some(level) if level == self.level() => json!({ "auto": true }),
            _ => json!({ "level": self.level() }),
        }
    }
}

/// The level a strip of `kind` opens at: Auto-Simplify's for the selection (or [`SIMPLIFY_LEVEL`]
/// without a document), or Auto-Smooth's.
pub(crate) fn opening_level(app: &VectorcraftApp, kind: StripKind) -> f64 {
    match kind {
        StripKind::Simplify => auto_simplify_level(&app.session).unwrap_or(SIMPLIFY_LEVEL),
        StripKind::Smooth => SMOOTH_AUTO_LEVEL,
    }
}

/// Opens the strip of `kind`, replacing (and cancelling the preview of) one that is open.
pub(crate) fn open(app: &mut VectorcraftApp, kind: StripKind) {
    if let Some(old) = app.object_dialogs.strip.take() {
        old.cancel(app);
    }
    let strip = Strip {
        kind,
        document: app.session.active().map(|doc| doc.uid),
        level: opening_level(app, kind),
        bounds: app.selection_bounds(),
        open: true,
        auto: None,
        preview: LivePreview::default(),
    };
    app.object_dialogs.strip = Some(strip);
}

/// Whether the Simplify slider is open.
pub(crate) fn showing(app: &VectorcraftApp) -> bool {
    showing_of(app, StripKind::Simplify)
}

/// Whether the strip of `kind` is open.
pub(super) fn showing_of(app: &VectorcraftApp, kind: StripKind) -> bool {
    app.object_dialogs.strip.as_ref().is_some_and(|s| s.kind == kind)
}

/// The error of a UI command whose strip is not open.
fn not_open(kind: StripKind) -> String {
    format!("the {} slider is not open", kind.label())
}

/// Auto-Simplify (`ui.simplifyAuto`): the open slider moves to the level `object.path.simplify
/// {auto: true}` picks for the selection, and the canvas previews it → {level}.
pub(crate) fn auto(app: &mut VectorcraftApp) -> Result<Value, String> {
    auto_of(app, StripKind::Simplify)
}

/// The auto button of the strip of `kind`.
pub(super) fn auto_of(app: &mut VectorcraftApp, kind: StripKind) -> Result<Value, String> {
    if !showing_of(app, kind) {
        return Err(not_open(kind));
    }
    let Some(mut strip) = app.object_dialogs.strip.take() else { return Err(not_open(kind)) };
    if !strip.in_source(app) {
        return Err(not_open(kind));
    }
    let answer = strip.preview.run(app, true, kind.label(), kind.cmd(), &json!({ "auto": true }));
    let result = answer.unwrap_or_else(|| Ok(json!({ "level": strip.level() })));
    if let Ok(v) = &result {
        strip.level = v["level"].as_f64().unwrap_or(kind.default_level());
        strip.auto = Some(strip.level());
    }
    let level = strip.level();
    app.object_dialogs.strip = Some(strip);
    result.map(|_| json!({ "level": level }))
}

/// More Options (`ui.simplifyMoreOptions`): the Simplify dialog takes the slider's place, at the
/// slider's tolerance → {tolerance}.
pub(crate) fn more_options(app: &mut VectorcraftApp) -> Result<Value, String> {
    if !showing_of(app, StripKind::Simplify) {
        return Err(not_open(StripKind::Simplify));
    }
    let Some(strip) = app.object_dialogs.strip.take() else { return Err(not_open(StripKind::Simplify)) };
    if !strip.in_source(app) {
        return Err(not_open(StripKind::Simplify));
    }
    let tolerance = simplify_level_tolerance(strip.level());
    strip.cancel(app);
    // PLAN: assumed `path_ops::open_simplify(app: &mut VectorcraftApp, tolerance: f64)` returning `()`.
    super::path_ops::open_simplify(app, tolerance);
    Ok(json!({ "tolerance": tolerance }))
}

/// Where the strip hangs: a point under the lowest corner on screen of the selection's bounds from
/// before the preview, centred on them, kept on the canvas.
fn anchor_rect(view: &CanvasView, bounds: Option<Rect>) -> ScreenRect {
    let canvas = view.rect;
    let at = match bounds {
        Some(b) => {
            let lowest = view.quad(b).iter().map(|c| c.y).fold(f32::NEG_INFINITY, f32::max);
            Pos2::new(view.to_screen(b.center()).x, lowest)
        }
        None => canvas.center(),
    };
    let at = Pos2::new(at.x.clamp(canvas.left(), canvas.right()), at.y.clamp(canvas.top(), canvas.bottom()));
    ScreenRect::from_min_max(at, at)
}

/// Shows the open strip as a popover on the canvas and runs its preview; called once per frame by
/// the canvas body.
pub(crate) fn show(app: &mut VectorcraftApp, ui: &mut egui::Ui, view: &CanvasView) {
    let Some(mut strip) = app.object_dialogs.strip.take() else { return };
    if !strip.in_source(app) {
        return;
    }
    let kind = strip.kind;
    let anchor = anchor_rect(view, strip.bounds);
    let (mut level, mut open) = (strip.level, strip.open);
    let (mut auto_clicked, mut more_clicked) = (false, false);
    let shown = Popover::new(tl!(kind.label()), Bind::bound(&mut open))
        .overlay(|o| o.place(Place::anchored(Anchor::Rect(anchor), Side::Below, Align::Center)))
        .dismiss(Dismiss::new().outside(Outside::Absorb))
        // The keyboard opens on the slider: arrows move it, Enter keeps the result.
        .initial_focus(Initial::Default)
        .close_on_enter(true)
        .escape(true)
        .show(ui, |ui, _| {
            Row::new().show(ui, |ui, _| {
                auto_clicked = IconButton::new(IconSpec::svg(IconId::new("wand-sparkles"))).label(tl!(kind.auto_tip())).show(ui).clicked();
                Slider::new(Bind::bound(&mut level), 0.0..=100.0).step(1.0).label(tl!(kind.label())).default_stop().show(ui);
                if kind.has_more_options() {
                    more_clicked = IconButton::new(IconSpec::svg(IconId::new("ellipsis"))).label(tl!("More Options")).show(ui).clicked();
                }
            });
        });
    strip.level = level;
    strip.open = open;
    let result = match shown.output.closed {
        Some(CloseReason::Escape) => {
            strip.cancel(app);
            return;
        }
        Some(_) => {
            let params = strip.params();
            let result = strip.preview.commit(app, kind.cmd(), params).map(|_| ());
            if let Err(e) = result {
                app.status(e);
            }
            return;
        }
        None if more_clicked => {
            app.object_dialogs.strip = Some(strip);
            more_options(app)
        }
        None if auto_clicked => {
            app.object_dialogs.strip = Some(strip);
            auto_of(app, kind)
        }
        None => {
            let params = strip.params();
            let answer = strip.preview.run(app, true, kind.label(), kind.cmd(), &params);
            app.object_dialogs.strip = Some(strip);
            answer.unwrap_or(Ok(Value::Null))
        }
    };
    if let Err(e) = result {
        app.status(e);
    }
}
