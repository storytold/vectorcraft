//! Unit Converter (`addon.units.*`), a built-in add-on that shows one length in every unit at once.
//! `addon.units.convert` reads a length as a number field reads it ([`Unit::parse`]: units and
//! arithmetic) and gives it in each unit of Preferences › Units › East Asian Type
//! ([`Unit::EAST_ASIAN`]), written as the fields write it ([`Unit::number`], [`Unit::format`]).
//! `addon.units.selectionSize` gives the selection's width and height, measured as the Transform
//! panel measures them, in the same units. Its panel and its place in Object › Plug-ins are the
//! UI's; the panel applies a length to the selection with `object.setBounds`.

#[cfg(test)]
mod tests;

use serde_json::{Value, json};
use vectorcraft_doc::{OrientedBox, Unit, arithmetic_signs, decimal_point, finite_in_every_unit};
use vectorcraft_render::effects;

use super::super::{CommandSpec, always, bad, cmd, has_selection};
use crate::{Result, Session};

const CONVERT: &str = "addon.units.convert";
const SELECTION_SIZE: &str = "addon.units.selectionSize";
/// The most characters of a refused value an error message shows.
const SHOWN_CHARS: usize = 64;
/// The longest `value` or `unit` text `addon.units.convert` reads, in bytes.
const MAX_TEXT_LEN: usize = 1024;
/// The most `+ - * /` signs `addon.units.convert` reads in one `value`, a multiplication sign
/// counted as a `*`: [`Unit::parse`] takes one level of recursion per sign.
const MAX_OPERATORS: usize = 128;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "addon.units.convert",
            "Convert Units",
            [],
            None,
            "{value: a length (a number, or text as a number field reads it, with a unit suffix and + - * / arithmetic, x, X or × multiplying as * does: \"10mm\", \"1in/2\", \"2p6\", \"3cm*50%\", \"2x3 mm\"; a decimal comma reads as a point: \"0,5 mm\"; at most 1024 bytes and 128 operators, each multiplication sign counted), unit?: the unit of a number without a suffix (a unit's name, suffix or key: points, picas, inches, millimeters, centimeters, pixels, feetInches, meters, yards, feet, q, ha; default: points)} the length in every unit of Preferences › Units › East Asian Type, written as the number fields write it (rounded to 3 decimals in pt, px, picas, Q and Ha and to 4 in the others; pixels are points). Nothing in the document changes → {points, units: [{unit (its key), label, number (no suffix), text (with the suffix)}]}",
            always,
            convert
        ),
        cmd!(
            query "addon.units.selectionSize",
            "Selection Size in Units",
            [],
            None,
            "{} the selection's width and height as the Transform panel shows them (its bounding box, rotated with rotated objects; with Use Preview Bounds, the visual bounds; with anchor points selected (Direct Selection), the box around those points), each as addon.units.convert gives it → {width: {points, units}, height: {points, units}}",
            has_selection,
            selection_size
        ),
    ]
}

/// The length `pt` (points) in every unit of [`Unit::EAST_ASIAN`]: `{points, units: [{unit,
/// label, number, text}]}`.
pub fn in_every_unit(pt: f64) -> Value {
    let units: Vec<Value> =
        Unit::EAST_ASIAN.iter().map(|u| json!({"unit": u.key(), "label": u.label(), "number": u.number(pt), "text": u.format(pt)})).collect();
    json!({ "points": pt, "units": units })
}

/// The unit `p` gives for a number without a suffix (points when left out; null is an error).
fn unit_param(p: &Value) -> Result<Unit> {
    let Some(given) = p.get("unit") else { return Ok(Unit::Points) };
    let named = given.as_str().filter(|name| name.len() <= MAX_TEXT_LEN).and_then(Unit::named_east_asian);
    named.ok_or_else(|| {
        let u = match given {
            Value::String(s) => clipped(s).escape_debug().to_string(),
            other => shown(other),
        };
        bad(CONVERT, format!("unknown units `{u}`"))
    })
}

fn convert(_: &mut Session, p: &Value) -> Result<Value> {
    let unit = unit_param(p)?;
    let Some(v) = p.get("value").filter(|v| !v.is_null()) else {
        return Err(bad(CONVERT, "missing `value`"));
    };
    let pt = match v {
        Value::String(s) if readable(s) => unit.parse(&decimal_point(s)),
        Value::Number(n) => n.as_f64().map(|n| unit.to_pt(n)),
        _ => None,
    };
    let pt = pt.filter(|&pt| finite_in_every_unit(pt));
    pt.map(in_every_unit).ok_or_else(|| {
        let v = shown(v);
        bad(CONVERT, format!("`value` must be a number, not {v}"))
    })
}

/// Is `s` short enough to read: at most [`MAX_TEXT_LEN`] bytes and [`MAX_OPERATORS`] `+ - * /`
/// signs, a multiplication sign typed as `x`, `X` or `×` counted as a `*` ([`arithmetic_signs`])?
fn readable(s: &str) -> bool {
    s.len() <= MAX_TEXT_LEN && arithmetic_signs(s) <= MAX_OPERATORS
}

/// `v` as an error message shows it: text cut at [`SHOWN_CHARS`] characters with "…", as JSON,
/// an array as `[…]` and an object as `{…}`.
fn shown(v: &Value) -> String {
    match v {
        Value::String(s) => Value::String(clipped(s)).to_string(),
        Value::Array(_) => "[…]".into(),
        Value::Object(_) => "{…}".into(),
        other => other.to_string(),
    }
}

/// `s` cut at [`SHOWN_CHARS`] characters, with "…" when it is longer.
fn clipped(s: &str) -> String {
    match s.char_indices().nth(SHOWN_CHARS) {
        Some((at, _)) => format!("{}…", s.get(..at).unwrap_or_default()),
        None => s.to_string(),
    }
}

fn selection_size(s: &mut Session, _: &Value) -> Result<Value> {
    let st = s.doc()?;
    let measure = s.bounds_measure();
    let b = match effects::preview::anchor_bounds_with(&st.selection, &st.doc, measure) {
        Some(r) => Some(OrientedBox::aligned(r)),
        None => s.transform_box(&st.selection.objects),
    };
    let b = b.ok_or_else(|| bad(SELECTION_SIZE, "nothing selected"))?;
    Ok(json!({ "width": in_every_unit(b.rect.width()), "height": in_every_unit(b.rect.height()) }))
}
