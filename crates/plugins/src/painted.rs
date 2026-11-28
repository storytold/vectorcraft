//! What a live effect that declares `"paintOutput": true` (at `api` 2) returns: painted pieces.
//!
//! Such an effect returns `{"objects": [...]}` as any live effect does, but each entry is a piece
//! drawn by itself, in the order returned, with paints of its own: `{"path", "fillRule"?,
//! "fills"?, "strokes"?}`. Fills and strokes take the forms a filter returns ([`crate::objects`]).
//! A piece without `fills` (or `strokes`) keeps the ones it was given; an empty list paints none.
//! An effect on a fill may return only `fills`, one on a stroke only `strokes`.
//!
//! An effect that also declares `"appearanceOutput": true` (at `api` 3) may give a piece, and each
//! fill or stroke it returns, an `opacity` (0–1), a `blend` (a [`BlendMode`] by its identifier) and
//! `effects` ([`Look`]): built-in effects only, which the host checks ([`CheckEffect`]), at most
//! [`MAX_LOOK_EFFECTS`] in one list and [`MAX_RESULT_EFFECTS`] in one result. A fill that carries
//! them is `{"paint", "opacity"?, "blend"?, "effects"?}`. The fills and strokes an effect on a fill
//! or a stroke returns take no `effects`.
//!
//! The result is checked whole before any of it is used ([`decode`]): every number finite and in
//! range (nothing is clamped), every key known, a gradient stop's `plates` only on a stop linked to
//! a spot swatch, and every swatch and pattern a paint names in the document ([`check_references`]).
//! One fault refuses the whole result, naming the field.

use serde_json::{Map, Value};
use vectorcraft_color::{BlendMode, Color, Paint};
use vectorcraft_doc::{Appearance, AppearanceItem, Document, Effect, LineCap, LineJoin};
use vectorcraft_geom::{FillRule, PathData};

use crate::objects::{MAX_COORD, MAX_ITEMS, MAX_OBJECTS, MAX_STROKE_WIDTH, MITER_LIMIT, OutStroke, check_point, out_err};
use crate::{Error, Result};

/// The keys a piece may have. `type` and `bounds` are read back from the input as it was given and
/// say nothing about the piece.
const PIECE_KEYS: [&str; 6] = ["type", "path", "fillRule", "bounds", "fills", "strokes"];
/// The keys a stroke of a piece may have.
const STROKE_KEYS: [&str; 5] = ["paint", "width", "join", "cap", "miterLimit"];
/// The keys a piece, a stroke or a fill also may have when the effect declares `appearanceOutput`.
const LOOK_KEYS: [&str; 3] = ["opacity", "blend", "effects"];
/// The look keys a fill or stroke has that an effect on a fill or a stroke returns.
const ROW_LOOK_KEYS: [&str; 2] = ["opacity", "blend"];
/// The keys an effect record has.
const EFFECT_KEYS: [&str; 3] = ["id", "params", "visible"];

/// Most effect records one piece, fill or stroke may carry.
pub const MAX_LOOK_EFFECTS: usize = 16;
/// Most effect records one result may carry, over all its pieces, fills and strokes.
pub const MAX_RESULT_EFFECTS: usize = 4_096;

/// How the host checks an effect record a piece (`false`) or one of its fills or strokes (`true`)
/// carries: `Err` with the reason when the record names no built-in effect that may be there or has
/// parameters that effect does not take. The plug-in crate checks the record's form; which effects
/// exist and what they take is the host's to say.
pub type CheckEffect = fn(&Effect, bool) -> std::result::Result<(), String>;

/// The [`CheckEffect`] of a host that lets no effect record in.
pub fn no_effects(_: &Effect, _: bool) -> std::result::Result<(), String> {
    Err("this host takes no effects here".into())
}

/// The host's [`CheckEffect`], set once ([`set_effect_check`]).
static CHECK: std::sync::OnceLock<CheckEffect> = std::sync::OnceLock::new();

/// Makes `check` how the effect records of painted results are checked in this process, unless
/// one was set before. Until one is set, every effect record is refused ([`no_effects`]).
pub fn set_effect_check(check: CheckEffect) {
    let _ = CHECK.set(check);
}

/// How the effect records of painted results are checked ([`set_effect_check`]).
pub(crate) fn effect_check() -> CheckEffect {
    CHECK.get().copied().unwrap_or(no_effects)
}

/// The opacity, blending mode and effects a piece, or one of its fills or strokes, is given: each
/// `None` (or empty) keeps what it would have without.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Look {
    pub opacity: Option<f32>,
    pub blend: Option<BlendMode>,
    /// Drawn after the effects it already has.
    pub effects: Vec<Effect>,
}

impl Look {
    /// Does it keep everything as it is?
    pub fn is_empty(&self) -> bool {
        self.opacity.is_none() && self.blend.is_none() && self.effects.is_empty()
    }

    /// Gives fill or stroke `item` this opacity and blending mode where given, and these effects
    /// after its own.
    fn dress(&self, item: &mut AppearanceItem) {
        let (opacity, blend, effects) = match item {
            AppearanceItem::Fill(f) => (&mut f.opacity, &mut f.blend, &mut f.effects),
            AppearanceItem::Stroke(s) => (&mut s.opacity, &mut s.blend, &mut s.effects),
        };
        if let Some(o) = self.opacity {
            *opacity = o;
        }
        if let Some(b) = self.blend {
            *blend = b;
        }
        effects.extend(self.effects.iter().cloned());
    }
}

/// One piece a live effect returned: its geometry and fill rule, and its fills and strokes in paint
/// order (`None`: the ones it was given), with the look of the piece and of each fill and stroke it
/// gives (`fill_looks[k]` for `fills[k]`; a missing one keeps everything).
#[derive(Clone, Debug, PartialEq)]
pub struct Piece {
    pub path: PathData,
    pub rule: FillRule,
    pub fills: Option<Vec<Paint>>,
    pub strokes: Option<Vec<OutStroke>>,
    pub look: Look,
    pub fill_looks: Vec<Look>,
    pub stroke_looks: Vec<Look>,
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
/// plug-in declined. `rule` is the fill rule a piece without `fillRule` takes. A top-level `notice`
/// is ignored. Any other top-level key, an unknown key in a piece, stroke or paint, a list or count past its limit, a number out of
/// range or a paint of the wrong form refuses the whole result, naming the field.
pub fn decode(bytes: &[u8], scope: Scope, rule: FillRule) -> Result<Vec<Piece>> {
    decode_with(bytes, scope, rule, None)
}

/// [`decode`] for an effect that declares `appearanceOutput` when `looks` is `Some`: its pieces,
/// fills and strokes may then carry opacity, a blending mode and effects, each effect record
/// checked by it.
pub fn decode_with(bytes: &[u8], scope: Scope, rule: FillRule, looks: Option<CheckEffect>) -> Result<Vec<Piece>> {
    let v: Value = serde_json::from_slice(bytes).map_err(|e| out_err("output", format!("not JSON: {e}")))?;
    if let Some(e) = crate::objects::declined(&v) {
        return Err(e);
    }
    let top = v.as_object().ok_or_else(|| out_err("output", "expected {\"objects\": [...]}"))?;
    if let Some(k) = top.keys().find(|k| !matches!(k.as_str(), "objects" | "error" | "notice")) {
        return Err(out_err(k, "a painted live effect returns only `objects` and `notice`"));
    }
    let objects = top.get("objects").and_then(Value::as_array).ok_or_else(|| out_err("objects", "expected {\"objects\": [...]}"))?;
    if objects.len() > MAX_OBJECTS {
        return Err(out_err("objects", format!("{} objects (limit {MAX_OBJECTS})", objects.len())));
    }
    let mut counts = Counts { anchors: 0, effects: 0, looks, row_effects: scope == Scope::Object };
    objects.iter().enumerate().map(|(i, o)| piece(o, &format!("objects[{i}]"), scope, rule, &mut counts)).collect()
}

/// What a result holds so far, which its limits count: anchors and effect records, and how its
/// effect records are checked (`None`: the result carries no look), and whether its fills and
/// strokes may carry `effects` (only those of an object's effect: one on a fill or a stroke sits
/// among that fill's or stroke's own effects).
struct Counts {
    anchors: usize,
    effects: usize,
    looks: Option<CheckEffect>,
    row_effects: bool,
}

impl Counts {
    /// The look keys a fill or stroke may have.
    fn row_look_keys(&self) -> &'static [&'static str] {
        if self.row_effects { &LOOK_KEYS } else { &ROW_LOOK_KEYS }
    }
}

fn piece(v: &Value, at: &str, scope: Scope, rule: FillRule, counts: &mut Counts) -> Result<Piece> {
    let o = v.as_object().ok_or_else(|| out_err(at, "an object must be a JSON object"))?;
    let known = |k: &str| PIECE_KEYS.contains(&k) || (counts.looks.is_some() && LOOK_KEYS.contains(&k));
    if let Some(k) = o.keys().find(|k| !known(k)) {
        if counts.looks.is_some() {
            return Err(out_err(
                &format!("{at}.{k}"),
                "a painted live effect's piece has only `path`, `fillRule`, `fills`, `strokes`, `opacity`, `blend` and `effects`",
            ));
        }
        return Err(out_err(&format!("{at}.{k}"), "a painted live effect's piece has only `path`, `fillRule`, `fills` and `strokes`"));
    }
    if let Some(t) = o.get("type")
        && !matches!(t.as_str(), Some("path" | "compound"))
    {
        return Err(out_err(&format!("{at}.type"), "`type` must be \"path\" or \"compound\""));
    }
    let path = o.get("path").ok_or_else(|| out_err(&format!("{at}.path"), "each piece needs a `path`"))?;
    // At most MAX_ANCHORS over all pieces.
    let path = crate::objects::decode_path(path, &format!("{at}.path"), &mut counts.anchors)?;
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
    let look = look(o, at, false, counts)?;
    let mut fill_looks = vec![];
    let fills = match fills {
        None => None,
        Some(f) => {
            let rows = f.iter().enumerate().map(|(i, p)| fill_row(p, &format!("{at}.fills[{i}]"), counts)).collect::<Result<Vec<_>>>()?;
            let (paints, looks): (Vec<Paint>, Vec<Look>) = rows.into_iter().unzip();
            fill_looks = looks;
            Some(paints)
        }
    };
    let mut stroke_looks = vec![];
    let strokes = match strokes {
        None => None,
        Some(s) => {
            let rows = s.iter().enumerate().map(|(i, s)| stroke(s, &format!("{at}.strokes[{i}]"), counts)).collect::<Result<Vec<_>>>()?;
            let (strokes, looks): (Vec<OutStroke>, Vec<Look>) = rows.into_iter().unzip();
            stroke_looks = looks;
            Some(strokes)
        }
    };
    // A look that keeps everything is left out, so a result without any compares as one.
    for looks in [&mut fill_looks, &mut stroke_looks] {
        if looks.iter().all(Look::is_empty) {
            looks.clear();
        }
    }
    Ok(Piece { path, rule, fills, strokes, look, fill_looks, stroke_looks })
}

/// A fill of a piece: a paint, or with `appearanceOutput` also `{"paint", "opacity"?, "blend"?,
/// "effects"?}` (an object with `paint` and no `type`), with its look.
fn fill_row(v: &Value, at: &str, counts: &mut Counts) -> Result<(Paint, Look)> {
    match v.as_object() {
        Some(o) if counts.looks.is_some() && o.contains_key("paint") && !o.contains_key("type") => {
            let keys: Vec<&str> = std::iter::once("paint").chain(counts.row_look_keys().iter().copied()).collect();
            only_keys(v, at, &keys)?;
            let look = look(o, at, true, counts)?;
            Ok((paint(o.get("paint").unwrap_or(&Value::Null), &format!("{at}.paint"))?, look))
        }
        _ => Ok((paint(v, at)?, Look::default())),
    }
}

/// The identifiers `blend` takes, in Transparency-panel order.
fn blend_names() -> String {
    BlendMode::ALL.iter().map(|m| format!("{m:?}")).collect::<Vec<_>>().join(", ")
}

/// The opacity, blending mode and effects object `o` of a piece (`row` off) or of one of its fills
/// or strokes gives, checked: an `opacity` from 0 to 1, a `blend` that is one of [`BlendMode`]'s
/// identifiers exactly, and at most [`MAX_LOOK_EFFECTS`] effect records, each `{id, params?,
/// visible?}` that the host's [`CheckEffect`] lets in, within [`MAX_RESULT_EFFECTS`] for the whole
/// result. Without `appearanceOutput` the keys are refused before this is reached.
fn look(o: &Map<String, Value>, at: &str, row: bool, counts: &mut Counts) -> Result<Look> {
    let Some(check) = counts.looks else { return Ok(Look::default()) };
    let opacity = match o.get("opacity") {
        None => None,
        Some(v) => match v.as_f64().filter(|x| within(*x, 0.0, 1.0)) {
            Some(x) => Some(x as f32),
            None => return Err(out_err(&format!("{at}.opacity"), "`opacity` must be a number from 0 to 1")),
        },
    };
    let blend = match o.get("blend") {
        None => None,
        Some(v) => match v.as_str().and_then(|s| BlendMode::ALL.into_iter().find(|m| format!("{m:?}") == s)) {
            Some(b) => Some(b),
            None => return Err(out_err(&format!("{at}.blend"), format!("unknown blending mode; `blend` is one of {}", blend_names()))),
        },
    };
    let records = match o.get("effects") {
        None => &[][..],
        Some(Value::Array(a)) if a.len() <= MAX_LOOK_EFFECTS => a.as_slice(),
        Some(_) => return Err(out_err(&format!("{at}.effects"), format!("`effects` must be an array of at most {MAX_LOOK_EFFECTS} records"))),
    };
    let mut effects = Vec::with_capacity(records.len());
    for (j, r) in records.iter().enumerate() {
        let at = format!("{at}.effects[{j}]");
        counts.effects += 1;
        if counts.effects > MAX_RESULT_EFFECTS {
            return Err(out_err(&at, format!("a result carries at most {MAX_RESULT_EFFECTS} effect records")));
        }
        only_keys(r, &at, &EFFECT_KEYS)?;
        let e = crate::objects::decode_effect(r, &at)?;
        check(&e, row).map_err(|why| out_err(&at, why))?;
        effects.push(e);
    }
    Ok(Look { opacity, blend, effects })
}

/// A stroke `{paint, width?, join?, cap?, miterLimit?}`: a width of 0–[`MAX_STROKE_WIDTH`] and a
/// miter limit in [`MITER_LIMIT`]; with `appearanceOutput` also its look.
fn stroke(v: &Value, at: &str, counts: &mut Counts) -> Result<(OutStroke, Look)> {
    let s = v.as_object().ok_or_else(|| out_err(at, "a stroke must be {\"paint\", \"width\"?, \"join\"?, \"cap\"?, \"miterLimit\"?}"))?;
    if counts.looks.is_some() && !counts.row_effects {
        let keys: Vec<&str> = STROKE_KEYS.iter().chain(&ROW_LOOK_KEYS).copied().collect();
        only_keys(v, at, &keys)?;
    }
    let known = |k: &str| STROKE_KEYS.contains(&k) || (counts.looks.is_some() && LOOK_KEYS.contains(&k));
    if let Some(k) = s.keys().find(|k| !known(k)) {
        if counts.looks.is_some() {
            return Err(out_err(
                &format!("{at}.{k}"),
                "a stroke has only `paint`, `width`, `join`, `cap`, `miterLimit`, `opacity`, `blend` and `effects`",
            ));
        }
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
    let out = OutStroke { paint, width: number("width", 0.0..=MAX_STROKE_WIDTH)?, join, cap, miter_limit: number("miterLimit", MITER_LIMIT)? };
    Ok((out, look(s, at, true, counts)?))
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
/// stops, opacities, midpoints and placements in range, and a gradient stop's `plates` only on a
/// stop linked to a swatch ([`check_references`] checks that the swatch is a spot swatch).
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
                // Plates print beside the tint of the spot swatch the stop itself links to; a
                // gradient's own `swatch` does not link its stops.
                if s.plates.is_some() && s.swatch.is_none() {
                    return Err(out_err(&format!("{at}.plates"), "`plates` requires a stop linked to a spot swatch"));
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
/// swatch of `doc`, that a gradient stop with `plates` links to a spot swatch, and every pattern a
/// pattern paint names is one of its patterns. Without a document (`None`), any swatch or pattern a
/// piece names is refused.
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
                g.gradient.stops.iter().enumerate().try_for_each(|(i, s)| {
                    let at = format!("{at}.gradient.stops[{i}]");
                    swatch(&s.swatch, &format!("{at}.swatch"))?;
                    match s.swatch.as_deref().and_then(|n| doc.and_then(|d| d.swatch(n))) {
                        Some(w) if s.plates.is_some() && !w.spot => {
                            Err(out_err(&format!("{at}.plates"), "`plates` requires a stop linked to a spot swatch"))
                        }
                        _ => Ok(()),
                    }
                })
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

/// How many paints, gradient stops and freeform points `pieces` hold, and one for each effect
/// record they carry and for each 50 bytes of its parameters: what a cache counts them as besides
/// their anchors.
pub(crate) fn paint_count(pieces: &[Piece]) -> usize {
    let one = |p: &Paint| match p {
        Paint::Gradient(g) => 1 + g.gradient.stops.len() + g.freeform.as_ref().map_or(0, |f| f.points.len()),
        _ => 1,
    };
    let effects = |l: &Look| l.effects.iter().map(|e| 1 + e.params.to_string().len() / 50).sum::<usize>();
    let looks = |p: &Piece| effects(&p.look) + p.fill_looks.iter().chain(&p.stroke_looks).map(effects).sum::<usize>();
    pieces
        .iter()
        .map(|p| p.fills.iter().flatten().map(one).sum::<usize>() + p.strokes.iter().flatten().map(|s| one(&s.paint)).sum::<usize>() + looks(p))
        .sum()
}

/// The appearance piece `p` of an object-level effect is drawn with: `base` (the object's) with
/// its fills, then its strokes, replaced by the piece's where it has them, as a filter's output
/// sets them (an existing item keeps everything but what the piece gives; new ones take the
/// defaults), each given its look ([`Look`]). The piece's own look is not part of it: the art the
/// piece becomes carries it.
pub fn appearance(base: &Appearance, p: &Piece) -> Appearance {
    let mut ap = base.clone();
    if let Some(fills) = &p.fills {
        crate::objects::set_items(&mut ap, true, fills.iter().cloned().map(OutStroke::paint).collect());
        dress_items(&mut ap, true, &p.fill_looks);
    }
    if let Some(strokes) = &p.strokes {
        crate::objects::set_items(&mut ap, false, strokes.clone());
        dress_items(&mut ap, false, &p.stroke_looks);
    }
    ap
}

/// Gives the fills (`fill`) or strokes of `ap`, in paint order, the looks in `looks`.
fn dress_items(ap: &mut Appearance, fill: bool, looks: &[Look]) {
    for (item, look) in ap.items.iter_mut().filter(|i| i.is_fill() == fill).zip(looks) {
        look.dress(item);
    }
}

/// The items piece `p` of an effect on `item` (a fill or a stroke) is drawn with, in paint order:
/// `item` itself when the piece keeps it, else one copy of it for each paint or stroke the piece
/// gives, with that paint (and that stroke's weight, join, cap and miter limit where given) and
/// that fill's or stroke's look.
pub fn items(item: &AppearanceItem, p: &Piece) -> Vec<AppearanceItem> {
    let dressed = |mut it: AppearanceItem, look: Option<&Look>| {
        if let Some(look) = look {
            look.dress(&mut it);
        }
        it
    };
    match item {
        AppearanceItem::Fill(f) => match &p.fills {
            None => vec![item.clone()],
            Some(fills) => fills
                .iter()
                .enumerate()
                .map(|(k, paint)| {
                    dressed(AppearanceItem::Fill(vectorcraft_doc::FillLayer { paint: paint.clone(), ..f.clone() }), p.fill_looks.get(k))
                })
                .collect(),
        },
        AppearanceItem::Stroke(s) => match &p.strokes {
            None => vec![item.clone()],
            Some(strokes) => strokes
                .iter()
                .enumerate()
                .map(|(k, o)| {
                    let mut s = s.clone();
                    crate::objects::style(&mut s, o.clone());
                    dressed(AppearanceItem::Stroke(s), p.stroke_looks.get(k))
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
