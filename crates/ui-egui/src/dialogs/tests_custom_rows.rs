//! Dialogs whose custom rows lay out several controls side by side or one under another take the width of their rows,
//! read from the access tree of the window.

use egui::accesskit::Role;
use serde_json::{Value, json};
use vectorcraft_engine::cmd::fileio::FORMATS;

use super::tests::{SCREEN, app, frame, settle};
use crate::VectorcraftApp;

/// The width of the dialog named `title` once it has settled.
fn dialog_width(app: &mut VectorcraftApp, title: &str) -> f32 {
    settle(app);
    let out = frame(app, vec![]);
    let update = out.platform_output.accesskit_update.expect("an accessibility tree");
    let dialog = update.nodes.iter().find(|(_, node)| node.role() == Role::Dialog && node.label() == Some(title));
    let bounds = dialog.and_then(|(_, node)| node.bounds()).unwrap_or_else(|| panic!("the {title} dialog is open"));
    (bounds.x1 - bounds.x0) as f32
}

/// Writes `value` into `key` of the top dialog, as automation does.
fn set(app: &VectorcraftApp, key: &str, value: Value) {
    let ctx = app.ui_ctx.clone().expect("a window app");
    let host = app.dialogs().expect("a dialog host");
    host.with(|h| {
        let id = h.open_dialogs().last().map(|d| d.id).expect("an open dialog");
        h.set_json_field(&ctx, id, key, value)
    })
    .unwrap();
}

/// Opens the options of raster format `id` for an export.
fn raster_options(app: &mut VectorcraftApp, id: &str) {
    let f = FORMATS.iter().find(|f| f.id == id).expect("a raster format");
    super::png_options::open(app, f, json!({"format": id, "path": format!("/out/a.{id}")}));
}

/// A custom row drawn as an egui layout measured as wide as the room the dialog offered it, so the dialog took the
/// window's whole width. Each dialog here holds such a row and stays well inside the window.
#[test]
fn dialogs_with_custom_rows_take_the_width_of_their_rows() {
    let half = SCREEN.x / 2.0;

    let mut a = app();
    crate::menus::invoke(&mut a, "ui.dxfOptionsDialog", json!({}));
    let dxf = dialog_width(&mut a, "DXF Options");

    let mut a = app();
    crate::menus::invoke(&mut a, "ui.newSwatch", json!({}));
    let swatch = dialog_width(&mut a, "New Swatch");

    let mut a = app();
    raster_options(&mut a, "png");
    settle(&mut a);
    set(&a, "ppi", json!(100));
    set(&a, "__otherPpi", json!(true));
    set(&a, "background", json!("#336699"));
    let png = dialog_width(&mut a, "PNG Options");

    let mut a = app();
    raster_options(&mut a, "jpg");
    let jpeg = dialog_width(&mut a, "JPEG Options");

    let mut a = app();
    a.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 120, "height": 80})).unwrap();
    a.run("select.all", json!({})).unwrap();
    super::effect::open(&mut a, &json!({"effect": "blur.radial"})).unwrap();
    let blur = dialog_width(&mut a, "Radial Blur");

    for (name, width) in [("DXF Options", dxf), ("New Swatch", swatch), ("PNG Options", png), ("JPEG Options", jpeg), ("Radial Blur", blur)] {
        assert!(width < half, "{name} is {width} pt wide in a {} pt window", SCREEN.x);
    }
}
