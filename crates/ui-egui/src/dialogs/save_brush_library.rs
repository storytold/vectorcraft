//! Save Brush Library: a name and where it goes: the user library folder (listed under Window →
//! Brush Libraries → User Defined) or a `.vectorcraft` file picked in a save dialog (a download on
//! the web). OK runs `brush.saveLibrary`. Fields: see [`super::save_library`].

use serde_json::Value;
use vectorcraft_engine::cmd::brushlib::BRUSHES_EXT;

use super::save_library::{self, Library};
use crate::VectorcraftApp;

/// The dialog kind of Save Brush Library.
pub const KIND: &str = "saveBrushLibrary";

/// Open Save Brush Library for the document's brushes (`names`: those selected in the panel).
pub fn open(app: &mut VectorcraftApp, names: Vec<String>) -> Result<Value, String> {
    let library = Library {
        key: KIND,
        title: tl!("Save Brush Library").to_string(),
        cmd: "brush.saveLibrary",
        only: tl!("Selected Brushes Only").to_string(),
        ext: |_| BRUSHES_EXT.to_string(),
        has_user_dir: app.session.brush_libraries.user_dir().is_some(),
    };
    save_library::open(app, library, names, vec![])
}
