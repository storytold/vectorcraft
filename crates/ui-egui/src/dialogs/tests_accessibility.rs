//! Strict accessibility checks on the real dialogs: kittest fails any input without a name, and
//! each test also asks for the names a screen reader must hear on the fields this migration
//! touched (form text fields, parameter fields, bleed, split-grid context).

use egui::accesskit::Role;
use egui::vec2;
use egui_kittest::kittest::Queryable;
use serde_json::json;
use vectorcraft_engine::Session;

use super::show;
use crate::VectorcraftApp;

type Harness = egui_kittest::Harness<'static, VectorcraftApp>;

/// Build a 300 by 300 document for a real-dialog accessibility harness.
fn app() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
    app
}

/// A kittest harness over the real dialog layer, with kittest's strict accessibility check on:
/// every visible input must have an accessible name.
fn strict(app: VectorcraftApp) -> Harness {
    let mut fonts = false;
    let mut h = egui_kittest::Harness::builder().with_size(vec2(1100.0, 900.0)).build_ui_state(
        move |ui, app: &mut VectorcraftApp| {
            if !std::mem::replace(&mut fonts, true) {
                crate::theme::install_fonts(ui.ctx());
                return;
            }
            show(app, ui.ctx());
        },
        app,
    );
    h.run_steps(5);
    h.check_accessibility();
    h
}

/// Export for Screens names its three text fields and exposes no raw widget ids.
#[test]
fn export_for_screens_names_the_form_text_fields() {
    let mut app = app();
    app.run("file.exportForScreens", json!({})).unwrap();
    let h = strict(app);
    for name in ["File Name:", "Prefix:", "Start at:"] {
        let _ = h.get_by_role_and_label(Role::TextInput, name);
    }
    assert!(h.query_by_role_and_label(Role::TextInput, "name").is_none(), "no internal widget id");
    assert!(h.query_by_role_and_label(Role::TextInput, "prefix").is_none(), "no internal widget id");
}

/// Flatten Transparency names its preset name and three resolution or balance fields and roles the
/// balance control as a slider.
#[test]
fn flatten_transparency_names_its_preset_and_option_fields() {
    let mut app = app();
    app.run("ui.flattenTransparencyDialog", json!({})).unwrap();
    {
        let d = app.ui.dialog.as_mut().expect("Flatten Transparency is open");
        d.fields.insert("__saving".into(), json!(true));
        d.fields.insert("__presetName".into(), json!("Mine"));
    }
    let h = strict(app);
    for name in ["Name:", "Raster/Vector Balance:", "Line Art and Text Resolution:", "Gradient and Mesh Resolution:"] {
        let _ = h.get_by_role_and_label(Role::TextInput, name);
    }
    let _ = h.get_by_role_and_label(Role::Slider, "Raster/Vector Balance:");
}

/// Document Setup names the top, bottom, left and right bleed inputs.
#[test]
fn document_setup_names_each_bleed_side() {
    let mut app = app();
    app.run("file.documentSetup", json!({})).unwrap();
    let h = strict(app);
    for side in ["Top", "Bottom", "Left", "Right"] {
        let _ = h.get_by_role_and_label(Role::TextInput, side);
    }
}

/// Split Into Grid keeps translated visible labels while naming row and column gutter and total
/// inputs by axis and context.
#[test]
fn split_into_grid_names_row_and_column_fields_with_context() {
    let mut app = app();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 120, "height": 80})).unwrap();
    app.run("select.all", json!({})).unwrap();
    crate::menus::invoke(&mut app, "object.path.splitIntoGrid", json!({}));
    assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some("splitIntoGrid"));
    let h = strict(app);
    for name in ["Rows: Gutter:", "Columns: Gutter:", "Rows: Total:", "Columns: Total:"] {
        let _ = h.get_by_role_and_label(Role::TextInput, name);
    }
    // The visible rows keep the existing translated labels; only their accessible names carry
    // the axis. Two labels of each remain, so the two inputs would otherwise share a name.
    assert_eq!(h.get_all_by_label("Gutter:").count(), 2, "the visible gutter labels are unchanged");
    assert_eq!(h.get_all_by_label("Total:").count(), 2, "the visible total labels are unchanged");
}

/// Artboard Options names the video ruler pixel aspect ratio input.
#[test]
fn artboard_options_names_the_video_pixel_aspect_ratio_field() {
    let mut app = app();
    app.select_tool("artboard");
    app.run("ui.newArtboard", json!({})).unwrap();
    assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some("artboardOptions"));
    let h = strict(app);
    let _ = h.get_by_role_and_label(Role::TextInput, "Video Ruler Pixel Aspect Ratio:");
}

/// A generic parameter dialog names its number and text inputs, combo boxes and checkbox.
#[test]
fn generic_parameter_dialog_names_numbers_text_and_choices() {
    let mut app = app();
    app.run(
        "ui.paramDialog",
        json!({
            "command": "type.pathOptions",
            "label": "Type on a Path Options",
            "params": {
                "effect": "rainbow",
                "alignToPath": "baseline",
                "spacing": 10.0,
                "flip": false,
                "note": "Draft"
            }
        }),
    )
    .unwrap();
    let h = strict(app);
    for name in ["Spacing:", "Note:"] {
        let _ = h.get_by_role_and_label(Role::TextInput, name);
    }
    let _ = h.get_by_role_and_label(Role::ComboBox, "Effect:");
    let _ = h.get_by_role_and_label(Role::ComboBox, "Align to Path:");
    let _ = h.get_by_role_and_label(Role::CheckBox, "Flip:");
}

/// Slice Options names its multiline text-display area.
#[test]
fn slice_options_names_its_text_area() {
    let mut app = app();
    app.run("object.slice.create", json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap();
    app.run("object.slice.options", json!({})).unwrap();
    app.ui.dialog.as_mut().expect("Slice Options is open").fields.insert("kind".into(), json!("noImage"));
    let h = strict(app);
    let _ = h.get_by_role_and_label(Role::MultilineTextInput, "Text Displayed in Cell:");
}
