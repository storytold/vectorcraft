//! Layers panel › Panel Options…: Show Layers Only, Locate Selected Objects, the row size and
//! which rows show thumbnails.
//!
//! Fields: `layersOnly`, `locateSelected`, `rowSize` (`small`, `medium`, `large` or `other`),
//! `otherSize` (points, 12–100, for `other`), `thumbLayers`, `thumbGroups` and `thumbObjects`. OK
//! applies them to the panel (kept with the UI state).

use serde_json::{Value, json};

use super::DialogSpec;
use super::swatch_options::{grid, label};
use crate::panels::layers::{PanelOptions, ROW_LARGE, ROW_MEDIUM, ROW_SMALL};
use crate::state::Dialog;
use crate::{VectorcraftApp, widgets};

/// The dialog kind of the Layers Panel Options.
pub const KIND: &str = "layersPanelOptions";

pub(super) const SPEC: DialogSpec =
    DialogSpec { heading: |_| tl!("Layers Panel Options").into(), body, confirm, min_width: 300.0, ..DialogSpec::FORM };

const SIZES: [(&str, &str, f32); 3] = [("small", "Small", ROW_SMALL), ("medium", "Medium", ROW_MEDIUM), ("large", "Large", ROW_LARGE)];

/// `ui.layersPanelOptions`: open the dialog on the panel's options.
pub fn open(app: &mut VectorcraftApp) -> Result<Value, String> {
    let o = &app.ui.layers_panel;
    let size = SIZES.iter().find(|s| (s.2 - o.row()).abs() < 0.01).map_or("other", |s| s.0);
    let fields = json!({
        "layersOnly": o.layers_only, "locateSelected": o.locate_selected, "rowSize": size, "otherSize": o.row(),
        "thumbLayers": o.thumb_layers, "thumbGroups": o.thumb_groups, "thumbObjects": o.thumb_objects,
    });
    app.ui.dialog = Some(Dialog::new(KIND, fields));
    Ok(Value::Null)
}

fn check(ui: &mut egui::Ui, d: &mut Dialog, key: &str, text: &str) {
    let v = d.bool(key);
    if widgets::check(ui, text, v, true) {
        d.fields.insert(key.into(), json!(!v));
    }
}

fn body(_: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    check(ui, d, "layersOnly", tl!("Show Layers Only"));
    check(ui, d, "locateSelected", tl!("Locate Selected Objects"));
    ui.add_space(8.0);
    grid(ui, |ui| {
        label(ui, tl!("Row Size:"));
        ui.horizontal(|ui| {
            let cur = d.str("rowSize");
            let labels: Vec<&str> = SIZES.iter().map(|s| s.1).chain(["Other"]).collect();
            let shown = SIZES.iter().find(|s| s.0 == cur).map_or("Other", |s| s.1);
            if let Some(i) = widgets::dropdown(ui, "layers-row-size", shown, &labels, 110.0) {
                d.fields.insert("rowSize".into(), json!(SIZES.get(i).map_or("other", |s| s.0)));
            }
            let v = d.f64("otherSize", f64::from(ROW_MEDIUM));
            let other = d.str("rowSize") == "other";
            if let Some(v) = ui.add_enabled_ui(other, |ui| widgets::range_field(ui, "otherSize", v, 12.0..=100.0, " pt", 0, 60.0)).inner {
                d.fields.insert("otherSize".into(), json!(v));
            }
        });
        ui.end_row();
    });
    ui.add_space(8.0);
    label(ui, tl!("Thumbnails:"));
    check(ui, d, "thumbLayers", tl!("Layers"));
    check(ui, d, "thumbGroups", tl!("Group"));
    check(ui, d, "thumbObjects", tl!("Object"));
    false
}

/// The options the fields describe.
pub(crate) fn options(d: &Dialog) -> PanelOptions {
    let size = d.str("rowSize");
    let row_size = SIZES.iter().find(|s| s.0 == size).map_or_else(|| (d.f64("otherSize", f64::from(ROW_MEDIUM)) as f32).clamp(12.0, 100.0), |s| s.2);
    PanelOptions {
        layers_only: d.bool("layersOnly"),
        locate_selected: d.bool("locateSelected"),
        row_size: if row_size.is_finite() { row_size } else { ROW_MEDIUM },
        thumb_layers: d.bool("thumbLayers"),
        thumb_groups: d.bool("thumbGroups"),
        thumb_objects: d.bool("thumbObjects"),
    }
}

fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    app.ui.layers_panel = options(d);
    app.ui.dialog = None;
    Ok(serde_json::to_value(&app.ui.layers_panel).unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_engine::Session;

    #[test]
    fn panel_options_round_trip_and_apply() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
        app.run("ui.layersPanelOptions", json!({})).unwrap();
        assert_eq!(app.ui.dialog.as_ref().unwrap().str("rowSize"), "medium");
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let mut out = ctx.run_ui(Default::default(), |ui| super::super::show(&mut app, ui.ctx()));
        out.textures_delta.clear();
        let d = app.ui.dialog.as_mut().unwrap();
        d.fields.insert("layersOnly".into(), json!(true));
        d.fields.insert("rowSize".into(), json!("other"));
        d.fields.insert("otherSize".into(), json!(500));
        d.fields.insert("thumbObjects".into(), json!(false));
        super::super::confirm(&mut app).unwrap();
        let o = &app.ui.layers_panel;
        assert!(o.layers_only && !o.thumb_objects && o.thumb_layers);
        assert_eq!(o.row(), 100.0, "kept to 100 pt");
        app.run("ui.layersPanelOptions", json!({})).unwrap();
        app.ui.dialog.as_mut().unwrap().fields.insert("rowSize".into(), json!("large"));
        super::super::confirm(&mut app).unwrap();
        assert_eq!(app.ui.layers_panel.row(), ROW_LARGE);
    }

    /// Locate Selected Objects is off by default and in options saved without it; OK turns it on
    /// and keeps it with the UI state, and the dialog opens on it again.
    #[test]
    fn locate_selected_objects_is_off_by_default_and_kept_with_the_ui_state() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
        app.run("ui.layersPanelOptions", json!({})).unwrap();
        let d = app.ui.dialog.as_mut().unwrap();
        assert_eq!(d.fields.get("locateSelected"), Some(&json!(false)));
        d.fields.insert("locateSelected".into(), json!(true));
        super::super::confirm(&mut app).unwrap();
        let back: crate::UiState = serde_json::from_value(serde_json::to_value(&app.ui).unwrap()).unwrap();
        assert!(back.layers_panel.locate_selected, "kept with the UI state");
        app.run("ui.layersPanelOptions", json!({})).unwrap();
        assert!(app.ui.dialog.as_ref().unwrap().bool("locateSelected"));
        let old: crate::UiState = serde_json::from_value(json!({"layers_panel": {"layersOnly": true}})).unwrap();
        assert!(old.layers_panel.layers_only && !old.layers_panel.locate_selected, "options saved without it");
    }
}
