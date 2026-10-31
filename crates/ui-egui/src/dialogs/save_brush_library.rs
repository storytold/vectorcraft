//! Save Brush Library: a name and where it goes: the user library folder (listed under Window →
//! Brush Libraries → User Defined) or a `.vectorcraft` file picked in a save dialog (a download on
//! the web). OK runs `brush.saveLibrary`.
//!
//! Fields: `name`, `user` (save to the user library folder), `selectedOnly` and `names` (the
//! brushes selected in the Brushes panel), `__user` (there is a user library folder).

use serde_json::Value;
use vectorcraft_engine::cmd::brushlib::BRUSHES_EXT;

use super::DialogSpec;
use super::save_swatch_library::{destination, fields, name_row, params, save};
use super::swatch_options::grid;
use crate::VectorcraftApp;
use crate::state::Dialog;

/// The dialog kind of Save Brush Library.
pub const KIND: &str = "saveBrushLibrary";

pub(super) const SPEC: DialogSpec = DialogSpec { heading: |_| tl!("Save Brush Library").into(), body, confirm, min_width: 360.0, ..DialogSpec::FORM };

/// Open Save Brush Library for the document's brushes (`names`: those selected in the panel).
pub fn open(app: &mut VectorcraftApp, names: Vec<String>) -> Result<Value, String> {
    let fields = fields(app, app.session.brush_libraries.user_dir().is_some(), names)?;
    app.ui.dialog = Some(Dialog::new(KIND, fields));
    Ok(Value::Null)
}

fn body(_: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    grid(ui, |ui| {
        name_row(ui, d);
        destination(ui, d, tl!("Selected Brushes Only"));
    });
    false
}

fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    save(app, d, "brush.saveLibrary", BRUSHES_EXT, params(d))
}
