//! Create Swatch Legend in the interface (Object › Plug-ins › Color Effects › Create Swatch
//! Legend…, #4919): a dialog for the columns, the chip size, the gap and which colour values the
//! labels show, and whether only the swatches selected in the Swatches panel are drawn. It
//! previews on the canvas and runs `addon.color.swatchLegend` on OK.

use craft::ui::patterns::form_dialogs::{DialogFields, FieldValue, Param, ParamDialog, ParamKind};
use serde_json::{Value, json};

use crate::VectorcraftApp;
use crate::addons::dialogs::{self, Spec};
use crate::canvas::rulers::general_lengths;

/// The command.
pub const COMMAND: &str = "addon.color.swatchLegend";
/// The values a label can show: (the command's name, the dialog's label).
pub const VALUES: [(&str, &str); 5] = [("hex", "HEX"), ("rgb", "RGB"), ("cmyk", "CMYK"), ("lab", "Lab"), ("gray", "Grayscale")];

/// How the dialog's values reach the command.
pub(crate) static SPEC: Spec = Spec { command: COMMAND, params, found: None };

/// The command's params from the dialog's fields.
pub(crate) fn params(fields: &DialogFields) -> Value {
    let mut p = json!({
        "columns": fields.int("columns").unwrap_or(4),
        "size": fields.number("size").unwrap_or(72.0),
        "gap": fields.number("gap").unwrap_or(12.0),
    });
    let values: Vec<&str> = VALUES.iter().filter(|(v, _)| fields.bool(v).unwrap_or(false)).map(|(v, _)| *v).collect();
    if !values.is_empty() {
        p["values"] = json!(values);
    }
    let names: Vec<String> = fields
        .list("names")
        .map(|list| list.iter().filter_map(|name| if let FieldValue::Text(text) = name { Some(text.clone()) } else { None }).collect())
        .unwrap_or_default();
    if fields.bool("selectedOnly").unwrap_or(false) && !names.is_empty() {
        p["names"] = json!(names);
    }
    p
}

/// Opens Create Swatch Legend's dialog on the command's defaults, each swatch showing its own values. The colour
/// swatches selected in the Swatches panel are read once, when it opens.
pub(crate) fn open(app: &mut VectorcraftApp) -> Result<Value, String> {
    let picked = crate::panels::swatches::selected(app);
    let names: Vec<FieldValue> = app
        .session
        .active()
        .map(|st| {
            st.doc
                .swatches_iter()
                .filter(|w| w.paint.color().is_some() && picked.contains(&w.name))
                .map(|w| FieldValue::Text(w.name.clone()))
                .collect()
        })
        .unwrap_or_default();
    let fields = DialogFields::from_pairs([
        ("columns", FieldValue::Int(4)),
        ("size", FieldValue::Number(72.0)),
        ("gap", FieldValue::Number(12.0)),
        ("selectedOnly", FieldValue::Bool(false)),
        ("names", FieldValue::List(names)),
    ]);
    let lengths = general_lengths(app);
    dialogs::open(app, &SPEC, fields, move |fields| {
        let mut rows = vec![
            Param::new("columns", tl!("Columns:"), ParamKind::int(Some(1), Some(50), 4)),
            Param::number("size", tl!("Size:"), 72.0, 1.0, 2).length(lengths).bounds(Some(8.0), Some(1000.0)),
            Param::number("gap", tl!("Gap:"), 12.0, 1.0, 2).length(lengths).bounds(Some(0.0), Some(1000.0)),
            // None ticked: each swatch shows the values of its own colour model.
            Param::section(tl!("Show:")),
        ];
        rows.extend(VALUES.iter().map(|&(value, label)| Param::new(value, tl!(label), ParamKind::bool(false))));
        // Only the swatches selected in the Swatches panel, when some are.
        if fields.list("names").is_some_and(|names| !names.is_empty()) {
            rows.push(Param::new("selectedOnly", tl!("Selected Swatches Only"), ParamKind::bool(false)));
        }
        ParamDialog::new(COMMAND, tl!("Create Swatch Legend"), rows).previews(true)
    })
}

#[cfg(test)]
mod tests;
