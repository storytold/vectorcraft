//! Save Graphic Style Library: a name and where it goes: the user library folder (listed under
//! Window → Graphic Style Libraries → User Defined) or a `.vcstyles` file picked in a save dialog
//! (a download on the web). OK runs `graphicStyle.saveLibrary`. Fields: see [`super::save_library`].

use serde_json::Value;
use vectorcraft_doc::style_libs::STYLES_EXT;

use super::save_library::{self, Library};
use crate::VectorcraftApp;

/// The dialog kind of Save Graphic Style Library.
pub const KIND: &str = "saveGraphicStyleLibrary";

/// Open Save Graphic Style Library for the document's styles (`names`: the ones selected in the
/// panel).
pub fn open(app: &mut VectorcraftApp, names: Vec<String>) -> Result<Value, String> {
    let library = Library {
        key: KIND,
        title: tl!("Save Graphic Style Library").to_string(),
        cmd: "graphicStyle.saveLibrary",
        only: tl!("Selected Styles Only").to_string(),
        ext: |_| STYLES_EXT.to_string(),
        has_user_dir: app.session.style_libraries.user_dir().is_some(),
    };
    save_library::open(app, library, names, vec![])
}
