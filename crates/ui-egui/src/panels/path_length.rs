//! Path Length panel: the selection's total path length (every path in it summed — a compound
//! path's and a selected group's paths included), editable to uniformly scale the selection, about
//! its centre, to a new length (`object.setLength`). Unlike Illustrator's Document Info, which only
//! reports a path's length, this field can be typed into.

use egui::Ui;
use serde_json::json;

use crate::VectorcraftApp;
use crate::widgets;

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let Some(st) = app.session.active() else {
        widgets::dim_label(ui, tl!("No document"));
        return;
    };
    let ids = st.selection.objects.clone();
    let units = app.session.general_unit();
    let len = app.session.selection_length(&ids).filter(|l| *l > 1e-9);
    let has = !ids.is_empty() && len.is_some();
    ui.horizontal(|ui| {
        widgets::dim_label(ui, tl!("Length:"));
        ui.add_enabled_ui(has, |ui| {
            if let Some(v) = widgets::num_field(ui, "pathlen-length", len, units, 80.0)
                && v > 0.0
                && let Err(e) = app.run("object.setLength", json!({"length": v}))
            {
                app.status(e);
            }
        });
    });
    if !ids.is_empty() && len.is_none() {
        widgets::dim_label(ui, tl!("Selection has no length to scale"));
    }
}
