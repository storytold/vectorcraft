//! Path dialogs: Average, and Offset Path, Simplify and Split Into Grid with Preview.

use serde_json::{Value, json};
use vectorcraft_doc::{NodeId, NodeKind, Unit};
use vectorcraft_geom::PathData;

use super::{DialogSpec, form, run_and_close};
use crate::VectorcraftApp;
use crate::state::Dialog;
use crate::theme::Tokens;

pub(super) const SPEC: DialogSpec = DialogSpec { heading: |d| title(&d.kind).into(), body, confirm, preview: true, ..DialogSpec::FORM };

/// Simplify's Corner Point Angle Threshold when the dialog opens, in degrees. An anchor whose
/// corner is sharper than the threshold stays a corner, so a higher threshold keeps more corners.
/// The command's `cornerAngle` is the turn above which an anchor stays a corner, so the dialog
/// sends 180° less the threshold: 150° sends `object.path.simplify`'s default of 30°.
pub(crate) const ANGLE_THRESHOLD: f64 = 150.0;

/// The field where Simplify keeps the ids of the paths its last preview ran on.
const ORIGINAL_IDS: &str = "__ids";

fn title(kind: &str) -> &'static str {
    match kind {
        "average" => tl!("Average"),
        "offsetPath" => tl!("Offset Path"),
        "simplify" => tl!("Simplify"),
        _ => tl!("Split Into Grid"),
    }
}

/// The command the dialog runs and its parameters.
fn command(d: &Dialog) -> (&'static str, Value) {
    match d.kind.as_str() {
        "average" => ("path.average", json!({"axis": d.str("axis")})),
        "offsetPath" => {
            ("object.path.offsetPath", json!({"offset": d.f64("offset", 10.0), "joins": d.str("joins"), "miterLimit": d.f64("miterLimit", 4.0)}))
        }
        "simplify" => {
            let turn = 180.0 - d.f64("angleThreshold", ANGLE_THRESHOLD).clamp(0.0, 180.0);
            ("object.path.simplify", json!({"tolerance": d.f64("tolerance", 1.0), "cornerAngle": turn, "straightLines": d.bool("straightLines")}))
        }
        _ => ("object.path.splitIntoGrid", form::params(d)),
    }
}

/// The undo label of the dialogs that preview on the canvas (all but Average).
fn previewed(kind: &str) -> Option<&'static str> {
    match kind {
        "offsetPath" => Some("Offset Path"),
        "simplify" => Some("Simplify"),
        "splitIntoGrid" => Some("Split Into Grid"),
        _ => None,
    }
}

/// The fields Split Into Grid opens with for a selection `width` × `height` points.
pub(crate) fn split_fields(width: f64, height: f64) -> Value {
    json!({"rows": 2, "columns": 2, "rowGutter": 12.0, "columnGutter": 12.0, "totalHeight": height, "totalWidth": width, "addGuides": false})
}

/// Split Into Grid's fields: the rows on the left and the columns on the right, each with its
/// number, size, gutter and total, and Add Guides. The size of a row or column is what the total
/// leaves each of them after the gutters, so it follows the other three; typing it sets the total.
fn split_grid(ui: &mut egui::Ui, d: &mut Dialog, unit: Unit) {
    const WIDTH: f32 = 84.0;
    let axes = [
        ("rows", tl!("Rows:"), tl!("Height:"), "rowGutter", "totalHeight"),
        ("columns", tl!("Columns:"), tl!("Width:"), "columnGutter", "totalWidth"),
    ];
    let gutter_label = form::humanized("gutter");
    egui::Grid::new("dlg").num_columns(4).spacing([10.0, 8.0]).show(ui, |ui| {
        for (number, label, ..) in axes {
            let row = form::row_label(ui, label);
            ui.label_inputs_by(row.id, |ui| {
                if let Some(v) = crate::widgets::range_field(ui, ("dlg-num", number), d.f64(number, 2.0), 1.0..=500.0, "", 0, WIDTH) {
                    d.fields.insert(number.into(), json!(v));
                }
            });
        }
        ui.end_row();
        for (number, _, label, gutter, total) in axes {
            let row = form::row_label(ui, label);
            let n = d.f64(number, 2.0).max(1.0);
            let gaps = d.f64(gutter, 0.0) * (n - 1.0);
            ui.label_inputs_by(row.id, |ui| {
                if let Some(v) = crate::widgets::num_field(ui, ("dlg-size", number), Some((d.f64(total, 0.0) - gaps) / n), unit, WIDTH) {
                    d.fields.insert(total.into(), json!(n * v + gaps));
                }
            });
        }
        ui.end_row();
        for (_, axis, _, gutter, _) in axes {
            // The visible rows stay the existing translated "Gutter:" and "Total:" labels; the
            // accessible names say which axis they belong to (two identical visible labels would
            // otherwise name two different inputs the same).
            let _ = form::row_label(ui, &gutter_label);
            ui.name_inputs(format!("{axis} {gutter_label}"), |ui| form::length(ui, d, gutter, unit, WIDTH));
        }
        ui.end_row();
        for (_, axis, _, _, total) in axes {
            let _ = form::row_label(ui, tl!("Total:"));
            ui.name_inputs(format!("{axis} {}", tl!("Total:")), |ui| form::length(ui, d, total, unit, WIDTH));
        }
        ui.end_row();
    });
    ui.add_space(8.0);
    form::check(ui, d, "addGuides", tl!("Add Guides"));
}

/// The fields the Simplify dialog opens with at `tolerance` (points): the slider's tolerance when
/// its More Options opens the dialog.
pub(crate) fn simplify_dialog(tolerance: f64) -> Value {
    json!({"tolerance": tolerance, "angleThreshold": ANGLE_THRESHOLD, "straightLines": false, "showOriginal": false, "retain": false, "preview": true})
}

/// The fields the Simplify dialog keeps when OK confirms it with `retain` ticked.
const SIMPLIFY_KEPT: [&str; 6] = ["tolerance", "angleThreshold", "straightLines", "showOriginal", "retain", "preview"];

/// Simplify's fields: Tolerance, Corner Point Angle Threshold (its label above a slider from 0° on
/// the left, the smoothest, to 180°, the sharpest), Convert to Straight Lines, Show Original Path
/// and Open this dialog with these settings next time.
fn simplify_fields(ui: &mut egui::Ui, d: &mut Dialog, unit: Unit) {
    let rail = Tokens::get(ui.ctx()).input_border;
    egui::Grid::new("dlg").num_columns(2).spacing([10.0, 8.0]).show(ui, |ui| {
        form::length_field(ui, d, "tolerance", &form::humanized("tolerance"), unit);
    });
    ui.add_space(8.0);
    form::row_label(ui, tl!("Corner Point Angle Threshold:"));
    ui.horizontal(|ui| form::slider_field_named(ui, d, "angleThreshold", tl!("Corner Point Angle Threshold:"), 0.0..=180.0, "°", &|_| rail));
    ui.add_space(8.0);
    form::check(ui, d, "straightLines", tl!("Convert to Straight Lines"));
    form::check(ui, d, "showOriginal", tl!("Show Original Path"));
    form::check(ui, d, "retain", tl!("Open this dialog with these settings next time"));
}

/// Simplify's Show Original Path: while the dialog previews with it on, every path the preview ran
/// on, with its id, as it was before the preview (from the interaction's snapshot). Empty
/// otherwise.
pub(crate) fn original_paths(app: &VectorcraftApp) -> Vec<(NodeId, &PathData)> {
    let Some(d) = app.ui.dialog.as_ref().filter(|d| d.kind == "simplify" && d.bool("showOriginal") && d.bool("preview")) else { return vec![] };
    let (Some(ids), Some(it)) = (d.fields.get(ORIGINAL_IDS).and_then(Value::as_array), app.session.active().and_then(|st| st.interaction.as_ref()))
    else {
        return vec![];
    };
    ids.iter()
        .filter_map(|id| {
            let id = NodeId(id.as_u64()?);
            match &it.doc.node(id)?.kind {
                NodeKind::Path { path, .. } => Some((id, path)),
                _ => None,
            }
        })
        .collect()
}

fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let unit = app.session.general_unit();
    if d.kind == "splitIntoGrid" {
        split_grid(ui, d, unit);
    } else if d.kind == "simplify" {
        simplify_fields(ui, d, unit);
    } else {
        form::grid(ui, d, unit);
    }
    if let Some(label) = previewed(&d.kind) {
        let (id, params) = command(d);
        if d.fields.get(form::PREVIEWED) != Some(&params) || !d.bool("preview") {
            d.fields.remove("__counts");
            d.fields.remove(ORIGINAL_IDS);
        }
        if let Some(r) = form::preview_result(app, ui, d, label, id, params)
            && d.kind == "simplify"
        {
            d.fields.insert("__counts".into(), json!({"before": r["before"], "after": r["after"]}));
            d.fields.insert(ORIGINAL_IDS.into(), r["ids"].clone());
        }
        if d.kind == "simplify"
            && d.bool("preview")
            && let Some(counts) = d.fields.get("__counts")
            && let (Some(before), Some(after)) = (counts["before"].as_u64(), counts["after"].as_u64())
        {
            ui.label(format!("{}: {before} → {after}", tl!("Anchor Points")));
        }
    }
    false
}

fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let (id, params) = command(d);
    if previewed(&d.kind).is_none() {
        return run_and_close(app, id, params);
    }
    let r = form::commit_preview(app, id, params)?;
    // Simplify: with retain ticked, the menu item opens this dialog on these fields next time; with it
    // unticked, the menu item opens the slider.
    if d.kind == "simplify" {
        let kept = SIMPLIFY_KEPT.into_iter().filter_map(|k| Some((k.to_string(), d.fields.get(k)?.clone())));
        app.ui.simplify = if d.bool("retain") { Value::Object(kept.collect()) } else { Value::Null };
    }
    // The fields as the dialog held them, so they show as typed when it opens again.
    if d.kind == "offsetPath" {
        // Preview among them: the checkbox keeps its last state across opens (#3195), as
        // Simplify's retained fields keep theirs.
        let kept = ["offset", "joins", "miterLimit", "preview"].into_iter().filter_map(|k| Some((k.to_string(), d.fields.get(k)?.clone())));
        app.ui.offset_path = Value::Object(kept.collect());
    }
    Ok(r)
}
