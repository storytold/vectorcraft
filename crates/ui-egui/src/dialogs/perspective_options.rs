//! Perspective Grid Options (double-click the Perspective Grid tool): whether the Plane Switching
//! Widget shows and which corner of the window it sits in, and what Automatic Plane Positioning
//! takes a point from. OK runs `perspective.widget.options` (the options live in the preferences).
//!
//! Fields: `show` (bool), `position` (`topLeft`, `topRight`, `bottomLeft`, `bottomRight`),
//! `anchorPoint` (bool) and `gridlineIntersection` (bool).

use serde_json::{Value, json};
use vectorcraft_tools::distort::perspective::widget::WidgetCorner;

use super::{DialogSpec, form};
use crate::state::Dialog;
use crate::{VectorcraftApp, widgets};

/// The dialog kind of Perspective Grid Options.
pub const KIND: &str = "perspectiveGridOptions";

pub(super) const SPEC: DialogSpec =
    DialogSpec { heading: |_| tl!("Perspective Grid Options").into(), body, confirm, min_width: 340.0, ..DialogSpec::FORM };

/// Open the dialog with the current options.
pub fn open(app: &mut VectorcraftApp) {
    app.ui.dialog = Some(Dialog::new(KIND, json!(app.session.prefs.perspective_widget)));
}

fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let r = app.run("perspective.widget.options", form::params(d));
    if r.is_ok() {
        app.ui.dialog = None;
    }
    r
}

fn body(_app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let on = d.bool("show");
    if widgets::check(ui, tl!("Show Active Plane Widget"), on, true) {
        d.fields.insert("show".into(), json!(!on));
    }
    ui.add_space(8.0);
    let corners: Vec<(&str, &str)> = WidgetCorner::ALL.iter().map(|c| (c.id(), c.label())).collect();
    ui.add_enabled_ui(on, |ui| form::choice(ui, d, "position", tl!("Widget Position:"), (110.0, 160.0), &corners));
    ui.add_space(8.0);
    widgets::section_header(ui, tl!("Automatic Plane Positioning"));
    for (key, label) in [("anchorPoint", tl!("Anchor Point of Perspective Artwork")), ("gridlineIntersection", tl!("Intersection of Gridlines"))] {
        let on = d.bool(key);
        if widgets::check(ui, label, on, true) {
            d.fields.insert(key.into(), json!(!on));
        }
    }
    false
}
