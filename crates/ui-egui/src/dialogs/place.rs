//! File → Place: the files picked, one line each about what they are, and the Place options row
//! (Link, Template, Replace, Show Import Options). OK places one file centred in the view (or in
//! place of the selected object) or loads the place cursor with several ([`crate::place::confirm`]).
//!
//! Fields: `files` (`[{path} | {name}]`), `link`, `template`, `replace`, `showImportOptions` (each
//! PDF, DXF or text file asks for its import options before it is placed), and `__replace` (one file
//! and one selected object: Replace applies), `__documents` and `__others` (VectorCraft documents
//! from files are picked, other files are: what Link's tooltip explains), `__info` (a line per
//! file; on the desktop the lines are read off the UI thread and are empty until they are read,
//! [`crate::place::fill_lines`]).

use craft::ui::choice::Checkbox;
use craft::ui::contract::{Bind, Props as _, Show as _, ShowContainer as _};
use craft::ui::dialog::{DialogId, DialogResult, DialogStack, StackDialog, dialog_id};
use craft::ui::display::{Shade, Text};
use craft::ui::layout_components::Row;
use craft::ui::patterns::form_dialogs::FormDialog;
use egui::Ui;
use serde_json::{Value, json};
use vectorcraft_engine::cmd::fileio;

use crate::place::PlaceOptions;
// PLAN: assumed craft_ui paths for Checkbox and the other components, as in place.rs.

/// The host key of the Place dialog.
pub const KIND: &str = "place";

/// The options row: (field, label, tooltip).
const OPTIONS: [(&str, &str, &str); 4] = [
    ("link", "Link", "Keep a link to an image file instead of only embedding it"),
    ("template", "Template", "Place onto a new locked, dimmed template layer below the current layer"),
    ("replace", "Replace", "Swap the selected object for the file, keeping its place and transform"),
    (
        "showImportOptions",
        "Show Import Options",
        "Ask for each PDF, DXF or text file's import options before placing it; off, use the options chosen last",
    ),
];

/// The Place dialog: the files to place with a line about each, and the options. Its answer is the
/// files and the options chosen.
pub(crate) struct PlaceDialog {
    id: DialogId,
    /// The files (`{path}` or `{name}`).
    pub(crate) files: Vec<Value>,
    /// A line about each file; empty until read where the lines are read off the UI thread.
    pub(crate) info: Vec<String>,
    options: PlaceOptions,
    /// One file and one selected object: Replace applies.
    replace_enabled: bool,
    /// VectorCraft documents are among the files.
    documents: bool,
    /// Other files are among them.
    others: bool,
}

impl PlaceDialog {
    /// The dialog for `files` with their `info` lines, opened with `options`.
    pub(crate) fn new(files: Vec<Value>, info: Vec<String>, options: PlaceOptions, replace_enabled: bool, documents: bool, others: bool) -> Self {
        PlaceDialog { id: dialog_id(KIND), files, info, options, replace_enabled, documents, others }
    }

    /// The options as OK answers them: Replace only where it applies.
    fn answer(&self) -> (Vec<Value>, PlaceOptions) {
        (self.files.clone(), PlaceOptions { replace: self.options.replace && self.replace_enabled, ..self.options.clone() })
    }
}

impl StackDialog for PlaceDialog {
    type Result = (Vec<Value>, PlaceOptions);

    fn id(&self) -> DialogId {
        self.id
    }

    fn key(&self) -> Option<&str> {
        Some(KIND)
    }

    fn json_fields(&self) -> Option<Value> {
        let o = &self.options;
        Some(json!({
            "files": self.files,
            "link": o.link,
            "template": o.template,
            "replace": o.replace,
            "showImportOptions": o.show_import_options,
            "__replace": self.replace_enabled,
            "__documents": self.documents,
            "__others": self.others,
            "__info": self.info,
        }))
    }

    fn set_json_field(&mut self, ctx: &egui::Context, field: &str, value: Value) -> Result<(), String> {
        if field == "replace" && !self.replace_enabled {
            return Err(format!("`{field}` takes no input now: its row is hidden or disabled"));
        }
        let Some(flag) = self.options.flag(field) else {
            return Err(match field {
                "files" | "__replace" | "__documents" | "__others" | "__info" => format!("`{field}` is read only"),
                _ => format!("unknown field `{field}`"),
            });
        };
        *flag = value.as_bool().ok_or_else(|| format!("`{field}` can't take {value}"))?;
        ctx.request_repaint();
        Ok(())
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<Self::Result>> {
        let (files, info, replace_enabled, documents, others) = (&self.files, &self.info, self.replace_enabled, self.documents, self.others);
        let shown = FormDialog::new(tl!("Place").to_string(), Bind::bound(&mut self.options)).ok_label(tl!("Place").to_string()).show(
            ui,
            |ui, _, options| {
                Text::new(if files.len() == 1 { tl!("File") } else { tl!("Files") }.to_string()).show(ui);
                for (i, file) in files.iter().enumerate() {
                    let path = file.get("path").and_then(Value::as_str);
                    let name =
                        path.map(fileio::file_name).or_else(|| file.get("name").and_then(Value::as_str).map(str::to_string)).unwrap_or_default();
                    let mut label = Text::new(name);
                    if let Some(path) = path {
                        label = label.tooltip(path.to_string());
                    }
                    label.show(ui);
                    Text::new(info.get(i).cloned().unwrap_or_default()).shade(Shade::Muted).show(ui);
                }
                if files.len() > 1 {
                    Text::new(tl!("Click to place each file at 100%, or drag to size it; arrow keys switch files, Esc skips one.").to_string())
                        .shade(Shade::Muted)
                        .show(ui);
                }
                Row::new().wrap(true).show(ui, |ui, _| {
                    for (key, label, tip) in OPTIONS {
                        let tip = if key == "link" { link_tip(documents, others) } else { tl!(tip).to_string() };
                        if let Some(flag) = options.flag(key) {
                            Checkbox::new(Bind::bound(flag))
                                .label(tl!(label).to_string())
                                .tooltip(tip)
                                .enabled(key != "replace" || replace_enabled)
                                .show(ui);
                        }
                    }
                });
                true
            },
        );
        match (shown.accepted, shown.closed) {
            (Some(_), _) => Some(DialogResult::Ok(self.answer())),
            (None, Some(_)) => Some(DialogResult::Cancel),
            (None, None) => None,
        }
    }

    fn confirm(&mut self, _: &egui::Context) -> Option<Self::Result> {
        Some(self.answer())
    }
}

/// Link's tooltip: what it does for the files picked (images, VectorCraft documents, or both), a
/// line each.
pub(crate) fn link_tip(documents: bool, others: bool) -> String {
    let lines = match (documents, others) {
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
