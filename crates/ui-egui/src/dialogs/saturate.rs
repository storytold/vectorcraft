//! Edit → Edit Colors → Saturate: an intensity slider (−100..100 %) previewed live on the canvas;
//! OK keeps the preview as one undo step (`edit.colors.saturate`). Hosted like Adjust Color Balance
//! (`color_balance::Adjust`).
//!
//! Fields: `intensity`. The preview check box is the host's.

use craft::theme::Srgba;
use craft::ui::patterns::form_dialogs::{DialogFields, Param};
use serde_json::{Value, json};
use vectorcraft_color::Color;

use super::color_balance::{Adjust, percent_row};
use super::color_picker::ColorDialogs;
use crate::VectorcraftApp;
use crate::panels::c32;

pub(crate) use super::color_balance::Open;

/// The dialog kind of Saturate.
pub const KIND: &str = "saturate";

const CMD: &str = "edit.colors.saturate";

const ADJUST: Adjust = Adjust { kind: KIND, cmd: CMD, label: "Saturate", slot, initial, rows, params };

fn slot(dialogs: &mut ColorDialogs) -> &mut Option<Open> {
    &mut dialogs.saturate
}

/// Opens the dialog at intensity 0.
pub fn open(app: &mut VectorcraftApp) {
    ADJUST.open(app);
}

/// Previews the open dialog; one line of `dialogs::host::pump`.
pub(crate) fn pump(app: &mut VectorcraftApp, _ctx: &egui::Context) {
    ADJUST.pump(app);
}

fn initial() -> DialogFields {
    DialogFields::from_pairs([("intensity", 0.0)])
}

fn params(fields: &DialogFields) -> Value {
    json!({"intensity": fields.number("intensity").unwrap_or(0.0)})
}

fn rows() -> Vec<Param> {
    vec![Param::custom("intensity", tl!("Intensity"), |ui, fields, _cx| {
        // The rail runs from grey to a fully saturated colour.
        percent_row(ui, fields, "intensity", tl!("Intensity"), |x| Srgba::from(c32(&Color::from_hsb(12.0, x, 0.85))));
        ui.response()
    })]
}

#[cfg(test)]
mod tests {
    use craft::ui::dialog::dialog_id;
    use vectorcraft_engine::Session;

    use super::*;
    use crate::dialogs::host;

    #[test]
    fn commits_without_a_drawn_frame_as_one_step() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        let ctx = crate::testing::ui_context();
        app.ui_ctx = Some(ctx.clone());
        app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
        let id = app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 50, "height": 50})).unwrap()["id"].as_u64().unwrap();
        app.run("paint.setFill", json!({"color": "#cc6666"})).unwrap();
        let undo = app.session.doc().unwrap().history.undo.len();
        open(&mut app);
        let dialogs = app.dialogs().unwrap();
        dialogs.with(|host| host.set_json_field(&ctx, dialog_id(KIND), "intensity", json!(-100))).unwrap();
        // OK in the frame the value was set, with no drawn pass of the dialog: still one undo step.
        dialogs.with(|host| host.confirm_by_id(&ctx, dialog_id(KIND)));
        host::pump(&mut app, &ctx);
        let st = app.session.doc().unwrap();
        assert_eq!(st.history.undo.len(), undo + 1);
        let c = st.doc.node(vectorcraft_doc::NodeId(id)).unwrap().appearance.fill_paint().color().unwrap();
        assert!(c.to_hsb()[1] < 1e-3, "fully desaturated");
    }
}
