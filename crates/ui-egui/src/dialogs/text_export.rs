//! Export As → Text and Export Selection to `.txt`: the Text Export Options (encoding, line
//! endings, selection only) for a picked `.txt` path. Fields are `document.export` params
//! (`format`, `path`…).

use craft::ui::contract::Bind;
use craft::ui::patterns::form_dialogs::{DialogFields, FieldValue, HostedParamDialog, Param, ParamDialog, ParamKind};
use craft::ui::runtime::env::Label;
use serde_json::Value;

use super::export_host::{self, After, Export, Open, options};
use super::form;
use crate::{VectorcraftApp, io};

pub(crate) const KIND: &str = "txtOptions";

const ENCODINGS: [&str; 2] = ["utf8", "utf16"];
const ENCODING_LABELS: [&str; 2] = ["UTF-8", "UTF-16 (Unicode)"];
const LINE_ENDINGS: [&str; 2] = ["lf", "crlf"];
const LINE_ENDING_LABELS: [&str; 2] = ["LF (macOS, Linux)", "CRLF (Windows)"];

/// The rows. Selection Only is always disabled without a selection to limit the export to (`can_select`).
fn params(can_select: bool) -> Vec<Param> {
    let mut selection_only = Param::new("selectionOnly", tl!("Selection Only"), ParamKind::bool(false));
    if !can_select {
        // An empty list of values never matches, so the row stays disabled.
        selection_only = selection_only.enabled_when_in("encoding", std::iter::empty::<FieldValue>());
    }
    vec![
        Param::new("encoding", tl!("Encoding:"), ParamKind::choice(options(&ENCODINGS, &ENCODING_LABELS), Some(ENCODINGS[0].into()))),
        Param::new("lineEndings", tl!("Line Endings:"), ParamKind::choice(options(&LINE_ENDINGS, &LINE_ENDING_LABELS), None)),
        selection_only,
    ]
}

/// Open the options for exporting the document's text to `path`; `selection` when Export Selection opened them.
pub fn open(app: &mut VectorcraftApp, path: &str, selection: bool) -> Result<(), String> {
    let ctx = app.ui_ctx.clone().ok_or("no window")?;
    // Export Selection keeps Selection Only on.
    let can_select = !selection && app.session.active().is_some_and(|st| !st.selection.objects.is_empty());
    // The platform's own line endings by default.
    let eol = if cfg!(windows) { LINE_ENDINGS[1] } else { LINE_ENDINGS[0] };
    let mut fields = DialogFields::from_pairs([("format", "txt"), ("path", path), ("encoding", ENCODINGS[0]), ("lineEndings", eol)]);
    fields.set("selectionOnly", selection);
    let dialog = HostedParamDialog::new(&ctx, KIND, fields, move |fields| {
        ParamDialog::new(KIND, tl!("Text Export Options"), params(can_select)).values(Bind::bound(fields)).ok_label(Label::from("Export"))
    });
    export_host::open(app, Export::Text, selection, dialog).map(|_| ()).ok_or_else(|| "no window".into())
}

/// Write the text file (the command runs first, so a bad option keeps the dialog open). Opened by
/// Export Selection, it writes `document.exportSelection`'s text.
pub(super) fn confirm(app: &mut VectorcraftApp, open: &Open, fields: &DialogFields) -> Result<After, String> {
    let mut params = Value::Object(form::json_of(&params(true), fields));
    let id = if open.selection {
        if let Some(o) = params.as_object_mut() {
            o.remove("selectionOnly");
        }
        "document.exportSelection"
    } else {
        "document.export"
    };
    io::save_command_output(app, id, "txt", params, |_, _| Ok(Value::Null))?;
    Ok(None)
}
