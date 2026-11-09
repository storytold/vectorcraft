//! What a live effect that declares `"paintOutput": true` (at `api` 2) returns: painted pieces.
//!
//! Such an effect returns `{"objects": [...]}` as any live effect does, but each entry is a piece
//! drawn by itself, in the order returned, with paints of its own: `{"path", "fillRule"?,
//! "fills"?, "strokes"?}`. Fills and strokes take the forms a filter returns ([`crate::objects`]).
//! A piece without `fills` (or `strokes`) keeps the ones it was given; an empty list paints none.
//! An effect on a fill may return only `fills`, one on a stroke only `strokes`.
//!
//! The result is checked whole before any of it is used ([`decode`]): every number finite and in
//! range (nothing is clamped), every key known, and every swatch and pattern a paint names in the
//! document ([`check_references`]). One fault refuses the whole result, naming the field.

use serde_json::Value;
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, AppearanceItem, Document, LineCap, LineJoin};
use vectorcraft_geom::{FillRule, PathData};

use crate::objects::{MAX_COORD, MAX_ITEMS, MAX_OBJECTS, MAX_STROKE_WIDTH, MITER_LIMIT, OutStroke, check_point, out_err};
use crate::{Error, Result};

/// The keys a piece may have. `type` and `bounds` are read back from the input as it was given and
/// say nothing about the piece.
const PIECE_KEYS: [&str; 6] = ["type", "path", "fillRule", "bounds", "fills", "strokes"];
/// The keys a stroke of a piece may have.
const STROKE_KEYS: [&str; 5] = ["paint", "width", "join", "cap", "miterLimit"];

/// One piece a live effect returned: its geometry and fill rule, and its fills and strokes in paint
/// order (`None`: the ones it was given).
#[derive(Clone, Debug, PartialEq)]
pub struct Piece {
    pub path: PathData,
    pub rule: FillRule,
    pub fills: Option<Vec<Paint>>,
    pub strokes: Option<Vec<OutStroke>>,
}

/// Where in an appearance the effect sits, which says what its pieces may paint.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    /// One of the object's effects: a piece may return fills and strokes.
    Object,
    /// One of a fill's own effects: a piece may return only `fills`.
    Fill,
    /// One of a stroke's own effects: a piece may return only `strokes`.
    Stroke,
}

/// Parses and checks a painted result: `{"objects": [...]}`, or `{"error": "message"}` when the
/// plug-in declined. `rule` is the fill rule a piece without `fillRule` takes. Any other top-level
/// key, an unknown key in a piece, stroke or paint, a list or count past its limit, a number out of
/// range or a paint of the wrong form refuses the whole result, naming the field.
pub fn decode(bytes: &[u8], scope: Scope, rule: FillRule) -> Result<Vec<Piece>> {
    let v: Value = serde_json::from_slice(bytes).map_err(|e| out_err("output", format!("not JSON: {e}")))?;
    if let Some(e) = crate::objects::declined(&v) {
        return Err(e);
    }
    let top = v.as_object().ok_or_else(|| out_err("output", "expected {\"objects\": [...]}"))?;
    if let Some(k) = top.keys().find(|k| !matches!(k.as_str(), "objects" | "error")) {
        return Err(out_err(k, "a painted live effect returns only `objects`"));
    }
    let objects = top.get("objects").and_then(Value::as_array).ok_or_else(|| out_err("objects", "expected {\"objects\": [...]}"))?;
    if objects.len() > MAX_OBJECTS {
        return Err(out_err("objects", format!("{} objects (limit {MAX_OBJECTS})", objects.len())));
    }
    let mut anchors = 0;
    objects.iter().enumerate().map(|(i, o)| piece(o, &format!("objects[{i}]"), scope, rule, &mut anchors)).collect()
}

fn piece(v: &Value, at: &str, scope: Scope, rule: FillRule, anchors: &mut usize) -> Result<Piece> {
    let o = v.as_object().ok_or_else(|| out_err(at, "an object must be a JSON object"))?;
    if let Some(k) = o.keys().find(|k| !PIECE_KEYS.contains(&k.as_str())) {
        return Err(out_err(&format!("{at}.{k}"), "a painted live effect's piece has only `path`, `fillRule`, `fills` and `strokes`"));
    }
    if let Some(t) = o.get("type")
        && !matches!(t.as_str(), Some("path" | "compound"))
    {
        return Err(out_err(&format!("{at}.type"), "`type` must be \"path\" or \"compound\""));
    }
    let path = o.get("path").ok_or_else(|| out_err(&format!("{at}.path"), "each piece needs a `path`"))?;
    // At most MAX_ANCHORS over all pieces.
    let path = crate::objects::decode_path(path, &format!("{at}.path"), anchors)?;
    let rule = match o.get("fillRule") {
        None => rule,
        Some(r) => match r.as_str() {
            Some("nonzero") => FillRule::NonZero,
            Some("evenodd") => FillRule::EvenOdd,
            _ => return Err(out_err(&format!("{at}.fillRule"), "`fillRule` must be \"nonzero\" or \"evenodd\"")),
        },
    };
    let list = |key: &str| -> Result<Option<&Vec<Value>>> {
        match o.get(key) {
            None => Ok(None),
            Some(Value::Array(a)) if a.len() <= MAX_ITEMS => Ok(Some(a)),
            Some(_) => Err(out_err(&format!("{at}.{key}"), format!("`{key}` must be an array of at most {MAX_ITEMS} entries"))),
        }
    };
    let (fills, strokes) = (list("fills")?, list("strokes")?);
    match scope {
        Scope::Fill if strokes.is_some() => return Err(out_err(&format!("{at}.strokes"), "an effect on a fill returns only `fills`")),
        Scope::Stroke if fills.is_some() => return Err(out_err(&format!("{at}.fills"), "an effect on a stroke returns only `strokes`")),
        _ => {}
    }
    let fills = fills.map(|f| f.iter().enumerate().map(|(i, p)| paint(p, &format!("{at}.fills[{i}]"))).collect::<Result<Vec<_>>>()).transpose()?;
    let strokes =
        strokes.map(|s| s.iter().enumerate().map(|(i, s)| stroke(s, &format!("{at}.strokes[{i}]"))).collect::<Result<Vec<_>>>()).transpose()?;
    Ok(Piece { path, rule, fills, strokes })
}

/// A stroke `{paint, width?, join?, cap?, miterLimit?}`: a width of 0–[`MAX_STROKE_WIDTH`] and a
/// miter limit in [`MITER_LIMIT`].
fn stroke(v: &Value, at: &str) -> Result<OutStroke> {
    let s = v.as_object().ok_or_else(|| out_err(at, "a stroke must be {\"paint\", \"width\"?, \"join\"?, \"cap\"?, \"miterLimit\"?}"))?;
    if let Some(k) = s.keys().find(|k| !STROKE_KEYS.contains(&k.as_str())) {
        return Err(out_err(&format!("{at}.{k}"), "a stroke has only `paint`, `width`, `join`, `cap` and `miterLimit`"));
    }
    let number = |key: &str, range: std::ops::RangeInclusive<f64>| -> Result<Option<f64>> {
        let Some(v) = s.get(key) else { return Ok(None) };
        match v.as_f64().filter(|x| x.is_finite() && range.contains(x)) {
            Some(x) => Ok(Some(x)),
            None => Err(out_err(&format!("{at}.{key}"), format!("`{key}` must be a number from {} to {}", range.start(), range.end()))),
        }
    };
    let join = match s.get("join").map(Value::as_str) {
        None => None,
        Some(Some("miter")) => Some(LineJoin::Miter),
        Some(Some("round")) => Some(LineJoin::Round),
        Some(Some("bevel")) => Some(LineJoin::Bevel),
        Some(_) => return Err(out_err(&format!("{at}.join"), "`join` must be \"miter\", \"round\" or \"bevel\"")),
    };
    let cap = match s.get("cap").map(Value::as_str) {
        None => None,
        Some(Some("butt")) => Some(LineCap::Butt),
        Some(Some("round")) => Some(LineCap::Round),
        Some(Some("square")) => Some(LineCap::Square),
        Some(_) => return Err(out_err(&format!("{at}.cap"), "`cap` must be \"butt\", \"round\" or \"square\"")),
    };
    let paint = paint(s.get("paint").unwrap_or(&Value::Null), &format!("{at}.paint"))?;
    Ok(OutStroke { paint, width: number("width", 0.0..=MAX_STROKE_WIDTH)?, join, cap, miter_limit: number("miterLimit", MITER_LIMIT)? })
}

/// Is `x` finite and in `lo..=hi`?
fn within(x: f64, lo: f64, hi: f64) -> bool {
    x.is_finite() && (lo..=hi).contains(&x)
}

fn in_range(x: f32, lo: f32, hi: f32, at: &str, what: &str) -> Result<()> {
    if within(f64::from(x), f64::from(lo), f64::from(hi)) { Ok(()) } else { Err(out_err(at, format!("{what} must be from {lo} to {hi}"))) }
}

/// A color with every component in range: 0–1, or Lab's L 0–100 and a, b −128–127.
fn color(c: &Color, at: &str) -> Result<()> {
    let unit = |x: f32| in_range(x, 0.0, 1.0, at, "a color component");
    match *c {
        Color::Rgb { r, g, b } => [r, g, b].into_iter().try_for_each(unit),
        Color::Cmyk { c, m, y, k } => [c, m, y, k].into_iter().try_for_each(unit),
        Color::Gray { k } => unit(k),
        Color::Lab { l, a, b } => {
            in_range(l, 0.0, 100.0, at, "Lab's L")?;
            in_range(a, -128.0, 127.0, at, "Lab's a")?;
            in_range(b, -128.0, 127.0, at, "Lab's b")
        }
    }
}

/// Refuses a key of object `v` that is not in `keys`, naming it after `at`. A value that is not an
/// object passes, and deserializing it reports the fault.
fn only_keys(v: &Value, at: &str, keys: &[&str]) -> Result<()> {
    match v.as_object().and_then(|o| o.keys().find(|k| !keys.contains(&k.as_str()))) {
        Some(k) => Err(out_err(&format!("{at}.{k}"), format!("unknown key `{k}`; the keys here are {}", keys.join(", ")))),
        None => Ok(()),
    }
}

/// [`only_keys`] for a color, whose keys follow its `model`. An unknown model passes, and
/// deserializing it reports the fault.
fn color_keys(v: &Value, at: &str) -> Result<()> {
    let keys: &[&str] = match v.get("model").and_then(Value::as_str) {
        Some("rgb") => &["model", "r", "g", "b"],
        Some("cmyk") => &["model", "c", "m", "y", "k"],
        Some("gray") => &["model", "k"],
        Some("lab") => &["model", "l", "a", "b"],
        _ => return Ok(()),
    };
    only_keys(v, at, keys)
}

/// [`only_keys`] for a point `{x, y}`.
fn point_keys(v: Option<&Value>, at: &str) -> Result<()> {
    v.map_or(Ok(()), |v| only_keys(v, at, &["x", "y"]))
}

/// Calls `f` on each entry of the array at `v`, named `at[i]`.
fn each(v: Option<&Value>, at: &str, f: impl Fn(&Value, &str) -> Result<()>) -> Result<()> {
    v.and_then(Value::as_array).into_iter().flatten().enumerate().try_for_each(|(i, e)| f(e, &format!("{at}[{i}]")))
}

/// Refuses an unknown key at any level of paint `v`: the paint itself, its colors, a gradient's
/// stops, placement and freeform points. Deserializing a paint drops unknown keys, so a key such
/// as `opacity` on a solid paint would otherwise be accepted and do nothing.
fn paint_keys(v: &Value, at: &str) -> Result<()> {
    match v.get("type").and_then(Value::as_str) {
        Some("none") => only_keys(v, at, &["type"]),
        Some("solid") => {
            only_keys(v, at, &["type", "color", "swatch", "tint"])?;
            v.get("color").map_or(Ok(()), |c| color_keys(c, &format!("{at}.color")))
        }
        Some("pattern") => only_keys(v, at, &["type", "pattern", "xf"]),
        Some("gradient") => {
            only_keys(v, at, &["type", "gradient", "geom", "angle", "swatch", "freeform"])?;
            if let Some(g) = v.get("gradient") {
                let at = format!("{at}.gradient");
                only_keys(g, &at, &["kind", "stops", "dither", "method"])?;
                each(g.get("stops"), &format!("{at}.stops"), |s, at| {
                    only_keys(s, at, &["offset", "color", "opacity", "midpoint", "swatch", "tint", "plates"])?;
                    s.get("color").map_or(Ok(()), |c| color_keys(c, &format!("{at}.color")))
                })?;
            }
            if let Some(geom) = v.get("geom") {
                let at = format!("{at}.geom");
                only_keys(geom, &at, &["start", "end", "aspect", "focal"])?;
                ["start", "end", "focal"].into_iter().try_for_each(|k| point_keys(geom.get(k), &format!("{at}.{k}")))?;
            }
            if let Some(ff) = v.get("freeform") {
                let at = format!("{at}.freeform");
                only_keys(ff, &at, &["points", "lines", "mode"])?;
                each(ff.get("points"), &format!("{at}.points"), |p, at| {
                    only_keys(p, at, &["at", "color", "opacity", "spread"])?;
                    point_keys(p.get("at"), &format!("{at}.at"))?;
                    p.get("color").map_or(Ok(()), |c| color_keys(c, &format!("{at}.color")))
                })?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

/// A paint as the filter form has it (`null`: none), checked rather than clamped: colors, tints,
/// stops, opacities, midpoints and placements in range.
fn paint(v: &Value, at: &str) -> Result<Paint> {
    if v.is_null() {
        return Ok(Paint::None);
    }
    paint_keys(v, at)?;
    let p: Paint = serde_json::from_value(v.clone()).map_err(|e| out_err(at, e))?;
    let unit = |x: f32, at: &str, what: &str| in_range(x, 0.0, 1.0, at, what);
    match &p {
        Paint::None => {}
        Paint::Solid { color: c, tint, .. } => {
            color(c, &format!("{at}.color"))?;
            unit(*tint, &format!("{at}.tint"), "`tint`")?;
        }
        Paint::Gradient(g) => {
            let stops = &g.gradient.stops;
            if stops.is_empty() || stops.len() > MAX_ITEMS {
                return Err(out_err(&format!("{at}.gradient.stops"), format!("a gradient needs 1-{MAX_ITEMS} stops")));
            }
            for (i, s) in stops.iter().enumerate() {
                let at = format!("{at}.gradient.stops[{i}]");
                unit(s.offset, &format!("{at}.offset"), "`offset`")?;
                unit(s.opacity, &format!("{at}.opacity"), "`opacity`")?;
                in_range(s.midpoint, 0.13, 0.87, &format!("{at}.midpoint"), "`midpoint`")?;
                unit(s.tint, &format!("{at}.tint"), "`tint`")?;
                color(&s.color, &format!("{at}.color"))?;
                for x in s.plates.iter().flatten() {
                    unit(*x, &format!("{at}.plates"), "a plate")?;
                }
            }
            if stops.windows(2).any(|w| w[1].offset < w[0].offset) {
                return Err(out_err(&format!("{at}.gradient.stops"), "stops must be in order of `offset`"));
            }
            if !g.angle.is_finite() {
                return Err(out_err(&format!("{at}.angle"), "`angle` must be a number"));
            }
            if let Some(geom) = &g.geom {
                check_point(geom.start, &format!("{at}.geom.start"))?;
                check_point(geom.end, &format!("{at}.geom.end"))?;
                if let Some(f) = geom.focal {
                    check_point(f, &format!("{at}.geom.focal"))?;
                }
                if !within(geom.aspect, 0.01, 100.0) {
                    return Err(out_err(&format!("{at}.geom.aspect"), "`aspect` must be from 0.01 to 100"));
                }
            }
            if let Some(ff) = &g.freeform {
                if ff.points.len() > MAX_ITEMS || ff.lines.len() > MAX_ITEMS || ff.lines.iter().any(|l| l.len() > MAX_ITEMS) {
                    return Err(out_err(&format!("{at}.freeform"), format!("a freeform gradient holds at most {MAX_ITEMS} points and lines")));
                }
                if ff.lines.iter().flatten().any(|i| *i >= ff.points.len()) {
                    return Err(out_err(&format!("{at}.freeform.lines"), "a freeform gradient line names a point it doesn't have"));
                }
                for (i, pt) in ff.points.iter().enumerate() {
                    let at = format!("{at}.freeform.points[{i}]");
                    check_point(pt.at, &format!("{at}.at"))?;
                    color(&pt.color, &format!("{at}.color"))?;
                    unit(pt.opacity, &format!("{at}.opacity"), "`opacity`")?;
                    unit(pt.spread, &format!("{at}.spread"), "`spread`")?;
                }
            }
        }
        Paint::Pattern { xf, .. } => {
            if !xf.as_coeffs().iter().all(|c| within(*c, -MAX_COORD, MAX_COORD)) {
                return Err(out_err(&format!("{at}.xf"), "pattern transform out of range"));
            }
        }
    }
    Ok(p)
}

/// Checks that every swatch a solid paint, gradient or gradient stop of `pieces` links to is a
/// swatch of `doc`, and every pattern a pattern paint names is one of its patterns. Without a
/// document (`None`), any swatch or pattern a piece names is refused.
pub fn check_references(pieces: &[Piece], doc: Option<&Document>) -> Result<()> {
    let swatch = |name: &Option<String>, at: &str| match name {
        Some(n) if doc.and_then(|d| d.swatch(n)).is_none() => Err(out_err(at, format!("the document has no swatch {n:?}"))),
        _ => Ok(()),
    };
    let check = |p: &Paint, at: &str| -> Result<()> {
        match p {
            Paint::None => Ok(()),
            Paint::Solid { swatch: s, .. } => swatch(s, &format!("{at}.swatch")),
            Paint::Gradient(g) => {
                swatch(&g.swatch, &format!("{at}.swatch"))?;
                g.gradient.stops.iter().enumerate().try_for_each(|(i, s)| swatch(&s.swatch, &format!("{at}.gradient.stops[{i}].swatch")))
            }
            Paint::Pattern { pattern, .. } if doc.and_then(|d| d.pattern(pattern)).is_none() => {
                Err(out_err(&format!("{at}.pattern"), format!("the document has no pattern {pattern:?}")))
            }
            Paint::Pattern { .. } => Ok(()),
        }
    };
    for (i, p) in pieces.iter().enumerate() {
        for (k, f) in p.fills.iter().flatten().enumerate() {
            check(f, &format!("objects[{i}].fills[{k}]"))?;
        }
        for (k, s) in p.strokes.iter().flatten().enumerate() {
            check(&s.paint, &format!("objects[{i}].strokes[{k}].paint"))?;
        }
    }
    Ok(())
}

/// The current color of a paint or stop linked to global swatch `name` at `tint`, when it differs
/// from `color`.
fn newer_color(color: Color, name: Option<&str>, tint: f32, doc: &Document) -> Option<Color> {
    doc.global_color(name?).map(|c| c.tinted(tint)).filter(|c| *c != color)
}

/// Does `p` hold a color older than the global swatch it is linked to?
fn behind_swatches(p: &Paint, doc: &Document) -> bool {
    match p {
        Paint::Solid { color, swatch, tint } => newer_color(*color, swatch.as_deref(), *tint, doc).is_some(),
        Paint::Gradient(g) => g.gradient.stops.iter().any(|s| newer_color(s.color, s.swatch.as_deref(), s.tint, doc).is_some()),
        Paint::None | Paint::Pattern { .. } => false,
    }
}

/// Gives a solid paint or gradient stop of `p` linked to a global swatch that swatch's current
/// color at its own tint.
fn take_swatch_colors(p: &mut Paint, doc: &Document) {
    match p {
        Paint::Solid { color, swatch, tint } => {
            if let Some(c) = newer_color(*color, swatch.as_deref(), *tint, doc) {
                *color = c;
            }
        }
        Paint::Gradient(g) => {
            for s in &mut g.gradient.stops {
                if let Some(c) = newer_color(s.color, s.swatch.as_deref(), s.tint, doc) {
                    s.color = c;
                }
            }
        }
        Paint::None | Paint::Pattern { .. } => {}
    }
}

/// `pieces` with every paint linked to a global swatch of `doc` in that swatch's current color, as
/// a global swatch edit changes linked art. A paint linked to a swatch that isn't global keeps the
/// color the plug-in gave.
pub(crate) fn with_swatch_colors(mut pieces: std::sync::Arc<Vec<Piece>>, doc: &Document) -> std::sync::Arc<Vec<Piece>> {
    let behind =
        |p: &Piece| p.fills.iter().flatten().any(|f| behind_swatches(f, doc)) || p.strokes.iter().flatten().any(|s| behind_swatches(&s.paint, doc));
    if pieces.iter().any(behind) {
        for p in std::sync::Arc::make_mut(&mut pieces) {
            p.fills.iter_mut().flatten().for_each(|f| take_swatch_colors(f, doc));
            p.strokes.iter_mut().flatten().for_each(|s| take_swatch_colors(&mut s.paint, doc));
        }
    }
    pieces
}

/// How many paints, gradient stops and freeform points `pieces` hold: what a cache counts them as
/// besides their anchors.
pub(crate) fn paint_count(pieces: &[Piece]) -> usize {
    let one = |p: &Paint| match p {
        Paint::Gradient(g) => 1 + g.gradient.stops.len() + g.freeform.as_ref().map_or(0, |f| f.points.len()),
        _ => 1,
    };
    pieces.iter().map(|p| p.fills.iter().flatten().map(one).sum::<usize>() + p.strokes.iter().flatten().map(|s| one(&s.paint)).sum::<usize>()).sum()
}

/// The appearance piece `p` of an object-level effect is drawn with: `base` (the object's) with
/// its fills, then its strokes, replaced by the piece's where it has them, as a filter's output
/// sets them (an existing item keeps everything but what the piece gives; new ones take the
/// defaults).
pub fn appearance(base: &Appearance, p: &Piece) -> Appearance {
    let mut ap = base.clone();
    if let Some(fills) = &p.fills {
        crate::objects::set_items(&mut ap, true, fills.iter().cloned().map(OutStroke::paint).collect());
    }
    if let Some(strokes) = &p.strokes {
        crate::objects::set_items(&mut ap, false, strokes.clone());
    }
    ap
}

/// The items piece `p` of an effect on `item` (a fill or a stroke) is drawn with, in paint order:
/// `item` itself when the piece keeps it, else one copy of it for each paint or stroke the piece
/// gives, with that paint (and that stroke's weight, join, cap and miter limit where given).
pub fn items(item: &AppearanceItem, p: &Piece) -> Vec<AppearanceItem> {
    match item {
        AppearanceItem::Fill(f) => match &p.fills {
            None => vec![item.clone()],
            Some(fills) => fills.iter().map(|paint| AppearanceItem::Fill(vectorcraft_doc::FillLayer { paint: paint.clone(), ..f.clone() })).collect(),
        },
        AppearanceItem::Stroke(s) => match &p.strokes {
            None => vec![item.clone()],
            Some(strokes) => strokes
                .iter()
                .map(|o| {
                    let mut s = s.clone();
                    crate::objects::style(&mut s, o.clone());
                    AppearanceItem::Stroke(s)
                })
                .collect(),
        },
    }
}

/// An error naming `name`, the plug-in, before what went wrong (`plugin.info`'s `lastError`).
pub(crate) fn named(name: &str, e: &Error) -> String {
    format!("{name}: {e}")
}

#[cfg(test)]
mod tests;
