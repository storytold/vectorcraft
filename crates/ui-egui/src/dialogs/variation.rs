//! The row of a tip value with a variation menu (Angle, Roundness, Size of the Eraser and the Blob Brush,
//! the brush options' scatter rows): the value with its menu of modes, then the Variation field the menu's
//! modes other than Fixed take.

// PLAN: `mod variation;` in `dialogs.rs` belongs to hub-w2 (not in this set's Files table).
use craft::ui::contract::{Bind, Entry, Props as _, Show as _, ShowContainer as _};
use craft::ui::form::Field;
use craft::ui::layout_components::{Row, Stack};
use craft::ui::patterns::form_dialogs::{DialogFields, FieldValue, Param};
use craft::ui::pickers::Select;
use craft::ui::runtime::env::UnitKind;
use craft::ui::text::NumberField;
use egui::{Response, Ui};
use vectorcraft_brush::Variation;

/// A `Number` row `key` with a mode menu before the field and a Variation field under it, greyed while the mode
/// is Fixed. The mode is kept under `<key>Mode` (the mode's id) and the Variation under `<key>Variation`; both
/// are companions of the row, so automation sets them as it sets `key`. `offered` lists the modes the menu has;
/// a stored mode outside it reads as the default.
pub(crate) fn variation_param(
    key: &'static str,
    label: &'static str,
    unit: UnitKind,
    decimals: u32,
    range: (f64, f64),
    variation: (f64, f64),
    offered: &'static [Variation],
) -> Param {
    let (mode_key, var_key) = (format!("{key}Mode"), format!("{key}Variation"));
    let keys = [mode_key.clone(), var_key.clone()];
    Param::custom(key, label, move |ui, fields, _| {
        variation_rows(ui, fields, (key, &mode_key, &var_key), (unit, decimals as usize), (range, variation), offered)
    })
    .companions(keys)
}

/// Draws the rows of [`variation_param`] over `fields` and writes back what the user changed.
fn variation_rows(
    ui: &mut Ui,
    fields: &mut DialogFields,
    (key, mode_key, var_key): (&str, &str, &str),
    (unit, decimals): (UnitKind, usize),
    (range, variation): ((f64, f64), (f64, f64)),
    offered: &[Variation],
) -> Response {
    let mode_at = fields.text(mode_key).and_then(Variation::parse).and_then(|m| offered.iter().position(|o| *o == m)).unwrap_or_default();
    let mode = offered.get(mode_at).copied().unwrap_or_default();
    let entries: Vec<Entry<usize>> = offered.iter().enumerate().map(|(i, m)| Entry::new(i, m.label())).collect();
    let (value, spread) = (fields.number(key).unwrap_or(range.0), fields.number(var_key).unwrap_or(0.0));
    let (mut set_value, mut set_mode, mut set_spread) = (None, None, None);
    let field = |label: &str, shown: f64, (lo, hi): (f64, f64), enabled: bool, salt: &str, ui: &mut Ui| {
        NumberField::new(label.to_owned(), Bind::given(Some(shown)))
            .range(lo..=hi)
            .unit_kind(unit)
            .fixed_decimals(decimals)
            .enabled(enabled)
            .id_salt(salt.to_owned())
            .show(ui)
            .edit
            .value()
            .and_then(|typed| typed.to_owned())
    };
    let shown = Stack::new().fill_width().show(ui, |ui, _| {
        Row::new().show(ui, |ui, _| {
            if let Some(picked) = Select::new(tl!("Mode"), Bind::given(mode_at), &entries).id_salt("mode").show(ui).value() {
                set_mode = offered.get(*picked).copied();
            }
            set_value = field(tl!("Value"), value, range, true, "value", ui);
        });
        Field::new(tl!("Variation:")).id_salt("variation").show(ui, |ui, _| {
            set_spread = field(tl!("Variation"), spread, variation, mode != Variation::Fixed, "spread", ui);
        });
    });
    if let Some(m) = set_mode {
        fields.set(mode_key, FieldValue::Text(m.id().to_owned()));
    }
    if let Some(v) = set_value {
        fields.set(key, FieldValue::Number(v));
    }
    if let Some(v) = set_spread {
        fields.set(var_key, FieldValue::Number(v));
    }
    shown.output
}
