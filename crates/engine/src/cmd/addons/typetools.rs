//! Type Tools (`addon.type.*`), a built-in add-on: Text Tweaker gives one character attribute to
//! the selected characters letter by letter, from a start value to an end value along a shape
//! (straight, eased, a bow, a spike, or at random), while the text stays live. Values are set as
//! the Character panel sets them (`text.setRangeStyle`), in its units and ranges
//! (<https://helpx.adobe.com/illustrator/using/formatting-type.html>). Its submenu
//! (Object › Plug-ins › Type Tools) and dialog are the UI's.

#[cfg(test)]
mod tests;

use serde_json::{Value, json};
use vectorcraft_color::recolor::Rng;
use vectorcraft_doc::{NodeId, TextRun};
use vectorcraft_text::edit;

use super::super::menucmds::whole_or;
use super::super::object::choice;
use super::super::textedit::{CharChange, text_mut, text_ref, type_ranges};
use super::super::typecmd::refresh_bounds;
use super::super::{CommandSpec, bad, cmd, has_selection, number_or, switch_or};
use crate::{EngineError, Result, Session};

/// An attribute Text Tweaker sets: its `text.setRangeStyle` param, the values it takes and its
/// default start and end.
type Attribute = (&'static str, (f64, f64), (f64, f64));

/// The attributes Text Tweaker sets.
pub const ATTRIBUTES: [Attribute; 8] = [
    ("size", (0.1, 1296.0), (12.0, 36.0)),
    ("baselineShift", (-1296.0, 1296.0), (0.0, 12.0)),
    ("rotation", (-360.0, 360.0), (0.0, 30.0)),
    ("hScale", (1.0, 10000.0), (100.0, 150.0)),
    ("vScale", (1.0, 10000.0), (100.0, 150.0)),
    ("tracking", (-1000.0, 10000.0), (0.0, 200.0)),
    ("leading", (0.1, 5000.0), (12.0, 36.0)),
    ("strokeWidth", (0.0, 1000.0), (0.0, 2.0)),
];
/// The shapes the values follow from the first character to the last.
pub const SHAPES: [&str; 5] = ["straight", "ease", "bow", "spike", "random"];
/// The most characters one run changes.
const MAX_CHARS: usize = 100_000;

pub fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        "addon.type.textTweaker",
        "Text Tweaker",
        ["Object", "Plug-ins", "Type Tools"],
        None,
        "{attribute?: \"size\" (pt 0.1..1296) | \"baselineShift\" (pt −1296..1296) | \"rotation\" (° −360..360) | \"hScale\" (% 1..10000) | \"vScale\" (% 1..10000) | \"tracking\" (1/1000 em −1000..10000) | \"leading\" (pt 0.1..5000) | \"strokeWidth\" (pt 0..1000) (\"size\"), from?: the first character's value, to?: the last character's value (each in the attribute's units and range; defaults: size 12 → 36, baselineShift 0 → 12, rotation 0 → 30, hScale and vScale 100 → 150, tracking 0 → 200, leading 12 → 36, strokeWidth 0 → 2), shape?: \"straight\" (an even ramp) | \"ease\" (slow at both ends) | \"bow\" (from at both ends, rising smoothly to `to` in the middle) | \"spike\" (from at both ends, straight up to `to` in the middle) | \"random\" (each character anywhere from `from` to `to`) (\"straight\"), seed?: whole number (0; random: the same seed and text give the same values), skipSpaces?: true (spaces and line breaks keep their values and don't count), id?, start?: byte, end?: byte, ids?: [id…] (instead of id)} give the characters one character attribute letter by letter, from `from` to `to` along the shape, as the Character panel sets it, as one undo step; the text stays live and editable. The range of `id`, else the type objects in `ids`, else the text the Type tool has selected (at a caret: all its text), else every selected type object; each object gets its own ramp. Stroke weight only shows on characters with a stroke → {count (characters changed), ids}",
        has_selection,
        text_tweaker
    )]
}

/// Where along `shape` (0 at `from`, 1 at `to`) character `i` of `n` lands; `random` for the
/// random shape.
fn amount(shape: &str, i: usize, n: usize, random: f64) -> f64 {
    let t = if n > 1 { i as f64 / (n - 1) as f64 } else { 0.0 };
    match shape {
        "ease" => t * t * (3.0 - 2.0 * t),
        "bow" => (std::f64::consts::PI * t).sin(),
        "spike" => 1.0 - (2.0 * t - 1.0).abs(),
        "random" => random,
        _ => t,
    }
}

/// A random number from 0 to 1.
fn unit(rng: &mut Rng) -> f64 {
    (rng.next_u64() >> 11) as f64 / (1u64 << 53) as f64
}

fn text_tweaker(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "addon.type.textTweaker";
    let attribute = choice(p, C, "attribute", &ATTRIBUTES.map(|a| a.0))?.unwrap_or("size");
    let Some(&(key, (lo, hi), (from0, to0))) = ATTRIBUTES.iter().find(|a| a.0 == attribute) else {
        return Err(bad(C, format!("unknown attribute `{attribute}`")));
    };
    let mut ends = [0.0; 2];
    for (end, (k, default)) in ends.iter_mut().zip([("from", from0), ("to", to0)]) {
        let v = number_or(p, k, default, C)?;
        if !(lo..=hi).contains(&v) {
            return Err(bad(C, format!("`{k}` must be from {lo} to {hi} for {key}")));
        }
        *end = v;
    }
    let [from, to] = ends;
    let shape = choice(p, C, "shape", &SHAPES)?.unwrap_or("straight");
    let mut rng = Rng::new(whole_or(p, "seed", 0, C)?);
    let skip = switch_or(p, "skipSpaces", true, C)?;
    let setup = s.doc()?.doc.setup.clone();
    // A range given as anything but whole bytes is refused, not read as all of the text.
    for k in ["start", "end"] {
        if p.get(k).is_some_and(|v| !v.is_null() && v.as_u64().is_none()) {
            return Err(bad(C, format!("`{k}` must be a whole number of bytes")));
        }
    }
    // At a caret, all of the object's text.
    let ranges = type_ranges(s, p, C, |text, _| Some((0, text.len())))?;
    // Each object's changed characters: (byte, its change), worked out before the edit.
    let mut plans: Vec<(NodeId, Vec<(usize, CharChange)>)> = vec![];
    let (mut seen, mut count) = (0, 0);
    for (id, a, b, _) in ranges {
        let t = text_ref(s, id)?;
        let text = t.plain_text();
        let chars: Vec<usize> =
            text.char_indices().filter(|&(i, c)| (a..b).contains(&i) && !(skip && (c.is_whitespace() || c.is_control()))).map(|(i, _)| i).collect();
        seen += chars.len();
        if seen > MAX_CHARS {
            return Err(bad(C, format!("too many characters: at most {MAX_CHARS}")));
        }
        // Each run's end byte, to find the style a character has now.
        let ends: Vec<usize> = t
            .runs
            .iter()
            .scan(0, |off, r| {
                *off += r.text.len();
                Some(*off)
            })
            .collect();
        let n = chars.len();
        let mut changes = Vec::with_capacity(n);
        let mut run = 0;
        for (i, at) in chars.into_iter().enumerate() {
            let random = if shape == "random" { unit(&mut rng) } else { 0.0 };
            let value = from + (to - from) * amount(shape, i, n, random);
            let mut change = CharChange::parse(&json!({ key: value }), C, &setup)?;
            change.localize(&t, C)?;
            while ends.get(run).is_some_and(|&e| e <= at) {
                run += 1;
            }
            // A character that already has the value is left out, so a run that changes nothing
            // makes no step.
            if let Some(r) = t.runs.get(run) {
                let mut style = r.style.clone();
                change.apply(&mut style);
                if style == r.style {
                    continue;
                }
            }
            changes.push((at, change));
        }
        count += changes.len();
        if !changes.is_empty() {
            plans.push((id, changes));
        }
    }
    if plans.is_empty() {
        return Ok(json!({ "count": 0, "ids": [] }));
    }
    let ids: Vec<u64> = plans.iter().map(|(id, _)| id.0).collect();
    s.edit("Text Tweaker", |d, _| {
        for (id, changes) in &plans {
            let t = text_mut(d, *id).ok_or(EngineError::NoNode(*id))?;
            t.runs = tweak(std::mem::take(&mut t.runs), changes);
            refresh_bounds(t);
        }
        Ok(())
    })?;
    Ok(json!({ "count": count, "ids": ids }))
}

/// `runs` with each character at a byte of `changes` (in order) changed: its run split into
/// characters, the changes applied, and runs that came out alike merged again.
fn tweak(runs: Vec<TextRun>, changes: &[(usize, CharChange)]) -> Vec<TextRun> {
    let mut out = Vec::with_capacity(runs.len() + changes.len() * 2);
    let mut next = changes.iter().peekable();
    let mut off = 0;
    for r in runs {
        let end = off + r.text.len();
        if r.inline.is_some() || next.peek().is_none_or(|(at, _)| *at >= end) {
            // An inline graphic is one character: its style changes whole.
            if let Some((_, change)) = next.next_if(|(at, _)| (off..end).contains(at)) {
                let mut r = r;
                change.apply(&mut r.style);
                out.push(r);
            } else {
                out.push(r);
            }
            off = end;
            continue;
        }
        for (i, c) in r.text.char_indices() {
            let mut piece = TextRun::new(c.to_string(), r.style.clone());
            if let Some((_, change)) = next.next_if(|(at, _)| *at == off + i) {
                change.apply(&mut piece.style);
            }
            out.push(piece);
        }
        off = end;
    }
    edit::normalize(&mut out);
    out
}
