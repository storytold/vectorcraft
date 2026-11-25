//! What a filter whose manifest lists `writes` may change about the type, images and symbol
//! instances it was given: each one's name and transform; type's runs, with their character
//! attributes, and its paragraph styles; an image's opacity; and the symbol an instance shows.
//! Every other field stays read-only, and every value is checked before anything changes.

use std::collections::HashSet;

use serde::de::DeserializeOwned;
use serde_json::{Map, Value};
use vectorcraft_doc::text::{AttrRangeError, INLINE_CHAR};
use vectorcraft_doc::{CharStyle, Document, JustificationOptions, KinsokuSet, Node, NodeKind, ParaStyle, TextObject, TextRun};
use vectorcraft_geom::Affine;

use super::{
    MAX_COORD, MAX_INPUT_RUNS, MAX_INPUT_TEXT, MAX_INPUT_TEXT_TOTAL, admit_paint, decode_name, decode_paint, first_difference, out_err,
    read_only_changed,
};
use crate::{Error, InputKind, Result};

/// The longest font family name a filter may set, in characters.
const MAX_FAMILY: usize = 256;
/// The smallest determinant a transform a filter sets may have: a smaller one flattens the object.
const MIN_DETERMINANT: f64 = 1e-12;
/// The most pairs of an old and a new paragraph, past those both texts start and end with, that a
/// change of the paragraph count compares by their text ([`moved_para_styles`]).
const MAX_MATCHED_PAIRS: usize = 1 << 18;

/// The fields of an object of `kind` a filter that lists the kind in `writes` may change.
fn writable(kind: InputKind) -> &'static [&'static str] {
    match kind {
        InputKind::Type => &["name", "transform", "text"],
        InputKind::Image => &["name", "transform", "opacity"],
        InputKind::SymbolInstance => &["name", "transform", "symbolId"],
    }
}

/// The changes entry `o` of a filter's output makes to object `given` of `kind` (as the input gave
/// it), `None` when it changes nothing. Fields it leaves out keep their values; a field it gives
/// that isn't [`writable`] must be as given. Changed type text counts toward `text`, the bytes of
/// text written over the whole output. The changes are keyed `name`, `transform`, `opacity`,
/// `symbolId`, `runs` and `paragraphs`; their values are checked when they are applied ([`apply`]).
pub(super) fn changes(o: &Map<String, Value>, given: &Value, kind: InputKind, at: &str, text: &mut usize) -> Result<Option<Map<String, Value>>> {
    let mut keys: Vec<&String> = o.keys().filter(|k| *k != "id").collect();
    keys.sort();
    let mut out = Map::new();
    for k in keys {
        let v = &o[k];
        let was = given.get(k);
        if !writable(kind).contains(&k.as_str()) {
            if let Some(field) = was.map_or_else(|| Some(k.clone()), |w| first_difference(v, w, k)) {
                return Err(read_only_changed(at, &field));
            }
            continue;
        }
        match (k.as_str(), was) {
            ("text", Some(was)) => text_changes(k, v, was, given.get("truncated") == Some(&Value::Bool(true)), at, text, &mut out)?,
            ("name", None) if v.is_null() => {}
            (_, Some(was)) if first_difference(v, was, "").is_none() => {}
            _ => {
                out.insert(k.clone(), v.clone());
            }
        }
    }
    Ok((!out.is_empty()).then_some(out))
}

/// The `runs` and `paragraphs` of type text `v`, the output's field `key`, that differ from `was`
/// (the text the input gave), into `out`; its other fields must be as given, and type given as
/// `truncated` keeps its text. A changed list is held to [`MAX_INPUT_RUNS`] entries, and changed
/// runs to [`MAX_INPUT_TEXT`] bytes of text, which count toward `total`, held to
/// [`MAX_INPUT_TEXT_TOTAL`], and to [`MAX_INPUT_RUNS`] paragraphs. With changed runs, `paragraphs`
/// given as they were go into `out` too, to be checked against the new paragraphs.
fn text_changes(key: &str, v: &Value, was: &Value, truncated: bool, at: &str, total: &mut usize, out: &mut Map<String, Value>) -> Result<()> {
    let t = v.as_object().ok_or_else(|| out_err(at, format!("`{key}` must be an object")))?;
    let mut keys: Vec<&String> = t.keys().collect();
    keys.sort();
    for k in keys {
        let field = format!("{key}.{k}");
        let given = was.get(k);
        let differs = given.is_none_or(|g| first_difference(&t[k], g, "").is_some());
        if !matches!(k.as_str(), "runs" | "paragraphs") {
            if differs {
                let shown = given.and_then(|g| first_difference(&t[k], g, &field)).unwrap_or_else(|| field.clone());
                return Err(read_only_changed(at, &shown));
            }
            continue;
        }
        if !differs {
            continue;
        }
        if truncated {
            return Err(out_err(&format!("{at}.{field}"), "the text of type given as `truncated` can't be changed"));
        }
        let list = t[k].as_array().filter(|l| l.len() <= MAX_INPUT_RUNS);
        let list = list.ok_or_else(|| out_err(&format!("{at}.{field}"), format!("must be a list of at most {MAX_INPUT_RUNS} entries")))?;
        if k == "runs" {
            if list.is_empty() {
                return Err(out_err(&format!("{at}.{field}"), "type needs at least one run"));
            }
            let bytes = list.iter().fold(0usize, |n, r| n.saturating_add(r.get("text").and_then(Value::as_str).map_or(0, str::len)));
            if bytes > MAX_INPUT_TEXT {
                return Err(out_err(&format!("{at}.{field}"), format!("{bytes} bytes of text (limit {MAX_INPUT_TEXT})")));
            }
            // Each paragraph gets a paragraph style; the text holds at most as many paragraphs as runs.
            let breaks = |r: &Value| r.get("text").and_then(Value::as_str).map_or(0, |s| s.bytes().filter(|&b| b == b'\n').count());
            let paragraphs = list.iter().fold(1usize, |n, r| n.saturating_add(breaks(r)));
            if paragraphs > MAX_INPUT_RUNS {
                return Err(out_err(&format!("{at}.{field}"), format!("{paragraphs} paragraphs (limit {MAX_INPUT_RUNS})")));
            }
            *total = total.saturating_add(bytes);
            if *total > MAX_INPUT_TEXT_TOTAL {
                return Err(out_err(&format!("{at}.{field}"), format!("more than {MAX_INPUT_TEXT_TOTAL} bytes of text over the output")));
            }
        }
        out.insert(k.clone(), t[k].clone());
    }
    // Paragraph styles given as they were still need one per paragraph of the new runs.
    if out.contains_key("runs")
        && !out.contains_key("paragraphs")
        && let Some(p) = t.get("paragraphs")
    {
        out.insert("paragraphs".into(), p.clone());
    }
    Ok(())
}

/// Every attribute name `T` reads, including those its JSON leaves out at their defaults: the field
/// names serde's derive hands `deserialize_struct`.
fn attr_names<T: DeserializeOwned>() -> &'static [&'static str] {
    use serde::de::{Error as _, Visitor, value::Error};

    struct Names<'a>(&'a mut &'static [&'static str]);
    impl<'de> serde::Deserializer<'de> for Names<'_> {
        type Error = Error;
        fn deserialize_any<V: Visitor<'de>>(self, _: V) -> std::result::Result<V::Value, Error> {
            Err(Error::custom("not a struct"))
        }
        fn deserialize_struct<V: Visitor<'de>>(self, _: &'static str, fields: &'static [&'static str], _: V) -> std::result::Result<V::Value, Error> {
            *self.0 = fields;
            Err(Error::custom("field names only"))
        }
        serde::forward_to_deserialize_any! {
            bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string bytes byte_buf option unit
            unit_struct newtype_struct seq tuple tuple_struct map enum identifier ignored_any
        }
    }

    let mut fields: &'static [&'static str] = &[];
    // Always an error: the probe only records the names.
    let _ = T::deserialize(Names(&mut fields));
    fields
}

/// `v`, the field `key` of the entry at `parent`, as a `T` whose attributes are all ones `T` reads:
/// a value that isn't an object is an error at `parent`, and an unknown key, as an ill-typed value,
/// an error at `parent.key`.
fn strict<T: DeserializeOwned>(v: &Value, parent: &str, key: &str) -> Result<T> {
    let o = v.as_object().ok_or_else(|| out_err(parent, format!("`{key}` must be an object")))?;
    let at = format!("{parent}.{key}");
    let names = attr_names::<T>();
    if let Some(k) = o.keys().find(|k| !names.contains(&k.as_str())) {
        let k = crate::permissions::display_text(k, crate::permissions::MAX_SHOWN_FIELD);
        return Err(out_err(&at, format!("unknown key '{k}'")));
    }
    serde_json::from_value(v.clone()).map_err(|e| out_err(&at, e))
}

/// The field `key` of `v`, the style at `at`, holds only attributes `T` reads: an object, or each
/// object of a list. An unknown key is an error at `at.key` (or `at.key[i]`); other values are left
/// to the style's own checks.
fn known_keys<T: DeserializeOwned>(v: &Value, at: &str, key: &str) -> Result<()> {
    let names = attr_names::<T>();
    let check = |o: &Map<String, Value>, at: &str| match o.keys().find(|k| !names.contains(&k.as_str())) {
        Some(k) => {
            let k = crate::permissions::display_text(k, crate::permissions::MAX_SHOWN_FIELD);
            Err(out_err(at, format!("unknown key '{k}'")))
        }
        None => Ok(()),
    };
    match v.get(key) {
        Some(Value::Object(o)) => check(o, &format!("{at}.{key}")),
        Some(Value::Array(list)) => list.iter().enumerate().try_for_each(|(i, e)| match e {
            Value::Object(o) => check(o, &format!("{at}.{key}[{i}]")),
            _ => Ok(()),
        }),
        _ => Ok(()),
    }
}

/// A transform from a filter's output: six finite numbers, each within ±[`MAX_COORD`], that don't
/// flatten the object.
fn decode_transform(v: &Value, at: &str) -> Result<Affine> {
    let c: Option<Vec<f64>> = v.as_array().and_then(|a| a.iter().map(Value::as_f64).collect());
    let c: [f64; 6] = c.and_then(|c| c.try_into().ok()).ok_or_else(|| out_err(at, "must be a list of 6 numbers"))?;
    if !c.iter().all(|x| x.is_finite() && x.abs() <= MAX_COORD) {
        return Err(out_err(at, format!("a number is out of range (limit ±{MAX_COORD})")));
    }
    let xf = Affine::new(c);
    if xf.determinant().abs() < MIN_DETERMINANT {
        return Err(out_err(at, "flattens the object to a line or a point"));
    }
    Ok(xf)
}

/// The transform object `n` is given with: type's and an instance's own, an image's (for a
/// distorted image, the placement nearest its corners at its middle).
fn given_transform(n: &Node) -> Option<Affine> {
    match &n.kind {
        NodeKind::Text(t) => Some(t.xf),
        NodeKind::Image(im) => Some(vectorcraft_doc::image_quad::nearest_affine(n).unwrap_or(im.xf)),
        NodeKind::SymbolInstance { xf, .. } => Some(*xf),
        _ => None,
    }
}

/// Applies `changes` ([`changes`]) to `n`, an input object of `doc`; `at` names it in errors. Each
/// value is checked first, and an error leaves `n` to be dropped.
pub(super) fn apply(doc: &Document, n: &mut Node, changes: &Map<String, Value>, at: &str) -> Result<()> {
    if let Some(v) = changes.get("transform") {
        let at = format!("{at}.transform");
        let new = decode_transform(v, &at)?;
        let old = given_transform(n).filter(|xf| xf.determinant().abs() >= MIN_DETERMINANT);
        let old = old.ok_or_else(|| out_err(&at, "an object flattened to a line or a point can't be placed again"))?;
        // Everything the object carries moves with it: a distorted image's corners, its gradients.
        n.transform(new * old.inverse(), false);
        let distorted = n.is_distorted_image();
        match &mut n.kind {
            NodeKind::Text(t) => t.xf = new,
            NodeKind::SymbolInstance { xf, .. } => *xf = new,
            NodeKind::Image(im) if !distorted => im.xf = new,
            _ => {}
        }
    }
    if let Some(v) = changes.get("opacity") {
        let op = v.as_f64().filter(|o| (0.0..=1.0).contains(o));
        n.opacity = op.ok_or_else(|| out_err(&format!("{at}.opacity"), "must be a number from 0 to 1"))? as f32;
    }
    if let Some(v) = changes.get("symbolId") {
        let at = format!("{at}.symbolId");
        let name = v.as_str().ok_or_else(|| out_err(&at, "must be the name of a symbol"))?;
        if !doc.symbols.iter().any(|s| s.name == name) {
            let shown = crate::permissions::display_text(name, crate::permissions::MAX_SHOWN_FIELD);
            return Err(out_err(&at, format!("'{shown}' is not a symbol in this document")));
        }
        let other = matches!(&n.kind, NodeKind::SymbolInstance { symbol, .. } if symbol != name);
        if let NodeKind::SymbolInstance { symbol, .. } = &mut n.kind {
            *symbol = name.to_string();
        }
        // An edited part's path names an object of the old symbol's art, as Replace Symbol finds.
        if other && !n.extras().symbol_overrides.is_empty() {
            n.extras_mut().symbol_overrides.clear();
        }
    }
    if changes.contains_key("runs") || changes.contains_key("paragraphs") {
        let story = doc.story_of(n.id).len();
        let NodeKind::Text(t) = &mut n.kind else { return Err(read_only_changed(at, "text")) };
        set_text(doc, t, story > 1, changes.get("runs"), changes.get("paragraphs"), at)?;
    }
    if let Some(v) = changes.get("name") {
        n.name = Some(decode_name(v, at)?).filter(|s| !s.trim().is_empty());
    }
    Ok(())
}

/// Gives type `t` the runs `runs` and paragraph styles `paragraphs` a filter returned (`threaded`:
/// it is a frame of a story that flows through others). Each run has `text`, `style` and, for an
/// inline graphic, `inline`; each attribute is one the type reads. Inline graphics stay as they
/// were, in the same order, and the characters of threaded type or type with ruby stay as they
/// were. Every number of every run's style is in the range the character style commands keep it
/// to ([`CharStyle::check_ranges`]), and a run whose style isn't one of the type's own has a font
/// family, and its paints are checked as an object's are. Paragraph styles are one per paragraph
/// ([`para_style`]). With runs alone, each paragraph keeps its style when the number of paragraphs stays
/// the same, and otherwise the styles follow the paragraphs' text ([`moved_para_styles`]).
fn set_text(doc: &Document, t: &mut TextObject, threaded: bool, runs: Option<&Value>, paragraphs: Option<&Value>, at: &str) -> Result<()> {
    if let Some(runs) = runs {
        let at = format!("{at}.text.runs");
        let list = runs.as_array().map(Vec::as_slice).unwrap_or_default();
        // The type's own styles, as JSON text, to look each run's style up once.
        let own: HashSet<String> = t.runs.iter().filter_map(|r| serde_json::to_string(&r.style).ok()).collect();
        let mut new = Vec::with_capacity(list.len());
        for (i, v) in list.iter().enumerate() {
            let at = format!("{at}[{i}]");
            let o = v.as_object().ok_or_else(|| out_err(&at, "a run must be {\"text\", \"style\", \"inline\"?}"))?;
            if let Some(k) = o.keys().find(|k| !matches!(k.as_str(), "text" | "style" | "inline")) {
                let k = crate::permissions::display_text(k, crate::permissions::MAX_SHOWN_FIELD);
                return Err(out_err(&at, format!("unknown key '{k}'")));
            }
            let key = "text";
            let text = o.get(key).and_then(Value::as_str).ok_or_else(|| out_err(&at, format!("`{key}` must be a string")))?;
            let style_v = o.get("style").unwrap_or(&Value::Null);
            let mut style: CharStyle = strict(style_v, &at, "style")?;
            known_keys::<vectorcraft_doc::Dash>(style_v, &format!("{at}.style"), "stroke_dash")?;
            let inline = match o.get("inline") {
                None | Some(Value::Null) => None,
                Some(v) => Some(serde_json::from_value(v.clone()).map_err(|e| out_err(&format!("{at}.inline"), e))?),
            };
            if inline.is_none() {
                if text.contains(INLINE_CHAR) {
                    return Err(out_err(&at, "`text` holds the character of an inline graphic"));
                }
                // Every number goes to the layout, so it is checked even in a style the type has.
                style.check_ranges(|_| true).map_err(|e| range_err(&format!("{at}.style"), &e))?;
                for key in ["fill", "stroke"] {
                    if let Some(p) = style_v.get(key) {
                        check_paint_keys(p, &format!("{at}.style.{key}"))?;
                    }
                }
                if !serde_json::to_string(&style).is_ok_and(|s| own.contains(&s)) {
                    check_style(doc, &mut style, t.runs.get(i).map(|r| &r.style), &format!("{at}.style"))?;
                }
            }
            new.push(TextRun { text: text.to_string(), style, inline });
        }
        // Inline graphics are compared as saved (their measured bounds aren't), and keep those.
        let saved = |r: &&TextRun| serde_json::to_value(r).ok();
        let old_inline: Vec<&TextRun> = t.runs.iter().filter(|r| r.inline.is_some()).collect();
        let new_inline: Vec<&TextRun> = new.iter().filter(|r| r.inline.is_some()).collect();
        if old_inline.len() != new_inline.len() || old_inline.iter().zip(&new_inline).any(|(a, b)| saved(a) != saved(b)) {
            return Err(out_err(&at, "inline graphics can't be added, removed, reordered or changed"));
        }
        let mut kept = old_inline.into_iter().cloned().collect::<Vec<_>>().into_iter();
        for r in new.iter_mut().filter(|r| r.inline.is_some()) {
            if let Some(old) = kept.next() {
                *r = old;
            }
        }
        let plain: String = new.iter().map(|r| r.text.as_str()).collect();
        let old = t.plain_text();
        let mut moved = None;
        if plain != old {
            if threaded {
                return Err(out_err(&at, "the characters of threaded type can't be changed"));
            }
            if !t.ruby.is_empty() {
                return Err(out_err(&at, "the characters of type with ruby can't be changed"));
            }
            let breaks = |s: &str| s.bytes().filter(|&b| b == b'\n').count();
            if paragraphs.is_none() && breaks(&plain) != breaks(&old) {
                // Paragraphs added or joined: the styles follow the paragraphs' text. With as many
                // paragraphs as before, each keeps its style.
                moved = Some(moved_para_styles(&t.paragraph_styles(), &old, &plain));
            }
        }
        t.runs = new;
        if let Some(styles) = moved {
            t.set_paragraph_styles(styles);
        }
    }
    match paragraphs {
        Some(v) => {
            let text = format!("{at}.text");
            let list = v.as_array().map(Vec::as_slice).unwrap_or_default();
            let count = t.paragraph_count();
            if list.len() != count {
                return Err(out_err(&format!("{text}.paragraphs"), format!("expected {count} paragraph styles, one per paragraph")));
            }
            let styles = list.iter().enumerate().map(|(i, p)| para_style(doc, p, &text, &format!("paragraphs[{i}]"))).collect::<Result<_>>()?;
            t.set_paragraph_styles(styles);
        }
        None => t.normalize_paras(),
    }
    // The layout is measured again before the type is drawn.
    t.cached_bounds = None;
    t.cached_baselines.clear();
    t.cached_ink = None;
    Ok(())
}

/// The paragraph styles of text `new` that replaced `old`, whose paragraphs had `styles` (one per
/// paragraph). The paragraphs, split at `\n`, are matched as a longest common subsequence of their
/// texts after the paragraphs both start and end with are set aside; when the paragraphs left in
/// the old text times those left in the new pass [`MAX_MATCHED_PAIRS`], they are paired in order
/// instead. A
/// matched paragraph keeps its style. Between two matched paragraphs, the changed ones take the
/// styles of the old paragraphs there in order, and the last of those past their count; paragraphs
/// added where there were none take the style of the paragraph before them, or of the next one at
/// the start. A style copied to a paragraph that was not there loses its list number
/// ([`vectorcraft_doc::text::ParaList::start`]).
fn moved_para_styles(styles: &[ParaStyle], old: &str, new: &str) -> Vec<ParaStyle> {
    let a: Vec<&str> = old.split('\n').collect();
    let b: Vec<&str> = new.split('\n').collect();
    let pre = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let (a_rest, b_rest) = (a.get(pre..).unwrap_or_default(), b.get(pre..).unwrap_or_default());
    let suf = a_rest.iter().rev().zip(b_rest.iter().rev()).take_while(|(x, y)| x == y).count();
    // `matched[j]`: the old paragraph that new paragraph `j` is, if any.
    let mut matched: Vec<Option<usize>> = vec![None; b.len()];
    for (j, m) in matched.iter_mut().enumerate().take(pre) {
        *m = Some(j);
    }
    for k in 0..suf {
        if let Some(m) = matched.get_mut(b.len() - 1 - k) {
            *m = Some(a.len() - 1 - k);
        }
    }
    let (am, bm) = (a_rest.get(..a_rest.len() - suf).unwrap_or_default(), b_rest.get(..b_rest.len() - suf).unwrap_or_default());
    if !am.is_empty() && !bm.is_empty() && am.len().saturating_mul(bm.len()) <= MAX_MATCHED_PAIRS {
        let mut ids = std::collections::HashMap::new();
        let mut id = |s: &str| {
            let next = ids.len();
            *ids.entry(s.to_string()).or_insert(next)
        };
        let ai: Vec<usize> = am.iter().map(|s| id(s)).collect();
        let bi: Vec<usize> = bm.iter().map(|s| id(s)).collect();
        // `len[i * w + j]`: the length of the longest common subsequence of `ai[i..]` and `bi[j..]`.
        let w = bi.len() + 1;
        let mut len = vec![0u32; (ai.len() + 1) * w];
        for i in (0..ai.len()).rev() {
            for j in (0..bi.len()).rev() {
                let at = |i: usize, j: usize| len.get(i * w + j).copied().unwrap_or(0);
                let v = if ai.get(i) == bi.get(j) { at(i + 1, j + 1) + 1 } else { at(i + 1, j).max(at(i, j + 1)) };
                if let Some(c) = len.get_mut(i * w + j) {
                    *c = v;
                }
            }
        }
        let at = |i: usize, j: usize| len.get(i * w + j).copied().unwrap_or(0);
        let (mut i, mut j) = (0, 0);
        while i < ai.len() && j < bi.len() {
            if ai.get(i) == bi.get(j) {
                if let Some(m) = matched.get_mut(pre + j) {
                    *m = Some(pre + i);
                }
                (i, j) = (i + 1, j + 1);
            } else if at(i + 1, j) >= at(i, j + 1) {
                i += 1;
            } else {
                j += 1;
            }
        }
    }
    let style = |i: usize, numbered: bool| {
        let mut s = styles.get(i).or(styles.last()).cloned().unwrap_or_default();
        if !numbered && let Some(l) = s.list.as_mut() {
            l.start = None;
        }
        s
    };
    let mut out = Vec::with_capacity(b.len());
    let (mut next_old, mut j) = (0, 0);
    while j < b.len() {
        if let Some(&Some(i)) = matched.get(j) {
            out.push(style(i, true));
            (next_old, j) = (i + 1, j + 1);
            continue;
        }
        // New paragraphs `j..end` stand where old paragraphs `next_old..old_end` were.
        let end = (j..b.len()).find(|&k| matched.get(k).is_some_and(Option::is_some)).unwrap_or(b.len());
        let old_end = matched.get(end).copied().flatten().unwrap_or(a.len());
        for g in 0..end - j {
            out.push(if next_old < old_end {
                style((next_old + g).min(old_end - 1), next_old + g < old_end)
            } else {
                style(next_old.checked_sub(1).unwrap_or(old_end), false)
            });
        }
        j = end;
    }
    out
}

/// An attribute of the style at `at` out of its range, naming the attribute.
fn range_err(at: &str, e: &AttrRangeError) -> Error {
    out_err(&format!("{at}.{}", e.key), &e.message)
}

/// Paragraph style `v` from a filter's output, the entry `key` of the text at `parent`: each
/// attribute one a paragraph reads, each number in the range the paragraph style commands keep it
/// to ([`ParaStyle::check_ranges`], the Justification options as given), and a custom Kinsoku set
/// one of `doc`'s, by name, whose characters it takes.
fn para_style(doc: &Document, v: &Value, parent: &str, key: &str) -> Result<ParaStyle> {
    let mut st: ParaStyle = strict(v, parent, key)?;
    let at = format!("{parent}.{key}");
    known_keys::<vectorcraft_doc::HyphenationOptions>(v, &at, "hyphenation")?;
    known_keys::<JustificationOptions>(v, &at, "justification")?;
    known_keys::<vectorcraft_doc::TabStop>(v, &at, "tabs")?;
    known_keys::<vectorcraft_doc::text::ParaList>(v, &at, "list")?;
    let justification = match v.get("justification") {
        None | Some(Value::Null) => None,
        Some(j) => Some(serde_json::from_value::<JustificationOptions>(j.clone()).map_err(|e| out_err(&format!("{at}.justification"), e))?),
    };
    st.check_ranges(justification.as_ref(), &[], |_, _| true).map_err(|e| range_err(&at, &e))?;
    if let Some(set) = &st.kinsoku_set {
        let found = KinsokuSet::named(&doc.kinsoku_sets, &set.name).ok_or_else(|| {
            let shown = crate::permissions::display_text(&set.name, crate::permissions::MAX_SHOWN_FIELD);
            out_err(&format!("{at}.kinsoku_set"), format!("the document has no Kinsoku set named '{shown}'"))
        })?;
        st.kinsoku_set = Some(std::sync::Arc::new(found.clone()));
    }
    Ok(st)
}

/// Checks character style `s`, a filter's own (`was`: the style of the run in its place before):
/// its font family is 1 to [`MAX_FAMILY`] printable characters, counted as given, and its fill and
/// stroke are admitted as an object's paints are. Its numbers and its paints' keys are checked for
/// every run ([`set_text`]).
fn check_style(doc: &Document, s: &mut CharStyle, was: Option<&CharStyle>, at: &str) -> Result<()> {
    let family = s.font_family.trim();
    if family.is_empty() || s.font_family.chars().count() > MAX_FAMILY || s.font_family.chars().any(char::is_control) {
        return Err(out_err(&format!("{at}.font_family"), format!("must be 1-{MAX_FAMILY} printable characters")));
    }
    let paint = |p: &vectorcraft_color::Paint, was: Option<&vectorcraft_color::Paint>, key: &str| {
        let at = format!("{at}.{key}");
        let v = serde_json::to_value(p).map_err(|e| out_err(&at, e))?;
        admit_paint(doc, decode_paint(&v, &at)?, was, &at)
    };
    s.fill = paint(&s.fill, was.map(|w| &w.fill), "fill")?;
    s.stroke = paint(&s.stroke, was.map(|w| &w.stroke), "stroke")?;
    Ok(())
}

/// Checks that paint `v` of a style, at `at`, has no key besides `type` and those its `type`
/// reads; a key of another paint, or an unknown one, is an error naming it.
fn check_paint_keys(v: &Value, at: &str) -> Result<()> {
    let Some(o) = v.as_object() else { return Ok(()) };
    let known: &[&str] = match o.get("type").and_then(Value::as_str) {
        Some("solid") => &["color", "swatch", "tint"],
        Some("gradient") => attr_names::<vectorcraft_color::GradientPaint>(),
        Some("pattern") => &["pattern", "xf"],
        _ => &[],
    };
    if let Some(k) = o.keys().find(|k| *k != "type" && !known.contains(&k.as_str())) {
        let k = crate::permissions::display_text(k, crate::permissions::MAX_SHOWN_FIELD);
        return Err(out_err(at, format!("unknown key '{k}'")));
    }
    Ok(())
}
