//! Divide into Dashes in the interface: its dialog (dash and gap lengths, Fit to Corners and Ends,
//! Remove Gaps), which previews on the canvas and runs `addon.pathtools.divideDashes` on OK.

use egui::Ui;
use serde_json::{Value, json};

use crate::VectorcraftApp;
use crate::dialogs::{DialogSpec, form};
use crate::state::Dialog;
use crate::widgets;

/// The command, and its dialog's kind.
pub const COMMAND: &str = "addon.pathtools.divideDashes";
pub const KIND: &str = "divideDashes";
/// The dash and gap lengths the command takes (pt).
const DASHES: std::ops::RangeInclusive<f64> = 0.01..=1e6;
const GAPS: std::ops::RangeInclusive<f64> = 0.0..=1e6;
const LABEL_W: f32 = 60.0;
const FIELD_W: f32 = 90.0;

/// The dialog's fields to start with: the command's defaults.
pub(crate) fn fields() -> Value {
    json!({ "dash": 10, "gap": 0, "fit": false, "removeGaps": true })
}

pub(crate) const SPEC: DialogSpec =
    DialogSpec { heading: |_| tl!("Divide into Dashes").to_string(), body, confirm, preview: true, min_width: 280.0, ..DialogSpec::FORM };

fn body(app: &mut VectorcraftApp, ui: &mut Ui, d: &mut Dialog) -> bool {
    let unit = app.session.general_unit();
    widgets::label_row(ui, tl!("Dash:"), LABEL_W, |ui| {
        form::length_within(ui, d, "dash", unit, FIELD_W, Some(DASHES));
    });
    widgets::label_row(ui, tl!("Gap:"), LABEL_W, |ui| {
        form::length_within(ui, d, "gap", unit, FIELD_W, Some(GAPS));
    });
    form::check(ui, d, "fit", tl!("Fit to Corners and Ends"));
    // Without a gap there are no gap pieces to remove.
    let gaps = d.f64("gap", 0.0) > 0.0;
    ui.add_enabled_ui(gaps, |ui| form::check(ui, d, "removeGaps", tl!("Remove Gaps")));
    form::preview(app, ui, d, "Divide into Dashes", COMMAND, form::params(d));
    false
}

/// OK: run the command (keep its preview) and close.
fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    form::commit_preview(app, COMMAND, form::params(d))
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use vectorcraft_engine::Session;

    use crate::tests_labels::painted_text;
    use crate::{VectorcraftApp, menus};

    #[test]
    fn divide_into_dashes_opens_its_dialog_previews_and_runs_on_ok() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        let anchors = json!([{"x": 0, "y": 0}, {"x": 100, "y": 0}]);
        let id = app.run("path.create", json!({"anchors": anchors})).unwrap()["id"].as_u64().unwrap();
        app.run("select.set", json!({"ids": [id]})).unwrap();
        let entries = menus::menu_entries(&app);
        let item = entries.iter().find(|e| e.path == ["Object", "Plug-ins", "Path Precision Tools"] && e.label == "Divide into Dashes…").unwrap();
        let (cmd, p) = menus::click_target(&item.label, item.command.as_deref().unwrap(), &item.params);
        menus::invoke(&mut app, &cmd, p);
        assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some(super::KIND));
        let shown = painted_text(&mut app, |app, ui| crate::dialogs::show(app, ui.ctx()));
        for s in ["Divide into Dashes", "Dash:", "Gap:", "Fit to Corners and Ends", "Remove Gaps", "Preview"] {
            assert!(shown.contains(s), "{s} in {shown}");
        }
        let count = |app: &VectorcraftApp| app.session.doc().unwrap().doc.layers[0].children().unwrap().len();
        let d = app.ui.dialog.as_mut().unwrap();
        d.fields.insert("dash".into(), json!(25));
        d.fields.insert("preview".into(), json!(true));
        painted_text(&mut app, |app, ui| crate::dialogs::show(app, ui.ctx()));
        assert_eq!(count(&app), 4, "previewed");
        crate::dialogs::cancel(&mut app);
        assert_eq!(count(&app), 1, "rolled back");
        menus::invoke(&mut app, &cmd, serde_json::Value::Null);
        let n = app.session.doc().unwrap().history.undo.len();
        app.ui.dialog.as_mut().unwrap().fields.insert("gap".into(), json!(10));
        crate::dialogs::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
        assert_eq!(app.session.doc().unwrap().history.undo.len(), n + 1);
        // 10 + 10 + … over 100: five dashes, the gaps left empty.
        assert_eq!(count(&app), 5);
    }
}
