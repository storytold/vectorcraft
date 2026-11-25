//! Divide into Dashes in the interface: its dialog (dash and gap lengths, Fit to Corners and Ends,
//! Remove Gaps), which previews on the canvas and runs `addon.pathtools.divideDashes` on OK.

use craft::ui::patterns::form_dialogs::{DialogFields, FieldValue, Param, ParamDialog, ParamKind};
use serde_json::{Value, json};

use crate::VectorcraftApp;
use crate::addons::dialogs::{self, Spec};
use crate::canvas::rulers::general_lengths;

/// The command.
pub const COMMAND: &str = "addon.pathtools.divideDashes";
/// The dash and gap lengths the command takes (pt).
const DASHES: std::ops::RangeInclusive<f64> = 0.01..=1e6;
const GAPS: std::ops::RangeInclusive<f64> = 0.0..=1e6;

/// How the dialog's values reach the command.
pub(crate) static SPEC: Spec = Spec { command: COMMAND, params, found: None };

/// The command's parameters from the dialog's fields.
pub(crate) fn params(fields: &DialogFields) -> Value {
    json!({
        "dash": fields.number("dash").unwrap_or(10.0),
        "gap": fields.number("gap").unwrap_or(0.0),
        "fit": fields.bool("fit").unwrap_or(false),
        "removeGaps": fields.bool("removeGaps").unwrap_or(true),
    })
}

/// Opens Divide into Dashes' dialog on the command's defaults.
pub(crate) fn open(app: &mut VectorcraftApp) -> Result<Value, String> {
    let fields = DialogFields::from_pairs([
        ("dash", FieldValue::Number(10.0)),
        ("gap", FieldValue::Number(0.0)),
        ("fit", FieldValue::Bool(false)),
        ("removeGaps", FieldValue::Bool(true)),
    ]);
    let lengths = general_lengths(app);
    dialogs::open(app, &SPEC, fields, move |_| {
        let rows = vec![
            Param::number("dash", tl!("Dash:"), 10.0, 1.0, 2).length(lengths).bounds(Some(*DASHES.start()), Some(*DASHES.end())),
            Param::number("gap", tl!("Gap:"), 0.0, 1.0, 2).length(lengths).bounds(Some(*GAPS.start()), Some(*GAPS.end())),
            Param::new("fit", tl!("Fit to Corners and Ends"), ParamKind::bool(false)),
            // Without a gap there are no gap pieces to remove.
            // PLAN: assumed `Param::enabled_above(key, threshold)` (pl.F4, forms-w1)
            Param::new("removeGaps", tl!("Remove Gaps"), ParamKind::bool(true)).enabled_above("gap", 0.0),
        ];
        ParamDialog::new(COMMAND, tl!("Divide into Dashes"), rows).previews(true)
    })
}
