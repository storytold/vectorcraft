//! File → Save for Office Documents…: an artboard as a PNG for office documents and slides, at
//! 72, 150 or 300 ppi, on white or transparent. Fields are `document.exportForOffice` params.

use craft::ui::contract::Bind;
use craft::ui::patterns::form_dialogs::{DialogFields, HostedParamDialog, Param, ParamDialog, ParamKind};
use craft::ui::runtime::env::Label;
use serde_json::{Value, json};

use super::export_host::{self, After, Export, Open};
use super::png_options::{RESOLUTION_LABELS, RESOLUTIONS};
use crate::{VectorcraftApp, io};

/// The dialog's key.
pub(crate) const KIND: &str = "saveForOffice";

/// The document's artboards as the dialog needs them: their names and the size each has at every resolution
/// (`sizes[board][resolution]`), worked out when the dialog opens.
struct Boards {
    names: Vec<String>,
    sizes: Vec<Vec<String>>,
}

impl Boards {
    fn of(doc: &vectorcraft_doc::Document) -> Self {
        let sizes = doc
            .artboards
            .iter()
            .map(|a| {
                RESOLUTIONS
                    .iter()
                    .map(|ppi| {
                        let (w, h) = vectorcraft_render::region_pixels(a.rect, ppi / 72.0);
                        format!("{w} × {h} px")
                    })
                    .collect()
            })
            .collect();
        Boards { names: doc.artboards.iter().map(|a| a.name.clone()).collect(), sizes }
    }
}

/// A number field's value, also when a client set it as the text of a choice's id.
fn number(fields: &DialogFields, key: &str) -> Option<f64> {
    fields.number(key).or_else(|| fields.text(key)?.parse().ok())
}

fn params(boards: &Boards, fields: &DialogFields) -> Vec<Param> {
    let ppi = number(fields, "ppi").unwrap_or(RESOLUTIONS[1]);
    let at = RESOLUTIONS.iter().position(|r| *r == ppi).unwrap_or(1);
    let board = (number(fields, "artboard").unwrap_or(0.0).max(0.0) as usize).min(boards.names.len().saturating_sub(1));
    let resolutions = RESOLUTIONS.iter().zip(RESOLUTION_LABELS.iter()).map(|(r, label)| (r.to_string(), Label::from(*label))).collect();
    let mut rows = vec![Param::new("ppi", tl!("Resolution:"), ParamKind::choice(resolutions, Some(RESOLUTIONS[1].to_string())))];
    if boards.names.len() > 1 {
        let names = boards.names.iter().enumerate().map(|(i, name)| (i.to_string(), Label::from(name.clone()))).collect();
        rows.push(Param::new("artboard", tl!("Artboard:"), ParamKind::choice(names, Some("0".into()))));
    }
    let size = boards.sizes.get(board).and_then(|s| s.get(at)).cloned().unwrap_or_default();
    rows.push(Param::read_only_text("size", tl!("Size:"), size));
    rows.push(Param::new("transparent", tl!("Transparent Background"), ParamKind::bool(false)));
    rows
}

/// Open the dialog (150 ppi on white, the first artboard).
pub fn open(app: &mut VectorcraftApp) -> Result<Value, String> {
    let boards = Boards::of(&app.session.active().ok_or("no document")?.doc);
    let ctx = app.ui_ctx.clone().ok_or("no window")?;
    let mut fields = DialogFields::from_pairs([("ppi", RESOLUTIONS[1].to_string()), ("artboard", "0".to_string())]);
    fields.set("transparent", false);
    let dialog = HostedParamDialog::new(&ctx, KIND, fields, move |fields| {
        let rows = params(&boards, fields);
        ParamDialog::new(KIND, tl!("Save for Office Documents"), rows).values(Bind::bound(fields)).ok_label(Label::from("Save…"))
    });
    export_host::open(app, Export::Office, false, dialog).ok_or("no window")?;
    Ok(Value::Null)
}

/// Pick the file and write it (the command runs first: bad options keep the dialog open).
pub(super) fn confirm(app: &mut VectorcraftApp, _open: &Open, fields: &DialogFields) -> Result<After, String> {
    let params = json!({
        "ppi": number(fields, "ppi").unwrap_or(RESOLUTIONS[1]),
        "artboard": number(fields, "artboard").unwrap_or(0.0).max(0.0) as u64,
        "transparent": fields.bool("transparent").unwrap_or(false),
    });
    io::save_command_output(app, "document.exportForOffice", "png", params, |_, _| Ok(Value::Null))?;
    Ok(None)
}
