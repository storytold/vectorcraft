//! Object › Path › Smooth's slider, as the Smooth slider help page describes
//! (https://helpx.adobe.com/illustrator/desktop/draw-shapes-and-paths/modify-paths/refine-path-segments-with-the-smooth-slider.html):
//! the strip of Simplify's slider (`simplify_slider::Strip`) with Auto-Smooth and a slider from
//! minimum smoothing (left) to maximum smoothing (right). While it is open the canvas previews
//! `object.path.smooth {level}`; Enter or a click outside the strip keeps the result as one undo
//! step, and Escape puts the paths back. It opens at Auto-Smooth's level, as Simplify's slider
//! opens at Auto-Simplify's (VectorCraft's choice: the page doesn't say where it starts).

use serde_json::Value;

use super::simplify_slider::{self, StripKind};
use crate::VectorcraftApp;

/// Opens the Smooth slider at Auto-Smooth's level.
pub(crate) fn open(app: &mut VectorcraftApp) {
    simplify_slider::open(app, StripKind::Smooth);
}

/// Whether the Smooth slider is open.
pub(crate) fn showing(app: &VectorcraftApp) -> bool {
    simplify_slider::showing_of(app, StripKind::Smooth)
}

/// Auto-Smooth (`ui.smoothAuto`): the open slider moves to the level `object.path.smooth {auto:
/// true}` picks, and the canvas previews it → {level}.
pub(crate) fn auto(app: &mut VectorcraftApp) -> Result<Value, String> {
    simplify_slider::auto_of(app, StripKind::Smooth)
}
