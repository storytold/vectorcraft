//! The Split Into Grid dialog names its row and column fields with their axis, read from the access tree of the window.

use egui::accesskit::Role;
use serde_json::json;

use super::tests::{app, frame, settle};

/// Split Into Grid keeps its visible labels while naming the row and column gutter and total inputs by axis and context
/// (`FieldLabelRole::Describes`): the labels describe, the names carry the axis.
#[test]
fn split_into_grid_names_row_and_column_fields_with_context() {
    let mut app = app();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 120, "height": 80})).unwrap();
    app.run("select.all", json!({})).unwrap();
    assert!(super::object_dialogs::open_from_menu(&mut app, "object.path.splitIntoGrid"));
    settle(&mut app);
    let out = frame(&mut app, vec![]);
    let update = out.platform_output.accesskit_update.expect("an accessibility tree");
    // The fields are number fields, which the framework exposes as spin buttons.
    let names: Vec<&str> = update
        .nodes
        .iter()
        .filter(|(_, node)| matches!(node.role(), Role::TextInput | Role::SpinButton))
        .filter_map(|(_, node)| node.label())
        .collect();
    for name in ["Rows: Gutter:", "Columns: Gutter:", "Rows: Total:", "Columns: Total:"] {
        assert!(names.contains(&name), "{name} among the text inputs: {names:?}");
    }
}
