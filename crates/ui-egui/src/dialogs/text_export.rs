//! Export As → Text and Export Selection to `.txt`: the Text Export Options (encoding, line
//! endings, selection only) for a picked `.txt` path. Fields are `document.export` params
//! (`format`, `path`…).

use serde_json::{Value, json};

use super::png_options::choice;
use super::{DialogSpec, form};
use crate::state::Dialog;
use crate::theme::Tokens;
use crate::{VectorcraftApp, io};

pub(crate) const KIND: &str = "txtOptions";

pub(super) const SPEC: DialogSpec =
    DialogSpec { heading: |_| tl!("Text Export Options").into(), body, confirm, ok: Some("Export"), min_width: 340.0, ..DialogSpec::FORM };

const ENCODINGS: [&str; 2] = ["utf8", "utf16"];
const ENCODING_LABELS: [&str; 2] = ["UTF-8", "UTF-16 (Unicode)"];
const LINE_ENDINGS: [&str; 2] = ["lf", "crlf"];
const LINE_ENDING_LABELS: [&str; 2] = ["LF (macOS, Linux)", "CRLF (Windows)"];

/// Open the options for exporting the document's text to `path`.
pub fn open(app: &mut VectorcraftApp, path: &str) {
    // The platform's own line endings by default.
    let eol = if cfg!(windows) { LINE_ENDINGS[1] } else { LINE_ENDINGS[0] };
    let fields = json!({"format": "txt", "path": path, "encoding": ENCODINGS[0], "lineEndings": eol, "selectionOnly": false});
    app.ui.dialog = Some(Dialog::new(KIND, fields));
}

fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let t = Tokens::get(ui.ctx());
    let label = |ui: &mut egui::Ui, text: &str| ui.label(egui::RichText::new(text).color(t.text_dim));
    egui::Grid::new("text-export").num_columns(2).spacing([10.0, 8.0]).show(ui, |ui| {
        label(ui, tl!("Encoding:"));
        choice(ui, d, "encoding", &ENCODINGS, &ENCODING_LABELS);
        ui.end_row();
        label(ui, tl!("Line Endings:"));
        choice(ui, d, "lineEndings", &LINE_ENDINGS, &LINE_ENDING_LABELS);
        ui.end_row();
    });
    ui.add_space(8.0);
    // Export Selection keeps Selection Only on.
    let selected = app.session.active().is_some_and(|st| !st.selection.objects.is_empty()) && !d.bool(super::export_as::SELECTION_FIELD);
    ui.add_enabled_ui(selected, |ui| form::check(ui, d, "selectionOnly", tl!("Selection Only")));
    false
}

/// Write the text file (the command runs first, so a bad option keeps the dialog open). Opened by
/// Export Selection, it writes `document.exportSelection`'s text.
fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let mut params = form::params(d);
    let id = if d.bool(super::export_as::SELECTION_FIELD) {
        if let Some(o) = params.as_object_mut() {
            o.remove("selectionOnly");
        }
        "document.exportSelection"
    } else {
        "document.export"
    };
    let path = io::save_command_output(app, id, "txt", params)?;
    app.ui.dialog = None;
    Ok(json!({ "path": path }))
}
