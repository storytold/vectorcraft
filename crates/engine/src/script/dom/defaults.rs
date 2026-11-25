//! A document's default fill and stroke in the script object model, the `Document` members whose
//! names start with `default` that the help page
//! <https://ai-scripting.docsforadobe.dev/jsobjref/Document/> describes, and `scaleFactor`.
//!
//! The defaults are what the next object drawn gets ([`crate::Session::new_art`]), the same for
//! every open document, as the help page says of these global settings. Reading one reads new
//! art's top fill or stroke, and assigning one runs `appearance.setNewArt`, which leaves the
//! selection, its art, the active proxy and the recent colors as they were. A value that changes
//! nothing runs no command, and a bad value throws an error with nothing changed.
//!
//! VectorCraft's own rules, which the help page does not state:
//! - A color is new art's fill or stroke itself: assigning a color also makes `defaultFilled` or
//!   `defaultStroked` true, and assigning a `NoColor` makes it false. `defaultFilled` or
//!   `defaultStroked` set to true with no default fill or stroke gives black.
//! - An RGB color is stored in the color mode of the document in front, as `paint.setFill` stores
//!   it.
//! - `defaultStrokeWidth` is 0 to 1000 points, a dash or gap 0 to 1,000,000 points (at most 64 of
//!   them, not all 0), and `defaultStrokeDashOffset` -1,000,000 to 1,000,000 points, which needs a
//!   dash pattern unless it is 0.
//! - While New Art Has Basic Appearance is off and an object is selected, assigning a default
//!   throws an error, since new art then takes that object's appearance.
//! - `scaleFactor` is 1: VectorCraft draws every document at its own size.

use serde_json::{Value, json};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, LineCap, LineJoin};
use vectorcraft_js::{Data, HostError, HostMember};

use super::paths::{CAPS, JOINS, MAX_STROKE_WIDTH, dash_lengths};
use super::{Answer, Host, Link, arg, boolean, colors, command, get, num, number, property, this_doc};

/// New art's appearance, after the check that the document `this` names is still open.
fn new_art(h: &Host, a: &[Data]) -> std::result::Result<Appearance, HostError> {
    this_doc(h, a)?;
    Ok(h.session.borrow().new_art())
}

/// Runs `appearance.setNewArt` with `params`, unless `same`.
fn set(h: &Host, same: bool, params: Value) -> Answer {
    if !same {
        command(h, "appearance.setNewArt", &params)?;
    }
    Ok(Data::Undefined)
}

fn top_paint(ap: &Appearance, fill: bool) -> Paint {
    if fill { ap.fill_paint() } else { ap.stroke_paint() }
}

fn default_color(h: &Host, a: &[Data], fill: bool) -> Answer {
    Ok(colors::of_paint(&top_paint(&new_art(h, a)?, fill)))
}

fn set_default_color(h: &Host, a: &[Data], fill: bool) -> Answer {
    let what = if fill { "defaultFillColor" } else { "defaultStrokeColor" };
    let color = colors::assigned(arg(a, 1), what)?;
    let now = top_paint(&new_art(h, a)?, fill);
    let same = colors::unchanged(arg(a, 1), color, &now, |c| crate::cmd::newart::stored_color(&h.session.borrow(), c));
    let paint = match color {
        Some(c) => json!({"color": colors::param(&c)}),
        None => json!({"none": true}),
    };
    let key = if fill { "fill" } else { "stroke" };
    set(h, same, json!({ key: paint }))
}

fn default_painted(h: &Host, a: &[Data], fill: bool) -> Answer {
    Ok(Data::Bool(!top_paint(&new_art(h, a)?, fill).is_none()))
}

fn set_default_painted(h: &Host, a: &[Data], fill: bool) -> Answer {
    let on = boolean(arg(a, 1), if fill { "defaultFilled" } else { "defaultStroked" })?;
    let now = !top_paint(&new_art(h, a)?, fill).is_none();
    let paint = if on { json!({"color": colors::param(&Color::BLACK)}) } else { json!({"none": true}) };
    let key = if fill { "fill" } else { "stroke" };
    set(h, now == on, json!({ key: paint }))
}

fn default_overprint(h: &Host, a: &[Data], fill: bool) -> Answer {
    let ap = new_art(h, a)?;
    Ok(Data::Bool(if fill { ap.fill().is_some_and(|f| f.overprint) } else { ap.stroke().is_some_and(|s| s.overprint) }))
}

fn set_default_overprint(h: &Host, a: &[Data], fill: bool) -> Answer {
    let (key, what) = if fill { ("fillOverprint", "defaultFillOverprint") } else { ("strokeOverprint", "defaultStrokeOverprint") };
    let on = boolean(arg(a, 1), what)?;
    let now = default_overprint(h, a, fill)? == Data::Bool(on);
    set(h, now, json!({ key: on }))
}

/// New art's top stroke option `f` reads, or `none` when new art has no stroke.
fn stroke_option<T>(h: &Host, a: &[Data], none: T, f: impl FnOnce(&vectorcraft_doc::StrokeLayer) -> T) -> std::result::Result<T, HostError> {
    Ok(new_art(h, a)?.stroke().map_or(none, f))
}

fn default_stroke_width(h: &Host, a: &[Data]) -> Answer {
    this_doc(h, a)?;
    Ok(num(h.session.borrow().paint.stroke_width))
}

fn set_default_stroke_width(h: &Host, a: &[Data]) -> Answer {
    let w = number(arg(a, 1), "defaultStrokeWidth")?;
    if !(0.0..=MAX_STROKE_WIDTH).contains(&w) {
        return Err(HostError::Range("defaultStrokeWidth must be from 0 to 1000 points".into()));
    }
    this_doc(h, a)?;
    let now = h.session.borrow().paint.stroke_width;
    set(h, now == w, json!({"weight": w}))
}

fn default_cap(h: &Host, a: &[Data]) -> Answer {
    let cap = stroke_option(h, a, LineCap::default(), |s| s.cap)?;
    Ok(Data::String(CAPS.iter().find(|c| c.0 == cap).map_or("StrokeCap.BUTTENDCAP", |c| c.1).into()))
}

fn set_default_cap(h: &Host, a: &[Data]) -> Answer {
    let found = match arg(a, 1) {
        Data::String(s) => CAPS.iter().find(|c| c.1 == s),
        _ => None,
    };
    let Some(&(cap, _, name)) = found else { return Err(HostError::Type("defaultStrokeCap must be a StrokeCap value".into())) };
    let now = stroke_option(h, a, LineCap::default(), |s| s.cap)?;
    set(h, now == cap, json!({"cap": name}))
}

fn default_join(h: &Host, a: &[Data]) -> Answer {
    let join = stroke_option(h, a, LineJoin::default(), |s| s.join)?;
    Ok(Data::String(JOINS.iter().find(|j| j.0 == join).map_or("StrokeJoin.MITERENDJOIN", |j| j.1).into()))
}

fn set_default_join(h: &Host, a: &[Data]) -> Answer {
    let found = match arg(a, 1) {
        Data::String(s) => JOINS.iter().find(|j| j.1 == s),
        _ => None,
    };
    let Some(&(join, _, name)) = found else { return Err(HostError::Type("defaultStrokeJoin must be a StrokeJoin value".into())) };
    let now = stroke_option(h, a, LineJoin::default(), |s| s.join)?;
    set(h, now == join, json!({"join": name}))
}

fn default_miter(h: &Host, a: &[Data]) -> Answer {
    Ok(num(stroke_option(h, a, 10.0, |s| s.miter_limit)?))
}

fn set_default_miter(h: &Host, a: &[Data]) -> Answer {
    let v = number(arg(a, 1), "defaultStrokeMiterLimit")?;
    if !(1.0..=500.0).contains(&v) {
        return Err(HostError::Range("defaultStrokeMiterLimit must be from 1 to 500".into()));
    }
    let now = stroke_option(h, a, 10.0, |s| s.miter_limit)?;
    set(h, now == v, json!({"miterLimit": v}))
}

fn dashes(h: &Host, a: &[Data]) -> std::result::Result<Vec<f64>, HostError> {
    stroke_option(h, a, Vec::new(), |s| s.dash.as_ref().map_or_else(Vec::new, |d| d.pattern.clone()))
}

fn default_dashes(h: &Host, a: &[Data]) -> Answer {
    Ok(Data::Array(dashes(h, a)?.into_iter().map(num).collect()))
}

fn set_default_dashes(h: &Host, a: &[Data]) -> Answer {
    let lengths = dash_lengths(arg(a, 1), "defaultStrokeDashes")?;
    let now = dashes(h, a)?;
    let dash = if lengths.is_empty() { Value::Null } else { json!(lengths) };
    set(h, now == lengths, json!({"dash": dash}))
}

fn default_dash_offset(h: &Host, a: &[Data]) -> Answer {
    Ok(num(stroke_option(h, a, 0.0, |s| s.dash.as_ref().map_or(0.0, |d| d.offset))?))
}

fn set_default_dash_offset(h: &Host, a: &[Data]) -> Answer {
    let v = number(arg(a, 1), "defaultStrokeDashOffset")?;
    if v.abs() > MAX_STROKE_WIDTH * 1000.0 {
        return Err(HostError::Range("defaultStrokeDashOffset must be from -1000000 to 1000000 points".into()));
    }
    if dashes(h, a)?.is_empty() && v != 0.0 {
        return Err(HostError::Error("defaultStrokeDashOffset needs defaultStrokeDashes first".into()));
    }
    let now = stroke_option(h, a, 0.0, |s| s.dash.as_ref().map_or(0.0, |d| d.offset))?;
    set(h, now == v, json!({"dashOffset": v}))
}

fn scale_factor(h: &Host, a: &[Data]) -> Answer {
    this_doc(h, a)?;
    Ok(num(1.0))
}

/// The `Document` members of this module.
pub(super) fn document_members(l: &Link) -> Vec<(String, HostMember)> {
    let members: Vec<(&str, HostMember)> = vec![
        ("defaultFillColor", property(l, |h, a| default_color(h, a, true), |h, a| set_default_color(h, a, true))),
        ("defaultFilled", property(l, |h, a| default_painted(h, a, true), |h, a| set_default_painted(h, a, true))),
        ("defaultFillOverprint", property(l, |h, a| default_overprint(h, a, true), |h, a| set_default_overprint(h, a, true))),
        ("defaultStrokeColor", property(l, |h, a| default_color(h, a, false), |h, a| set_default_color(h, a, false))),
        ("defaultStroked", property(l, |h, a| default_painted(h, a, false), |h, a| set_default_painted(h, a, false))),
        ("defaultStrokeOverprint", property(l, |h, a| default_overprint(h, a, false), |h, a| set_default_overprint(h, a, false))),
        ("defaultStrokeWidth", property(l, default_stroke_width, set_default_stroke_width)),
        ("defaultStrokeCap", property(l, default_cap, set_default_cap)),
        ("defaultStrokeJoin", property(l, default_join, set_default_join)),
        ("defaultStrokeMiterLimit", property(l, default_miter, set_default_miter)),
        ("defaultStrokeDashes", property(l, default_dashes, set_default_dashes)),
        ("defaultStrokeDashOffset", property(l, default_dash_offset, set_default_dash_offset)),
        ("scaleFactor", get(l, scale_factor)),
    ];
    members.into_iter().map(|(k, m)| (k.to_string(), m)).collect()
}
