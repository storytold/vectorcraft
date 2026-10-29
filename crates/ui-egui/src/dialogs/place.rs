//! File → Place: the files picked, one line each about what they are, and the Place options row
//! (Link, Template, Replace). OK places one file centred in the view (or in place of the selected
//! object) or loads the place cursor with several ([`crate::place::confirm`]).
//!
//! Fields: `files` (`[{path} | {name}]`), `link`, `template`, `replace`, and `__replace` (one file
//! and one selected object: Replace applies), `__documents` and `__others` (VectorCraft documents
//! from files are picked, other files are: what Link's tooltip explains), `__info` (a line per
//! file; on the desktop the lines are read off the UI thread and are empty until they are read,
//! [`crate::place::fill_lines`]).

use serde_json::{Value, json};

use super::DialogSpec;
use crate::state::Dialog;
use crate::theme::{self, Tokens};
use crate::{VectorcraftApp, widgets};

/// The dialog kind of Place.
pub const KIND: &str = "place";

pub(super) const SPEC: DialogSpec = DialogSpec {
    heading: |_| tl!("Place").into(),
    body,
    confirm: crate::place::confirm,
    ok: Some("Place"),
    min_width: 340.0,
    max_width: Some(460.0),
    ..DialogSpec::FORM
};

/// The options row: (field, label, tooltip).
const OPTIONS: [(&str, &str, &str); 3] = [
    ("link", "Link", "Keep a link to an image file instead of only embedding it"),
    ("template", "Template", "Place onto a new locked, dimmed template layer below the current layer"),
    ("replace", "Replace", "Swap the selected object for the file, keeping its place and transform"),
];

fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    if crate::place::fill_lines(app, d) {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
    }
    let t = Tokens::get(ui.ctx());
    let files: Vec<Value> = d.fields.get("files").and_then(Value::as_array).cloned().unwrap_or_default();
    let info = d.fields.get("__info").cloned().unwrap_or_default();
    widgets::subheader(ui, if files.len() == 1 { tl!("File") } else { tl!("Files") });
    widgets::list_box(ui, |ui| {
        egui::ScrollArea::vertical().max_height(180.0).auto_shrink([false, true]).show(ui, |ui| {
            ui.set_width(ui.available_width());
            for (i, f) in files.iter().enumerate() {
                let path = f.get("path").and_then(Value::as_str);
                let name = path
                    .map(vectorcraft_engine::cmd::fileio::file_name)
                    .or_else(|| f.get("name").and_then(Value::as_str).map(str::to_string))
                    .unwrap_or_default();
                ui.horizontal(|ui| {
                    ui.add_space(6.0);
                    ui.vertical(|ui| {
                        ui.add_space(3.0);
                        let label = ui.label(egui::RichText::new(name).font(theme::semibold(12.0)).color(t.text_strong));
                        if let Some(p) = path {
                            label.on_hover_text(p);
                        }
                        ui.label(egui::RichText::new(info[i].as_str().unwrap_or_default()).size(11.5).color(t.text_dim));
                        ui.add_space(3.0);
                    });
                });
            }
        });
    });
    if files.len() > 1 {
        ui.add_space(4.0);
        widgets::dim_label(ui, tl!("Click to place each file at 100%, or drag to size it; arrow keys switch files, Esc skips one."));
    }
    ui.add_space(10.0);
    // The options row.
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 18.0;
        for (key, label, tip) in OPTIONS {
            let on = d.bool(key);
            let enabled = key != "replace" || d.bool("__replace");
            let tip = if key == "link" { link_tip(d) } else { tl!(tip).to_string() };
            if widgets::check_tip(ui, tl!(label), on && enabled, enabled, &tip) {
                d.fields.insert(key.into(), json!(!on));
            }
        }
    });
    false
}

/// Link's tooltip: what it does for the files picked (images, VectorCraft documents, or both), a
/// line each.
pub(crate) fn link_tip(d: &Dialog) -> String {
    let lines = match (d.bool("__documents"), d.bool("__others")) {
        (true, false) => {
            vec![
                tl!("On: one locked object showing the document, updated when it changes."),
                tl!("Off: an editable copy of its art, embedded in this document."),
            ]
        }
        (true, true) => {
            vec![
                tl!("Images: link to the file instead of only embedding it."),
                tl!("VectorCraft documents: one locked object, updated when the file changes."),
            ]
        }
        _ => vec![tl!("Keep a link to an image file instead of only embedding it")],
    };
    lines.join("\n")
}
