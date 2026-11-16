//! Pixel Snapping Options (the Control bar's Snapping menu): whether View › Snap to Pixel lands
//! drawing, moving and scaling on whole pixels. OK runs `view.snapToPixel.options` (the options live
//! in the preferences); Cancel leaves them as they were.
//!
//! Fields: `drawing`, `moving` and `scaling` (bool). While Snap to Pixel is off, the options stay
//! editable and a note says they take effect once it is on.

use serde_json::{Value, json};

use super::DialogSpec;
use crate::state::Dialog;
use crate::theme::Tokens;
use crate::{VectorcraftApp, widgets};

/// The dialog kind of Pixel Snapping Options.
pub const KIND: &str = "pixelSnapOptions";

pub(super) const SPEC: DialogSpec =
    DialogSpec { heading: |_| tl!("Pixel Snapping Options").into(), body, confirm, min_width: 320.0, ..DialogSpec::FORM };

/// Open the dialog with the current options.
pub fn open(app: &mut VectorcraftApp) {
    app.ui.dialog = Some(Dialog::new(KIND, json!(app.session.prefs.pixel_snap)));
}

fn check(ui: &mut egui::Ui, d: &mut Dialog, key: &str, text: &str) {
    let v = d.bool(key);
    if widgets::check(ui, text, v, true) {
        d.fields.insert(key.into(), json!(!v));
    }
}

fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    check(ui, d, "drawing", tl!("Snap to Pixel While Drawing"));
    check(ui, d, "moving", tl!("Snap to Pixel While Moving"));
    check(ui, d, "scaling", tl!("Snap to Pixel While Scaling"));
    if !app.ui.view.snap_to_pixel {
        let dim = Tokens::get(ui.ctx()).text_dim;
        ui.label(egui::RichText::new(tl!("These options take effect when View › Snap to Pixel is on.")).color(dim));
    }
    false
}

fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    super::run_and_close(app, "view.snapToPixel.options", Value::Object(d.fields.clone()))
}
