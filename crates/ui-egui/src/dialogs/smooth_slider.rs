//! Object › Path › Smooth's slider, as the Smooth slider help page describes
//! (https://helpx.adobe.com/illustrator/desktop/draw-shapes-and-paths/modify-paths/refine-path-segments-with-the-smooth-slider.html):
//! the strip of Simplify's slider (`simplify_slider::Strip`) with Auto-Smooth and a slider from
//! minimum smoothing (left) to maximum smoothing (right). While it is open the canvas previews
//! `object.path.smooth {level}`; Enter or a click outside the strip keeps the result as one undo
//! step, and Escape puts the paths back. It opens at Auto-Smooth's level, as Simplify's slider
//! opens at Auto-Simplify's (VectorCraft's choice: the page doesn't say where it starts).
//!
//! Fields: `level` (0..=100, the command's parameter).

use serde_json::{Value, json};
use vectorcraft_engine::SMOOTH_AUTO_LEVEL;

use super::simplify_slider::Strip;
use super::{DialogResult, DialogSpec};
use crate::VectorcraftApp;
use crate::state::Dialog;

/// The dialog kind of the Smooth slider.
pub(crate) const KIND: &str = "smoothSlider";

pub(super) const SPEC: DialogSpec = DialogSpec { window: Some(show), confirm, preview: true, ..DialogSpec::FORM };

/// What the Smooth slider previews, and its button.
const STRIP: Strip = Strip {
    kind: KIND,
    cmd: "object.path.smooth",
    label: "Smooth",
    default_level: SMOOTH_AUTO_LEVEL,
    auto: ("Auto-Smooth", auto),
    more_options: None,
};

/// The fields the slider opens with: Auto-Smooth's level.
pub(crate) fn fields() -> Value {
    json!({ "level": SMOOTH_AUTO_LEVEL })
}

/// Whether the Smooth slider is open.
pub(crate) fn showing(app: &VectorcraftApp) -> bool {
    STRIP.showing(app)
}

fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> DialogResult {
    STRIP.confirm(app, d)
}

fn show(app: &mut VectorcraftApp, ctx: &egui::Context) {
    STRIP.show(app, ctx);
}

/// Auto-Smooth (`ui.smoothAuto`): the open slider moves to the level `object.path.smooth {auto:
/// true}` picks, and the canvas previews it → {level}.
pub(crate) fn auto(app: &mut VectorcraftApp) -> Result<Value, String> {
    let d = STRIP.open(app).ok_or_else(|| "the Smooth slider is not open".to_string())?;
    STRIP.auto(app, d)
}
