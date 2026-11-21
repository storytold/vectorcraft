//! View → Edit Views…: the document's saved views in the shared saved-names dialog, to rename or
//! delete. The dialog applies every edit as one `view.saved.edit` undo step on OK.

use vectorcraft_engine::doc::SavedView;

// PLAN: assumed super::saved_names is a sibling module (od.OS16, object-dialogs-w21)
use super::saved_names::{self, SavedNames};
use crate::VectorcraftApp;

/// The dialog kind of Edit Views.
pub const KIND: &str = "editViews";

/// Open Edit Views on the active document's saved views.
pub fn open(app: &mut VectorcraftApp) -> Result<(), String> {
    let names: Vec<String> = app.session.active().map(|d| d.doc.views.iter().map(|v| v.name.clone()).collect()).unwrap_or_default();
    if names.is_empty() {
        return Err("No saved views (View → New View…)".into());
    }
    // PLAN: saved_names translates title/list (SavedNames fields are &'static str; production used tl!)
    saved_names::open(
        app,
        SavedNames { kind: KIND, title: "Edit Views", list: "Saved Views", command: "view.saved.edit", max: SavedView::MAX },
        names,
    )
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::VectorcraftApp;

    #[test]
    fn it_needs_a_saved_view() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({})).unwrap();
        assert!(super::open(&mut app).is_err());
        assert!(!app.any_dialog_open());
    }
}
