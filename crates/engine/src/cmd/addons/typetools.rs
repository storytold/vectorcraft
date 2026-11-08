//! Type Tools (`addon.type.*`), a built-in add-on: Text Tweaker gives one character attribute to
//! the selected characters letter by letter, from a start value to an end value along a shape
//! (straight, eased, a bow, a spike, or at random), while the text stays live. Values are set as
//! the Character panel sets them (`text.setRangeStyle`), in its units and ranges
//! (<https://helpx.adobe.com/illustrator/using/formatting-type.html>). Release Type on a Path
//! splits type on a path into point type holding the same text and the plain path it was set on
//! (#5532). The reference app has no command for it, so it goes beyond parity and changes no
//! parity score. Its submenu (Object › Plug-ins › Type Tools) and dialogs are the UI's.

#[cfg(test)]
mod tests;

use serde_json::{Value, json};
use vectorcraft_color::Paint;
use vectorcraft_color::recolor::Rng;
use vectorcraft_doc::{Appearance, Document, Node, NodeId, NodeKind, TextKind, TextObject, TextRun};
use vectorcraft_geom::{Affine, ArcPath, Point};
use vectorcraft_text::edit;

use super::super::menucmds::whole_or;
use super::super::object::choice;
use super::super::textedit::{CharChange, text_mut, text_ref, type_ranges};
use super::super::typecmd::refresh_bounds;
use super::super::typemenu::text_ids;
use super::super::{CommandSpec, bad, cmd, has_selection, number_or, switch_or, targets};
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
    vec![
        cmd!(
            "addon.type.textTweaker",
            "Text Tweaker",
            ["Object", "Plug-ins", "Type Tools"],
            None,
            "{attribute?: \"size\" (pt 0.1..1296) | \"baselineShift\" (pt −1296..1296) | \"rotation\" (° −360..360) | \"hScale\" (% 1..10000) | \"vScale\" (% 1..10000) | \"tracking\" (1/1000 em −1000..10000) | \"leading\" (pt 0.1..5000) | \"strokeWidth\" (pt 0..1000) (\"size\"), from?: the first character's value, to?: the last character's value (each in the attribute's units and range; defaults: size 12 → 36, baselineShift 0 → 12, rotation 0 → 30, hScale and vScale 100 → 150, tracking 0 → 200, leading 12 → 36, strokeWidth 0 → 2), shape?: \"straight\" (an even ramp) | \"ease\" (slow at both ends) | \"bow\" (from at both ends, rising smoothly to `to` in the middle) | \"spike\" (from at both ends, straight up to `to` in the middle) | \"random\" (each character anywhere from `from` to `to`) (\"straight\"), seed?: whole number (0; random: the same seed and text give the same values), skipSpaces?: true (spaces and line breaks keep their values and don't count), id?, start?: byte, end?: byte (range endpoints inside a character round down to its start), ids?: [id…] (instead of id)} give the characters one character attribute letter by letter, from `from` to `to` along the shape, as the Character panel sets it, as one undo step; the text stays live and editable. The range of `id`, else the type objects in `ids`, else the text the Type tool has selected (at a caret: all its text), else every selected type object; each object gets its own ramp. Stroke weight only shows on characters with a stroke → {count (characters changed), ids}",
            has_selection,
            text_tweaker
        ),
        cmd!(
            "addon.type.releasePath",
            "Release Type on a Path…",
            ["Object", "Plug-ins", "Type Tools"],
            None,
            "{mode?: \"both\" (point type and the path) | \"path\" (the path only) | \"text\" (point type only) (\"both\"), keepOriginal?: false (true: the type on a path stays and the released objects go just above it), ids?: [id…], id? (instead of ids; neither: the selection)} split each type on a path among the objects (inside groups too) into point type and the path it was set on. The point type holds the same text with the same character and paragraph attributes, on one straight line whose baseline starts where the first character sat on the path; Type on a Path Options (effect, align to path, spacing, flip, brackets) don't apply to point type and are dropped. The path keeps its exact shape (open or closed, anchors and handles, the object's transform) and comes out with no fill and no stroke, in the type's layer and place in the stacking order. Other objects are left alone, and with no type on a path among them nothing changes and no undo step is made. One undo step; the released objects are selected → {ids (released, bottom to top), paths, texts}",
            has_selection,
            release_path
        ),
    ]
}

/// Release Type on a Path's command (the id in [`specs`]).
pub const RELEASE: &str = "addon.type.releasePath";
/// What Release Type on a Path keeps: both, the path only, or the text only.
pub const RELEASE_MODES: [&str; 3] = ["both", "path", "text"];

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
    // At a caret, all of the object's text.
    let ranges = type_ranges(s, p, C, |text, _| Some((0, text.len())))?;
    // Each object's changed characters: (byte, its change), worked out before the edit.
    let mut plans: Vec<(NodeId, Vec<(usize, CharChange)>)> = vec![];
    let (mut seen, mut count) = (0, 0);
    for (id, a, b, _) in ranges {
        let t = text_ref(s, id)?;
        let text = t.plain_text();
        let (a, b) = (edit::floor_char(&text, a), edit::floor_char(&text, b));
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

/// Type on a path among the objects `p` names (or the selection), inside groups too, in stacking
/// order. An id that names no object is an error.
fn path_types(s: &Session, p: &Value) -> Result<Vec<NodeId>> {
    let ids = targets(s, p)?;
    let d = &s.doc()?.doc;
    if let Some(id) = ids.iter().find(|id| d.node(**id).is_none()) {
        return Err(EngineError::NoNode(*id));
    }
    let on_path = |id: &NodeId| matches!(d.node(*id).map(|n| &n.kind), Some(NodeKind::Text(t)) if matches!(t.kind, TextKind::OnPath { .. }));
    Ok(text_ids(d, &ids).into_iter().filter(on_path).collect())
}

/// Where the first character laid out in `lay` starts its baseline: the pen position of the first
/// glyph in text order (a list marker, which has no characters, before the character it leads).
/// None without a glyph.
fn first_pen(lay: &vectorcraft_text::TextLayout) -> Option<Point> {
    lay.glyphs.iter().filter(|g| g.origin.x.is_finite() && g.origin.y.is_finite()).min_by_key(|g| (g.byte, g.len != 0)).map(|g| g.origin)
}

/// `t`, type on a path, as point type on one straight line: the same runs and character and
/// paragraph attributes, its baseline starting where its first character sat on the path (in the
/// object's own text space, so its transform stays). Type on a Path Options are dropped. None for
/// other type, or when it can't be placed.
fn released_text(t: &TextObject) -> Option<TextObject> {
    let TextKind::OnPath { path, .. } = &t.kind else { return None };
    let db = vectorcraft_text::FontDb::global();
    // Where the first character sat. Text that didn't fit on the path has no glyph there: it
    // starts at the start bracket.
    let at = first_pen(&vectorcraft_text::layout(db, t)).or_else(|| {
        let ap = ArcPath::new(path);
        let (from, _) = t.kind.path_span()?;
        let (p, _) = ap.at(from * ap.len());
        (p.x.is_finite() && p.y.is_finite()).then_some(p)
    })?;
    let mut out = t.clone();
    out.kind = TextKind::Point;
    out.path_effect = Default::default();
    out.path_align = Default::default();
    out.path_spacing = 0.0;
    // Point type is in no thread: its first paragraph starts in it.
    out.first_para_continues = false;
    // Where the point type puts its own first character; the origin moves so it lands at `at`,
    // whatever the paragraph's alignment and direction.
    let own = first_pen(&vectorcraft_text::layout(db, &out)).unwrap_or(Point::ZERO);
    let shift = at - own;
    if !(shift.x.is_finite() && shift.y.is_finite()) {
        return None;
    }
    out.xf *= Affine::translate(shift);
    refresh_bounds(&mut out);
    Some(out)
}

/// The path `t` is set on, as a plain path object `id` in document space, with no fill and no
/// stroke: type paths carry no paint of their own. None for other type.
fn released_path(id: NodeId, t: &TextObject) -> Option<Node> {
    Some(Node::path(id, t.type_path()?, Appearance::basic(Paint::None, Paint::None, 1.0)))
}

fn release_path(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = RELEASE;
    let mode = choice(p, C, "mode", &RELEASE_MODES)?.unwrap_or("both");
    let keep = switch_or(p, "keepOriginal", false, C)?;
    let want_path = mode != "text";
    let ids = path_types(s, p)?;
    if ids.is_empty() {
        return Ok(json!({ "ids": [], "paths": [], "texts": [] }));
    }
    // The point type, worked out before the edit: one object that can't be placed fails the run.
    let mut plans: Vec<(NodeId, Option<TextObject>)> = Vec::with_capacity(ids.len());
    {
        let d = &s.doc()?.doc;
        for id in ids {
            let Some(NodeKind::Text(t)) = d.node(id).map(|n| &n.kind) else { continue };
            let text = match mode {
                "path" => None,
                _ => Some(released_text(t).ok_or_else(|| bad(C, format!("object {} can't be set as point type", id.0)))?),
            };
            plans.push((id, text));
        }
    }
    let (paths, texts) = s.edit("Release Type on a Path", |d, sel| {
        let (mut paths, mut texts) = (vec![], vec![]);
        for (id, text) in plans {
            let (path, text) = release_one(d, id, text, want_path, keep)?;
            paths.extend(path);
            texts.extend(text);
        }
        sel.set(paths.iter().chain(&texts).copied());
        Ok((paths, texts))
    })?;
    // Bottom to top.
    let d = &s.doc()?.doc;
    let mut all: Vec<(Vec<usize>, NodeId)> = paths.iter().chain(&texts).map(|id| (d.index_path(*id).unwrap_or_default(), *id)).collect();
    all.sort();
    let nums = |v: &[NodeId]| v.iter().map(|i| i.0).collect::<Vec<_>>();
    let all: Vec<NodeId> = all.into_iter().map(|(_, id)| id).collect();
    Ok(json!({ "ids": nums(&all), "paths": nums(&paths), "texts": nums(&texts) }))
}

/// Release type on a path `id` in `d`: its path (`want_path`) where the type was, or just above it
/// when the original stays (`keep`), then the point type `text` (None: no text) above that.
/// Without `keep` the original goes, and released text takes its place as the same object, so a
/// frame of threaded text leaves its thread with the text it holds, as Convert To Point Type's
/// does. The released (path, text).
fn release_one(d: &mut Document, id: NodeId, text: Option<TextObject>, want_path: bool, keep: bool) -> Result<(Option<NodeId>, Option<NodeId>)> {
    let (parent, idx, _) = d.position(id).ok_or(EngineError::NoNode(id))?;
    let original = d.node(id).cloned().ok_or(EngineError::NoNode(id))?;
    let NodeKind::Text(t) = &original.kind else { return Ok((None, None)) };
    let path =
        if want_path { Some(released_path(d.alloc_id(), t).ok_or_else(|| bad(RELEASE, format!("object {} has no path", id.0)))?) } else { None };
    if !keep && text.is_none() {
        d.remove(id)?;
    }
    // Where the next released object goes: above the original when it stays, else in its place.
    let mut at = if keep { idx + 1 } else { idx };
    let path = match path {
        Some(n) => {
            at += 1;
            Some(d.insert(parent, at - 1, n)?)
        }
        None => None,
    };
    let text = match text {
        None => None,
        Some(text) if keep => {
            let mut n = original.clone();
            n.id = d.alloc_id();
            n.kind = NodeKind::Text(Box::new(text));
            Some(d.insert(parent, at, n)?)
        }
        Some(text) => {
            // The original, now just above the path.
            d.node_mut(id).ok_or(EngineError::NoNode(id))?.kind = NodeKind::Text(Box::new(text));
            Some(id)
        }
    };
    Ok((path, text))
}
