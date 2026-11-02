//! Actions panel menu › Batch…: play the selected action on every file in a
//! folder, or on every data set of the open document (`action.batch`).

use serde_json::{Value, json};

use super::{DialogSpec, form};
use crate::state::Dialog;
use crate::theme::Tokens;
use crate::{VectorcraftApp, widgets};

pub const KIND: &str = "actionBatch";
const LABEL: f32 = 130.0;

pub(super) const SPEC: DialogSpec =
    DialogSpec { heading: |_| tl!("Batch").into(), body, confirm, ok: Some("OK"), min_width: 420.0, max_width: Some(460.0), ..DialogSpec::FORM };

/// Open the Batch dialog for the selected action (`set`, `action` indexes into `action_sets`).
pub fn open(app: &mut VectorcraftApp, set: usize, action: usize) -> Result<Value, String> {
    let a = app.ui.action_sets.get(set).and_then(|s| s.actions.get(action)).ok_or("no such action")?;
    if a.steps.is_empty() {
        return Err("the action has no steps".into());
    }
    let name = a.name.clone();
    app.ui.dialog = Some(Dialog::new(KIND, initial_fields(set, action, &name)));
    Ok(Value::Null)
}

fn initial_fields(set: usize, action: usize, name: &str) -> Value {
    json!({
        "set": set,
        "action": action,
        "actionName": name,
        "source": "folder",
        "datasetFileNaming": "fileNumber",
        "folder": "",
        "includeSubdirs": false,
        "destination": "folder",
        "destinationFolder": "",
        "overrideOpen": true,
        "overrideSave": true,
        "overrideExport": true,
        "stopOnError": true,
    })
}

fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let t = Tokens::get(ui.ctx());
    ui.label(egui::RichText::new(d.str("actionName")).color(t.text_strong));
    ui.add_space(8.0);

    form::choice(ui, d, "source", "Play:", (LABEL, 200.0), &[("folder", "Folder"), ("datasets", "Data Sets")]);
    let source = d.str("source");
    if source == "folder" {
        widgets::label_row(ui, tl!("Source Folder:"), LABEL, |ui| {
            form::text(ui, d, "folder", 200.0);
            if app.services.pick_folder.is_some() && ui.button(tl!("Choose…")).clicked() {
                crate::picks::folder_field(app, d, "folder");
            }
        });
        form::check(ui, d, "includeSubdirs", "Include All Subdirectories");
    } else {
        ui.label(egui::RichText::new(tl!("Uses every data set of the open document.")).size(11.5).color(t.text_dim));
        form::choice(
            ui,
            d,
            "datasetFileNaming",
            "File Name:",
            (LABEL, 200.0),
            &[("fileNumber", "File + Number"), ("fileDataSetName", "File + Data Set Name"), ("dataSetName", "Data Set Name")],
        );
    }
    ui.add_space(8.0);

    form::choice(ui, d, "destination", "Destination:", (LABEL, 200.0), &[("none", "None"), ("saveAndClose", "Save and Close"), ("folder", "Folder")]);
    if d.str("destination") == "folder" {
        widgets::label_row(ui, tl!("Destination Folder:"), LABEL, |ui| {
            form::text(ui, d, "destinationFolder", 200.0);
            if app.services.pick_folder.is_some() && ui.button(tl!("Choose…")).clicked() {
                crate::picks::folder_field(app, d, "destinationFolder");
            }
        });
    }
    ui.add_space(6.0);
    widgets::subheader(ui, tl!("Overrides"));
    form::check(ui, d, "overrideOpen", "Override Action \"Open\" Commands");
    form::check(ui, d, "overrideSave", "Override Action \"Save\" Commands");
    form::check(ui, d, "overrideExport", "Override Action \"Export\" Commands");
    form::check(ui, d, "stopOnError", "Stop on Errors");
    false
}

fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let set = d.fields.get("set").and_then(Value::as_u64).unwrap_or(0) as usize;
    let action = d.fields.get("action").and_then(Value::as_u64).unwrap_or(0) as usize;
    let a = app.ui.action_sets.get(set).and_then(|s| s.actions.get(action)).ok_or("no such action")?.clone();
    let commands: Vec<Value> = a.steps.iter().map(|(c, p)| json!({"command": c, "params": p})).collect();
    let source = d.str("source");
    if source == "folder" && d.str("folder").trim().is_empty() {
        return Err("choose a source folder".into());
    }
    let destination = d.str("destination");
    if destination == "folder" && d.str("destinationFolder").trim().is_empty() {
        return Err("choose a destination folder".into());
    }
    if source == "datasets" && app.session.active().is_none() {
        return Err("open a document with data sets".into());
    }
    let params = params(d, &a.name, commands);
    let r = app.run("action.batch", params)?;
    app.ui.dialog = None;
    let count = r["count"].as_u64().unwrap_or(0);
    let failed = r["failed"].as_u64().unwrap_or(0);
    if failed > 0 {
        app.status(format!("Batch: {count} processed, {failed} failed"));
    } else {
        app.status(format!("Batch: {count} processed"));
    }
    Ok(r)
}

fn params(d: &Dialog, action_name: &str, commands: Vec<Value>) -> Value {
    let source = d.str("source");
    let destination = d.str("destination");
    let mut params = json!({
        "commands": commands,
        "label": action_name,
        "source": source,
        "destination": destination,
        "overrideOpen": d.bool("overrideOpen"),
        "overrideSave": d.bool("overrideSave"),
        "overrideExport": d.bool("overrideExport"),
        "stopOnError": d.bool("stopOnError"),
        "includeSubdirs": d.bool("includeSubdirs"),
    });
    if source == "folder" {
        params["folder"] = json!(d.str("folder"));
    } else if source == "datasets" {
        params["datasetFileNaming"] = json!(d.str("datasetFileNaming"));
    }
    if destination == "folder" {
        params["destinationFolder"] = json!(d.str("destinationFolder"));
    }
    params
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_engine::Session;

    fn shown(source: &str) -> Vec<String> {
        fn collect(shape: &egui::Shape, out: &mut Vec<String>) {
            match shape {
                egui::Shape::Text(t) => out.push(t.galley.text().to_string()),
                egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| collect(shape, out)),
                _ => {}
            }
        }
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        let mut d = Dialog::new(KIND, initial_fields(0, 0, "Test"));
        d.fields["source"] = json!(source);
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let mut frame = ctx.run_ui(Default::default(), |ui| {
            body(&mut app, ui, &mut d);
        });
        frame.textures_delta.clear();
        let mut labels = vec![];
        frame.shapes.iter().for_each(|shape| collect(&shape.shape, &mut labels));
        labels
    }

    #[test]
    fn file_name_choice_shows_only_for_data_sets() {
        assert!(shown("datasets").iter().any(|label| label == "File Name:"));
        assert!(shown("datasets").iter().any(|label| label == "File + Number"));
        assert!(!shown("folder").iter().any(|label| label == "File Name:"));
    }

    #[test]
    fn data_set_naming_params_follow_dialog_choice() {
        let mut d = Dialog::new(KIND, initial_fields(0, 0, "Test"));
        assert_eq!(d.str("datasetFileNaming"), "fileNumber");
        d.fields["source"] = json!("datasets");
        for choice in ["fileNumber", "fileDataSetName", "dataSetName"] {
            d.fields["datasetFileNaming"] = json!(choice);
            assert_eq!(params(&d, "Test", vec![])["datasetFileNaming"], choice);
        }
        d.fields["source"] = json!("folder");
        assert!(params(&d, "Test", vec![]).get("datasetFileNaming").is_none());
    }
}
