//! Export As → Text and Export Selection to `.txt`: the Text Export Options (encoding, line
//! endings, selection only) for a picked `.txt` path. Fields are `document.export` params
//! (`format`, `path`…).

use craft::ui::contract::Bind;
use craft::ui::patterns::form_dialogs::{DialogFields, FieldValue, HostedParamDialog, Param, ParamDialog, ParamKind};
use craft::ui::runtime::env::Label;
use serde_json::Value;
use vectorcraft_doc::NodeId;

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

/// The drawing and selected objects a Selection Only export started with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SelectionSource {
    drawing: (u64, u64),
    objects: Vec<NodeId>,
}

impl SelectionSource {
    fn of(app: &VectorcraftApp) -> Option<Self> {
        let state = app.session.active()?;
        let mut objects = state.selection.objects.clone();
        objects.sort_unstable();
        Some(Self { drawing: (state.uid, state.reverts), objects })
    }
}

/// Open the options for exporting the document's text to `path`; `selection` when Export Selection opened them.
pub fn open(app: &mut VectorcraftApp, path: &str, selection: bool) -> Result<(), String> {
    let ctx = app.ui_ctx.clone().ok_or("no window")?;
    // Export Selection keeps Selection Only on.
    let source = SelectionSource::of(app);
    let can_select = !selection && source.as_ref().is_some_and(|source| !source.objects.is_empty());
    // The platform's own line endings by default.
    let eol = if cfg!(windows) { LINE_ENDINGS[1] } else { LINE_ENDINGS[0] };
    let mut fields = DialogFields::from_pairs([("format", "txt"), ("path", path), ("encoding", ENCODINGS[0]), ("lineEndings", eol)]);
    fields.set("selectionOnly", selection);
    let dialog = HostedParamDialog::new(&ctx, KIND, fields, move |fields| {
        ParamDialog::new(KIND, tl!("Text Export Options"), params(can_select)).values(Bind::bound(fields)).ok_label(Label::from("Export"))
    });
    export_host::open(app, Export::Text { source }, selection, dialog).map(|_| ()).ok_or_else(|| "no window".into())
}

/// Write the text file (the command runs first, so a bad option keeps the dialog open). Opened by
/// Export Selection, it writes `document.exportSelection`'s text.
pub(super) fn confirm(app: &mut VectorcraftApp, open: &Open, source: Option<&SelectionSource>, fields: &DialogFields) -> Result<After, String> {
    if (open.selection || fields.bool("selectionOnly") == Some(true)) && SelectionSource::of(app).as_ref() != source {
        return Err(tl!("Select the original artwork to export it, or reopen Text Export Options.").into());
    }
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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::super::tests_export::{app_saving, frame, kind, set, try_confirm};

    #[test]
    fn selection_only_text_export_stays_on_the_opening_artwork() {
        for export_selection in [true, false] {
            let (mut app, written) = app_saving(1, |_| "/output/selection.txt".into());
            let first = app.run("text.create", json!({"x": 5, "y": 10, "text": "First text"})).unwrap()["id"].clone();
            let second = app.run("text.create", json!({"x": 5, "y": 25, "text": "Second text"})).unwrap()["id"].clone();
            app.run("select.set", json!({"ids": [first]})).unwrap();
            if export_selection {
                app.run("document.exportSelection", json!({})).unwrap();
                frame(&mut app);
            } else {
                app.run("file.exportAs", json!({"format": "txt"})).unwrap();
                try_confirm(&mut app).unwrap();
                set(&mut app, "selectionOnly", json!(true));
            }
            assert_eq!(kind(&app).as_deref(), Some("txtOptions"));

            app.run("select.set", json!({"ids": [second]})).unwrap();
            let why = try_confirm(&mut app).expect_err("the original selection is required");
            assert!(why.contains("original artwork"));
            assert_eq!(kind(&app).as_deref(), Some("txtOptions"));
            assert!(written.borrow().is_empty());

            app.run("select.set", json!({"ids": [first]})).unwrap();
            try_confirm(&mut app).unwrap();
            let output = written.borrow();
            assert_eq!(output.len(), 1);
            let contents = std::str::from_utf8(&output[0].1).unwrap();
            assert!(contents.contains("First text"));
            assert!(!contents.contains("Second text"));
        }
    }
}
