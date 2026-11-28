//! The Split Into Grid dialog names its row and column fields with their axis and lays them out in two columns, read
//! from the access tree of the window.

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

/// Split Into Grid lays its fields out as a grid: the columns' fields stand right of the rows' fields on the same
/// lines, and Add Guides stands below the last of them.
#[test]
fn split_into_grid_lays_out_its_fields_above_add_guides() {
    let mut app = app();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 120, "height": 80})).unwrap();
    app.run("select.all", json!({})).unwrap();
    assert!(super::object_dialogs::open_from_menu(&mut app, "object.path.splitIntoGrid"));
    settle(&mut app);
    let out = frame(&mut app, vec![]);
    let update = out.platform_output.accesskit_update.expect("an accessibility tree");
    let rect = |name: &str| {
        let node = update.nodes.iter().map(|(_, node)| node).find(|node| node.label() == Some(name));
        node.and_then(|node| node.bounds()).unwrap_or_else(|| panic!("{name} has a rect"))
    };
    let (rows, columns, guides) = (rect("Rows: Total:"), rect("Columns: Total:"), rect("Add Guides"));
    assert!(columns.x0 > rows.x1, "the columns' fields stand right of the rows' fields: {rows:?} {columns:?}");
    assert!(columns.y0 < rows.y1 && rows.y0 < columns.y1, "the two Total fields share a line: {rows:?} {columns:?}");
    assert!(guides.y0 >= rows.y1.max(columns.y1), "Add Guides stands below the grid: {guides:?} {rows:?} {columns:?}");
}
