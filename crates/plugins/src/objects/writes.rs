//! What a filter whose manifest lists `writes` may change about the type, images and symbol
//! instances it was given: each one's name and transform; type's runs, with their character
//! attributes, and its paragraph styles; an image's opacity; and the symbol an instance shows.
//! Every other field stays read-only, and every value is checked before anything changes.

use serde::de::DeserializeOwned;
use serde_json::{Map, Value};
use vectorcraft_doc::text::INLINE_CHAR;
use vectorcraft_doc::{CharStyle, Document, Node, NodeKind, ParaStyle, TextObject, TextRun};
use vectorcraft_geom::Affine;

use super::{
    MAX_COORD, MAX_INPUT_RUNS, MAX_INPUT_TEXT, MAX_INPUT_TEXT_TOTAL, admit_paint, decode_name, decode_paint, first_difference, out_err,
    read_only_changed,
};
use crate::{InputKind, Result};

/// The smallest and largest font size a filter may set, in points (as the Character panel's).
const FONT_SIZE: std::ops::RangeInclusive<f64> = 0.1..=1296.0;
/// The longest font family name a filter may set, in characters.
const MAX_FAMILY: usize = 256;
/// The smallest determinant a transform a filter sets may have: a smaller one flattens the object.
const MIN_DETERMINANT: f64 = 1e-12;

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
            ("text", Some(was)) => text_changes(v, was, given.get("truncated") == Some(&Value::Bool(true)), at, text, &mut out)?,
            ("name", None) if v.is_null() => {}
            (_, Some(was)) if first_difference(v, was, "").is_none() => {}
            _ => {
                out.insert(k.clone(), v.clone());
            }
        }
    }
    Ok((!out.is_empty()).then_some(out))
}

/// The `runs` and `paragraphs` of type text `v` that differ from `was` (the text the input gave),
/// into `out`; its other fields must be as given, and type given as `truncated` keeps its text. A
/// changed list is held to [`MAX_INPUT_RUNS`] entries, and changed runs to [`MAX_INPUT_TEXT`] bytes
/// of text, which count toward `total`, held to [`MAX_INPUT_TEXT_TOTAL`].
fn text_changes(v: &Value, was: &Value, truncated: bool, at: &str, total: &mut usize, out: &mut Map<String, Value>) -> Result<()> {
    let t = v.as_object().ok_or_else(|| out_err(at, "`text` must be an object"))?;
    let mut keys: Vec<&String> = t.keys().collect();
    keys.sort();
    for k in keys {
        let field = format!("text.{k}");
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
            *total = total.saturating_add(bytes);
            if *total > MAX_INPUT_TEXT_TOTAL {
                return Err(out_err(&format!("{at}.{field}"), format!("more than {MAX_INPUT_TEXT_TOTAL} bytes of text over the output")));
            }
        }
        out.insert(k.clone(), t[k].clone());
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

/// `v` as a `T` whose attributes are all ones `T` reads: an unknown key, as an ill-typed value, is
/// an error at `at`.
fn strict<T: DeserializeOwned>(v: &Value, at: &str, what: &str) -> Result<T> {
    let o = v.as_object().ok_or_else(|| out_err(at, format!("{what} must be an object")))?;
    let names = attr_names::<T>();
    if let Some(k) = o.keys().find(|k| !names.contains(&k.as_str())) {
        let k = crate::permissions::display_text(k, crate::permissions::MAX_SHOWN_FIELD);
        return Err(out_err(at, format!("unknown attribute '{k}'")));
    }
    serde_json::from_value(v.clone()).map_err(|e| out_err(at, e))
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
        let NodeKind::Text(t) = &mut n.kind else { return Err(out_err(at, "only type has text")) };
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
/// were. A run whose style isn't one of the type's own has a font family and a size in
/// [`FONT_SIZE`], and its paints are checked as an object's are. Paragraph styles are one per
/// paragraph. With runs alone, each paragraph keeps its style when the number of paragraphs stays
/// the same, and otherwise the styles move with the text as typing the change moves them.
fn set_text(doc: &Document, t: &mut TextObject, threaded: bool, runs: Option<&Value>, paragraphs: Option<&Value>, at: &str) -> Result<()> {
    if let Some(runs) = runs {
        let at = format!("{at}.text.runs");
        let list = runs.as_array().map(Vec::as_slice).unwrap_or_default();
        let own: Vec<Value> = t.runs.iter().map(|r| serde_json::to_value(&r.style).unwrap_or(Value::Null)).collect();
        let mut new = Vec::with_capacity(list.len());
        for (i, v) in list.iter().enumerate() {
            let at = format!("{at}[{i}]");
            let o = v.as_object().ok_or_else(|| out_err(&at, "a run must be {\"text\", \"style\", \"inline\"?}"))?;
            if let Some(k) = o.keys().find(|k| !matches!(k.as_str(), "text" | "style" | "inline")) {
                let k = crate::permissions::display_text(k, crate::permissions::MAX_SHOWN_FIELD);
                return Err(out_err(&at, format!("unknown key '{k}'")));
            }
            let text = o.get("text").and_then(Value::as_str).ok_or_else(|| out_err(&at, "`text` must be a string"))?;
            let style_v = o.get("style").unwrap_or(&Value::Null);
            let mut style: CharStyle = strict(style_v, &format!("{at}.style"), "`style`")?;
            let inline = match o.get("inline") {
                None | Some(Value::Null) => None,
                Some(v) => Some(serde_json::from_value(v.clone()).map_err(|e| out_err(&format!("{at}.inline"), e))?),
            };
            if inline.is_none() {
                if text.contains(INLINE_CHAR) {
                    return Err(out_err(&at, "`text` holds the character of an inline graphic"));
                }
                if !own.contains(style_v) {
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
        if plain != old {
            if threaded {
                return Err(out_err(&at, "the characters of threaded type can't be changed"));
            }
            if !t.ruby.is_empty() {
                return Err(out_err(&at, "the characters of type with ruby can't be changed"));
            }
            let breaks = |s: &str| s.bytes().filter(|&b| b == b'\n').count();
            if paragraphs.is_none() && breaks(&plain) != breaks(&old) {
                // Paragraphs added or joined: the styles follow the text as typing the change would
                // move them. With as many paragraphs as before, each keeps its style.
                let (start, old_end, new_end) = changed_range(&old, &plain);
                t.splice_paras(start, old_end, plain.get(start..new_end).unwrap_or_default());
            }
        }
        t.runs = new;
    }
    match paragraphs {
        Some(v) => {
            let at = format!("{at}.text.paragraphs");
            let list = v.as_array().map(Vec::as_slice).unwrap_or_default();
            let count = t.paragraph_count();
            if list.len() != count {
                return Err(out_err(&at, format!("expected {count} paragraph styles, one per paragraph")));
            }
            let styles =
                list.iter().enumerate().map(|(i, p)| strict::<ParaStyle>(p, &format!("{at}[{i}]"), "a paragraph style")).collect::<Result<_>>()?;
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

/// The bytes that differ between `old` and `new` once their common start and end are taken away,
/// on character boundaries: `old[start..old_end]` became `new[start..new_end]`.
fn changed_range(old: &str, new: &str) -> (usize, usize, usize) {
    let start = old.char_indices().zip(new.chars()).find(|((_, a), b)| a != b).map_or(old.len().min(new.len()), |((i, _), _)| i);
    let (old_rest, new_rest) = (old.get(start..).unwrap_or_default(), new.get(start..).unwrap_or_default());
    let same: usize = old_rest.chars().rev().zip(new_rest.chars().rev()).take_while(|(a, b)| a == b).map(|(c, _)| c.len_utf8()).sum();
    (start, old.len() - same, new.len() - same)
}

/// Checks character style `s`, a filter's own (`was`: the style of the run in its place before):
/// its font family is 1 to [`MAX_FAMILY`] printable characters, its size is in [`FONT_SIZE`], and
/// its fill and stroke are admitted as an object's paints are.
fn check_style(doc: &Document, s: &mut CharStyle, was: Option<&CharStyle>, at: &str) -> Result<()> {
    let family = s.font_family.trim();
    if family.is_empty() || family.chars().count() > MAX_FAMILY || s.font_family.chars().any(char::is_control) {
        return Err(out_err(&format!("{at}.font_family"), format!("must be 1-{MAX_FAMILY} printable characters")));
    }
    if !FONT_SIZE.contains(&s.size) {
        return Err(out_err(&format!("{at}.size"), format!("must be {} to {} pt", FONT_SIZE.start(), FONT_SIZE.end())));
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
