//! Art styles (`%AI9_BeginArtStyles`; not publicly documented): the appearance an object's
//! `1 (style) XW` names, a tree of filters. A compound filter (`/CompoundFilter :`) holds the
//! filters after it until the `;` that closes it; a basic one (`/BasicFilter :`) ends at its
//! `/Part ,`:
//!
//! ```text
//! /KnownStyle :
//! (name) /Name ,
//! /ActiveStyle :
//! /CompoundFilter :
//! (Chain Style Filter) 0 0 /Filter ,      the object: its effects, its stack, its transparency
//! 1 /Visible ,
//! /BasicFilter :
//! (Adobe Deform) 1 0 /Filter ,            an effect on the whole object
//! … /Part ,
//! /CompoundFilter :
//! (Stack Style Filter) 0 0 /Filter ,      its fills and strokes, the top one first
//! … (Stroke Style Filter) … /Part ,       a stroke with no effects of its own
//! /CompoundFilter :
//! (Chain Style Filter) 0 0 /Filter ,      a fill with effects and a transparency of its own
//! … (Adobe Offset Path) … (Fill Style Filter) … (Adobe Transform) … (Blend Style Filter) …
//! ;
//!  /Part ,
//! … (Conduit Filter) …                    the object's contents (type's characters, a group's members)
//! ;
//!  /Part ,
//! (Blend Style Filter) …                  the object's own transparency (written on its look too)
//! ;
//! ```
//!
//! A fill's style paints with a colour (`c m y k r g b Xa`), a pattern (`(name) … [matrix] p`) or a
//! gradient (`Bb` … `flag (name) x y angle length … Bg` … `BB`); `stroke fill Xd` says whether it
//! paints at all. A blend filter's `mode opacity isolate knockout shape Xy` is its transparency.
//!
//! A style is *plain* when every filter in it is one read here: fills and strokes (in a colour; fills
//! with a pattern or gradient too), each with its own transparency and effects, and the effects of
//! [`effect`]. Such objects are read as themselves with an appearance of those fills, strokes and
//! effects; others keep their drawn look. Effect › Pathfinder effects (`(Adobe Pathfinder)` in the
//! object's chain) are read on a group with no fills or strokes of its own, which keeps its members.

use std::collections::HashMap;

use serde_json::json;
use vectorcraft_color::{BlendMode, Paint};
use vectorcraft_doc::{Dash, Effect, LineCap, LineJoin, WidthProfile};
use vectorcraft_effects::is_pathfinder;

use super::lex::{Lexer, Tok};
use super::obj::V;
use super::paint;

const GLOW: &str = "Adobe Outer Glow"; // brand-ok: the filter's name in the files
const FEATHER: &str = "Adobe Feather"; // brand-ok: the filter's name in the files
const SHADOW: &str = "Adobe Drop Shadow"; // brand-ok: the filter's name in the files
const DEFORM: &str = "Adobe Deform"; // brand-ok: the filter's name in the files
const OFFSET: &str = "Adobe Offset Path"; // brand-ok: the filter's name in the files
const TRANSFORM: &str = "Adobe Transform"; // brand-ok: the filter's name in the files
const GAUSSIAN: &str = "Adobe PSL Gaussian Blur"; // brand-ok: the filter's name in the files
const BAS_RELIEF: &str = "PSAdapter_plugin_BsRl";
const PATHFINDER: &str = "Adobe Pathfinder"; // brand-ok: the filter's name in the files

const CHAIN: &str = "Chain Style Filter";
const STACK: &str = "Stack Style Filter";
const FILL: &str = "Fill Style Filter";
const STROKE: &str = "Stroke Style Filter";
const CONDUIT: &str = "Conduit Filter";
const BLEND: &str = "Blend Style Filter";

/// The warp styles by their number in `DeformStyle` (Effect › Warp's list, from 1).
const WARPS: [&str; 15] = [
    "warp.arc",
    "warp.arcLower",
    "warp.arcUpper",
    "warp.arch",
    "warp.bulge",
    "warp.shellLower",
    "warp.shellUpper",
    "warp.flag",
    "warp.wave",
    "warp.fish",
    "warp.rise",
    "warp.fisheye",
    "warp.inflate",
    "warp.squeeze",
    "warp.twist",
];

/// The Effect › Pathfinder effects by their number in `Command`, from 0 (the numbers a public
/// scripting example lists: <https://raw.githubusercontent.com/mark1bean/live-effect-functions-for-illustrator/aa727c40b84ad8ecb5a747d14b62ecd5b3fb1a50/scripts/LE_Functions.js>).
/// Trap (12) is not read here.
const PATHFINDER_IDS: [&str; 12] = [
    "pathfinder.add",
    "pathfinder.intersect",
    "pathfinder.exclude",
    "pathfinder.subtract",
    "pathfinder.minusBack",
    "pathfinder.divide",
    "pathfinder.outline",
    "pathfinder.trim",
    "pathfinder.merge",
    "pathfinder.crop",
    "pathfinder.hardMix",
    "pathfinder.softMix",
];

/// `Command`'s number for Soft Mix, the one effect whose `Mix` is its own setting.
const SOFT_MIX: usize = 11;

/// The Pathfinder Options and Trap values an Effect › Pathfinder filter must hold, each with its
/// type and the one value read: the effects here have no such settings of their own.
const PATHFINDER_OPTIONS: [(&str, ParamKind, f64); 11] = [
    ("ConvertCustom", ParamKind::Bool, 1.0),
    ("ExtractUnpainted", ParamKind::Bool, 1.0),
    ("Precision", ParamKind::Real, 10.0),
    ("RemovePoints", ParamKind::Bool, 0.0),
    ("TrapAspect", ParamKind::Real, 1.0),
    ("TrapConvertCustom", ParamKind::Bool, 0.0),
    ("TrapMaxTint", ParamKind::Real, 1.0),
    ("TrapReverse", ParamKind::Bool, 0.0),
    ("TrapThickness", ParamKind::Real, 0.25),
    ("TrapTint", ParamKind::Real, 0.4),
    ("TrapTintTolerance", ParamKind::Real, 0.05),
];

/// The `Mix` every Pathfinder effect but Soft Mix holds.
const PATHFINDER_MIX: f64 = 0.5;

/// The warp style (`arc`, `arcLower`, …) numbered `n` in `DeformStyle`.
pub(super) fn warp_style(n: i64) -> Option<&'static str> {
    let id = WARPS.get(usize::try_from(n.checked_sub(1)?).ok()?)?;
    id.strip_prefix("warp.")
}

/// A plain style's paint, as the object comes in.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct ArtStyle {
    /// The style reads as [`Self::items`] and [`Self::effects`].
    pub plain: bool,
    /// It is one fill and one stroke at most, in solid colours or the object's own gradient, with
    /// no transparency or effects of their own, and only Outer Glow, Feather and Drop Shadow on the
    /// object (and Pathfinder effects when it has no fills or strokes): what any object is read with. Type takes the other plain styles too (its drawn look
    /// can't be retyped); other objects keep their drawn look, which is the app's own rendering.
    pub simple: bool,
    /// Its fills and strokes, bottom first.
    pub items: Vec<StyleItem>,
    /// Each item's own visibility, transparency and effects, in the order of [`Self::items`].
    pub looks: Vec<ItemLook>,
    /// The object's live effects, in the order they apply.
    pub effects: Vec<Effect>,
    /// How many of [`Self::items`] paint below the object's contents (`None`: none do).
    pub contents_index: Option<usize>,
    /// [`Self::effects`] holds Effect › Pathfinder effects, which only a group is read with.
    pub pathfinder: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) enum StyleItem {
    Fill(Paint),
    /// A fill with a gradient: the swatch `name` at `angle` degrees, fitted to the object (the
    /// object's own gradient when it is the style's only one).
    Gradient {
        name: Option<String>,
        angle: f64,
    },
    /// A fill with pattern swatch `name`, its space taken to art space by `matrix`.
    Pattern {
        name: String,
        matrix: Option<[f64; 6]>,
    },
    Stroke {
        paint: Paint,
        width: f64,
        cap: LineCap,
        join: LineJoin,
        miter: f64,
        dash: Option<Dash>,
        profile: Option<WidthProfile>,
    },
}

/// A fill's or stroke's own state in the stack.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct ItemLook {
    pub visible: bool,
    pub opacity: f32,
    pub blend: BlendMode,
    pub effects: Vec<Effect>,
}

impl Default for ItemLook {
    fn default() -> Self {
        Self { visible: true, opacity: 1.0, blend: BlendMode::Normal, effects: vec![] }
    }
}

/// The type a dictionary value is written with (`/Int`, `/Real`, `/Bool`).
#[derive(Clone, Copy, Debug, PartialEq)]
enum ParamKind {
    Int,
    Real,
    Bool,
}

#[derive(Debug)]
struct Filter {
    name: String,
    visible: bool,
    /// The compound filter it is in (`None`: the style's first).
    parent: Option<usize>,
    compound: bool,
    params: HashMap<String, f64>,
    /// The type each of [`Self::params`] is written with.
    kinds: HashMap<String, ParamKind>,
    /// The keys of its dictionary's strings (`(text) /String (key) ,`).
    texts: Vec<String>,
    /// Its dictionary has a key twice, a key without a number, or more than [`MAX_TEXT_KEYS`]
    /// string keys.
    malformed: bool,
    /// Its dictionary's arrays (`/Array : v /Real , … ; (key) ,`), by key.
    arrays: HashMap<String, Vec<f64>>,
    fill: Option<Paint>,
    stroke: Option<Paint>,
    width: f64,
    cap: LineCap,
    join: LineJoin,
    miter: f64,
    dash: Option<Dash>,
    /// `stroke fill Xd`.
    paints: (bool, bool),
    gradient: bool,
    /// The gradient's swatch and angle (`Bg`).
    gradient_of: Option<(Option<String>, f64)>,
    /// `(name) … [matrix] p`.
    pattern: Option<(String, Option<[f64; 6]>)>,
    /// `mode opacity … Xy`.
    xy: Option<(usize, f32)>,
}

impl Default for Filter {
    fn default() -> Self {
        Self {
            name: String::new(),
            visible: true,
            parent: None,
            compound: false,
            params: HashMap::new(),
            kinds: HashMap::new(),
            texts: vec![],
            malformed: false,
            arrays: HashMap::new(),
            fill: None,
            stroke: None,
            width: 1.0,
            cap: LineCap::Butt,
            join: LineJoin::Miter,
            miter: 10.0,
            dash: None,
            paints: (false, false),
            gradient: false,
            gradient_of: None,
            pattern: None,
            xy: None,
        }
    }
}

/// Most filters read in one style.
const MAX_FILTERS: usize = 4096;

/// Most string keys kept for one filter.
const MAX_TEXT_KEYS: usize = 16;

/// The styles of the section `data` (between its markers), by name.
pub(super) fn art_styles(data: &[u8]) -> HashMap<String, ArtStyle> {
    let mut styles = HashMap::new();
    let mut name: Option<String> = None;
    let mut filters: Vec<Filter> = vec![];
    let mut operands: Vec<V> = vec![];
    // The compound filters not yet closed, the kind the next `/Filter` is, and whether a basic
    // filter's dictionaries are being read (their `;` close those, not a compound filter).
    let mut open: Vec<usize> = vec![];
    let mut pending: Option<bool> = None;
    let mut in_basic = false;
    let mut finish = |name: Option<String>, filters: &mut Vec<Filter>| {
        if let Some(name) = name {
            styles.insert(name, style(filters));
        }
        filters.clear();
    };
    let mut array: Option<Vec<V>> = None;
    let mut dict_array: Option<Vec<f64>> = None;
    // The last token was `/String` or `/UnicodeString`: a string key may follow.
    let mut text_key = false;
    let mut lex = Lexer::new(data);
    while let Some(t) = lex.next_token() {
        let after_text = std::mem::take(&mut text_key);
        if let (Some(a), Tok::Num(v)) = (array.as_mut(), &t.tok) {
            a.push(V::Num(*v));
            continue;
        }
        match t.tok {
            Tok::Num(v) => operands.push(V::Num(v)),
            Tok::Str(s) | Tok::Hex(s) => {
                if after_text && let Some(f) = filters.last_mut() {
                    let key = String::from_utf8_lossy(&s).into_owned();
                    // Past MAX_TEXT_KEYS a filter isn't one read here, and the keys aren't kept.
                    f.malformed |= f.texts.len() >= MAX_TEXT_KEYS || f.texts.contains(&key);
                    if f.texts.len() < MAX_TEXT_KEYS {
                        f.texts.push(key);
                    }
                }
                operands.push(V::Str(s));
            }
            Tok::Name(n) => match n {
                b"KnownStyle" => {
                    finish(name.take(), &mut filters);
                    operands.clear();
                    open.clear();
                    (pending, in_basic) = (None, false);
                }
                b"Name" if name.is_none() && filters.is_empty() => name = operands.iter().rev().find_map(V::text),
                b"CompoundFilter" => pending = Some(true),
                b"BasicFilter" => pending = Some(false),
                b"Part" => in_basic = false,
                b"Filter" => {
                    let compound = pending.take().unwrap_or(false);
                    if filters.len() < MAX_FILTERS {
                        filters.push(Filter {
                            name: operands.iter().rev().find_map(V::text).unwrap_or_default(),
                            parent: open.last().copied(),
                            compound,
                            ..Filter::default()
                        });
                        if compound {
                            open.push(filters.len() - 1);
                        }
                    }
                    in_basic = !compound;
                }
                b"Visible" => {
                    if let Some(f) = filters.last_mut() {
                        f.visible = operands.iter().rev().find_map(V::num) != Some(0.0);
                    }
                }
                b"Array" => dict_array = Some(vec![]),
                b"String" | b"UnicodeString" => text_key = true,
                b"Int" | b"Real" | b"Bool" => {
                    let kind = match n {
                        b"Int" => ParamKind::Int,
                        b"Real" => ParamKind::Real,
                        _ => ParamKind::Bool,
                    };
                    // The number right before the type: an earlier one isn't this value.
                    let value = operands.last().and_then(V::num);
                    operands.clear();
                    if let Some(a) = dict_array.as_mut() {
                        a.extend(value);
                        continue;
                    }
                    // The key follows: `value /Real (key) ,`.
                    if let (Some(Tok::Str(key)), Some(f)) = (lex.next_token().map(|t| t.tok), filters.last_mut()) {
                        let key = String::from_utf8_lossy(&key).into_owned();
                        match value {
                            Some(value) => {
                                f.malformed |= f.params.insert(key.clone(), value).is_some();
                                f.kinds.insert(key, kind);
                            }
                            None => f.malformed = true,
                        }
                    }
                    continue;
                }
                _ => {}
            },
            Tok::Word(w) => {
                let op = std::str::from_utf8(w).unwrap_or_default();
                if op == "[" {
                    array = Some(vec![]);
                    continue;
                }
                if op == "]" {
                    let items = array.take().unwrap_or_default();
                    operands.push(V::Arr(items));
                    continue;
                }
                if op == ";"
                    && let Some(values) = dict_array.take()
                {
                    if let (Some(Tok::Str(key)), Some(f)) = (lex.next_token().map(|t| t.tok), filters.last_mut()) {
                        f.arrays.insert(String::from_utf8_lossy(&key).into_owned(), values);
                    }
                    operands.clear();
                    continue;
                }
                if op == ";" && !in_basic {
                    open.pop();
                }
                if let Some(f) = filters.last_mut() {
                    paint_op(f, op, &operands);
                }
                if op == "," || op == ";" || op.starts_with('X') || op.starts_with('B') || op.len() <= 2 {
                    operands.clear();
                }
                continue;
            }
            Tok::Comment(_) | Tok::Data(_) | Tok::Preview(_) => {}
        }
        if operands.len() > 64 {
            operands.clear();
        }
    }
    finish(name, &mut filters);
    styles
}

/// A paint operator of a fill's or stroke's style.
fn paint_op(f: &mut Filter, op: &str, vals: &[V]) {
    let last = vals.iter().rev().find_map(V::num);
    match op {
        "Xa" | "k" | "g" | "x" | "Xk" | "XA" | "K" | "G" | "X" | "XK" => {
            let Some((p, _)) = paint::color_op(op, vals) else { return };
            if op.ends_with(|c: char| c.is_ascii_lowercase()) {
                f.fill = Some(p);
            } else {
                f.stroke = Some(p);
            }
        }
        "w" => f.width = last.filter(|v| v.is_finite() && *v >= 0.0).unwrap_or(f.width),
        "J" => {
            f.cap = match last.map(|v| v as i64) {
                Some(1) => LineCap::Round,
                Some(2) => LineCap::Square,
                _ => LineCap::Butt,
            }
        }
        "j" => {
            f.join = match last.map(|v| v as i64) {
                Some(1) => LineJoin::Round,
                Some(2) => LineJoin::Bevel,
                _ => LineJoin::Miter,
            }
        }
        "M" => f.miter = last.filter(|v| v.is_finite() && *v >= 1.0).unwrap_or(f.miter).min(500.0),
        "d" => {
            let pattern: Vec<f64> = vals
                .iter()
                .rev()
                .find_map(|v| if let V::Arr(a) = v { Some(a.iter().filter_map(V::num).collect()) } else { None })
                .unwrap_or_default();
            f.dash = (!pattern.is_empty() && pattern.len() <= 64 && pattern.iter().all(|v| v.is_finite() && *v >= 0.0)).then(|| Dash {
                pattern,
                offset: last.filter(|v| v.is_finite()).unwrap_or(0.0),
                ..Dash::default()
            });
        }
        "Xd" => {
            let n: Vec<f64> = vals.iter().filter_map(V::num).collect();
            if let [.., stroke, fill] = n.as_slice() {
                f.paints = (*stroke != 0.0, *fill != 0.0);
            }
        }
        "Xy" => {
            let n: Vec<f64> = vals.iter().filter_map(V::num).collect();
            if let [mode, opacity, ..] = n.as_slice()
                && mode.is_finite()
                && opacity.is_finite()
            {
                f.xy = Some((mode.max(0.0) as usize, opacity.clamp(0.0, 1.0) as f32));
            }
        }
        // `flag (name) x y angle length a b c d tx ty Bg`.
        "Bg" => {
            f.gradient = true;
            let at = vals.iter().rposition(|v| matches!(v, V::Str(_)));
            let name = at.and_then(|i| vals.get(i)).and_then(V::text);
            let n: Vec<f64> = at.map_or(&[][..], |i| vals.get(i + 1..).unwrap_or_default()).iter().filter_map(V::num).collect();
            let angle = n.get(2).copied().filter(|a| a.is_finite()).unwrap_or(0.0);
            f.gradient_of = Some((name, angle));
        }
        "p" => {
            let name = vals.iter().find_map(|v| if let V::Str(_) = v { v.text() } else { None });
            let matrix = vals.iter().rev().find_map(|v| match v {
                V::Arr(a) => {
                    let n: Vec<f64> = a.iter().filter_map(V::num).collect();
                    <[f64; 6]>::try_from(n.as_slice()).ok().filter(|m| m.iter().all(|v| v.is_finite()))
                }
                _ => None,
            });
            if let Some(name) = name {
                f.pattern = Some((name, matrix));
            }
        }
        _ => {}
    }
}

/// A variable-width stroke's profile: its widths (fractions of the stroke's) at positions along the
/// path, the same both sides unless it gives each side's.
fn width_profile(arrays: &HashMap<String, Vec<f64>>) -> Option<WidthProfile> {
    let at = arrays.get("BSVariableWidthTValuesArray")?;
    let side = |key: &str| arrays.get(key).filter(|w| w.len() == at.len());
    let (left, right) = match (side("BSLeftWidthsArray"), side("BSRightWidthsArray")) {
        (Some(left), Some(right)) => (left, right),
        _ => (side("BSCenteredWidthsArray")?, side("BSCenteredWidthsArray")?),
    };
    if at.len() > 1024 {
        return None;
    }
    // The stroker expects the positions sorted over 0..=1 and no negative widths.
    let mut points: Vec<(f64, f64, f64)> = at
        .iter()
        .zip(left)
        .zip(right)
        .filter(|((t, l), r)| t.is_finite() && l.is_finite() && r.is_finite())
        .map(|((t, l), r)| (t.clamp(0.0, 1.0), l.max(0.0), r.max(0.0)))
        .collect();
    points.sort_by(|a, b| a.0.total_cmp(&b.0));
    let uniform = points.iter().all(|(_, l, r)| (l - 1.0).abs() < 1e-9 && (r - 1.0).abs() < 1e-9);
    (!uniform).then_some(WidthProfile { points })
}

/// The live effect filter `f` is, when it is one read here.
fn effect(f: &Filter) -> Option<Effect> {
    let param = |k: &str| f.params.get(k).copied().filter(|v| v.is_finite());
    let flag = |k: &str| param(k).is_some_and(|v| v != 0.0);
    let mode = |default: usize| super::read::blend_name(param("blnd").map_or(default, |v| v.max(0.0) as usize));
    let color = f.fill.as_ref().and_then(Paint::color).unwrap_or(vectorcraft_color::Color::BLACK);
    let (id, params) = match f.name.as_str() {
        // `blur`'s scale isn't publicly documented: read as the Gaussian's σ, an effect's blur being 2σ.
        GLOW => (
            "stylize.outerGlow",
            json!({"mode": mode(2), "opacity": param("opac").unwrap_or(0.75) * 100.0, "blur": 2.0 * param("blur").unwrap_or(5.0), "color": color}),
        ),
        // LiveEffect XML uses `radius`; art styles abbreviate it `rad` (public scripting posts on applyEffect).
        FEATHER => ("stylize.feather", json!({"radius": param("rad").or_else(|| param("radius")).unwrap_or(5.0)})),
        SHADOW => {
            let mut params = json!({
                "mode": mode(1),
                "opacity": param("opac").unwrap_or(0.75) * 100.0,
                "blur": 2.0 * param("blur").unwrap_or(5.0),
                // Undocumented; read as the dialog's offsets, not flipped like art-space y: its default 7/7 pt is
                // right and down (https://helpx.adobe.com/illustrator/using/drop-shadows-glows-feathering.html).
                "x": param("horz").unwrap_or(7.0),
                "y": param("vert").unwrap_or(7.0),
                "color": color,
            });
            // A `csrc` of 1 is Darkness, the object's own colors with black added; any value other than 0 is read as
            // Darkness. `dark` is the percent of black (0 to 100, where `opac` is a fraction), and 100 when it is missing or not finite.
            if param("csrc").unwrap_or(0.0) != 0.0 {
                params["shade"] = json!("darkness");
                params["darkness"] = json!(param("dark").map_or(100.0, |v| v.clamp(0.0, 100.0)));
            }
            ("stylize.dropShadow", params)
        }
        // Effect › Warp: `DeformStyle` numbers the styles from 1 as the dialog lists them, `Rotate`
        // is Vertical, and the bend and distortions are fractions.
        DEFORM => {
            let style = param("DeformStyle").map_or(1, |v| v as i64);
            let id = WARPS.get(usize::try_from(style - 1).ok()?)?;
            let pct = |k: &str| (param(k).unwrap_or(0.0) * 100.0).clamp(-100.0, 100.0);
            (
                *id,
                json!({
                    "bend": pct("DeformValue"),
                    "horizontal": pct("DeformHoriz"),
                    "vertical": pct("DeformVert"),
                    "orientation": if flag("Rotate") { "vertical" } else { "horizontal" },
                }),
            )
        }
        // `jntp`: 0 round, 1 bevel, 2 miter (the joins LiveEffect XML writes in public scripts).
        OFFSET => (
            "path.offsetPath",
            json!({
                "offset": param("ofst").unwrap_or(10.0).clamp(-1e5, 1e5),
                "joins": match param("jntp").map(|v| v as i64) { Some(0) => "round", Some(1) => "bevel", _ => "miter" },
                "miterLimit": param("mlim").unwrap_or(4.0).clamp(1.0, 500.0),
            }),
        ),
        // Art space has y up: a move up is a negative `moveV` in the document.
        TRANSFORM => (
            "distort.transform",
            json!({
                "scaleH": param("scaleH_Percent").unwrap_or(100.0),
                "scaleV": param("scaleV_Percent").unwrap_or(100.0),
                "moveH": param("moveH_Pts").unwrap_or(0.0),
                "moveV": -param("moveV_Pts").unwrap_or(0.0),
                "rotate": param("rotate_Degrees").unwrap_or(0.0),
                "copies": param("numCopies").unwrap_or(0.0).clamp(0.0, 1000.0) as i64,
                "reflectX": flag("reflectX"),
                "reflectY": flag("reflectY"),
                "reference": param("pinPoint").unwrap_or(4.0).clamp(0.0, 8.0) as i64,
                "random": flag("randomize"),
            }),
        ),
        GAUSSIAN => ("blur.gaussian", json!({"radius": param("blur").unwrap_or(5.0).clamp(0.0, 1000.0)})),
        // Its settings are in a Photoshop plug-in's binary block: the effect's defaults stand in.
        BAS_RELIEF => ("sketch.basRelief", json!({"detail": 13.0, "smoothness": 3.0, "lightDirection": "bottom"})),
        PATHFINDER => return pathfinder_effect(f),
        _ => return None,
    };
    Some(Effect { id: id.into(), params, visible: f.visible })
}

/// The Effect › Pathfinder effect filter `f` is: `Command` (an `/Int` from 0 to 11) picks it, and
/// every option key is there once, with its type and the value [`PATHFINDER_OPTIONS`] reads. `Mix`
/// is a fraction: 0.5, or Soft Mix's rate from 0.01 to 1. `None` for anything else, which keeps the
/// drawn look.
fn pathfinder_effect(f: &Filter) -> Option<Effect> {
    if f.malformed || !f.arrays.is_empty() || f.texts.iter().any(|k| k != "DisplayString") {
        return None;
    }
    let typed = |key: &str, kind: ParamKind| f.params.get(key).copied().filter(|v| v.is_finite() && f.kinds.get(key) == Some(&kind));
    let command = typed("Command", ParamKind::Int).filter(|c| c.fract() == 0.0 && (0.0..=SOFT_MIX as f64).contains(c))? as usize;
    let id = PATHFINDER_IDS.get(command)?;
    let known = |k: &str| k == "Command" || k == "Mix" || PATHFINDER_OPTIONS.iter().any(|(o, ..)| *o == k);
    if f.params.len() != PATHFINDER_OPTIONS.len() + 2 || !f.params.keys().all(|k| known(k)) {
        return None;
    }
    if !PATHFINDER_OPTIONS.iter().all(|(key, kind, value)| typed(key, *kind) == Some(*value)) {
        return None;
    }
    let mix = typed("Mix", ParamKind::Real)?;
    let params = if command == SOFT_MIX {
        if !(0.01..=1.0).contains(&mix) {
            return None;
        }
        // In percent, to 2 decimal places: 0.3 is 30, not 30.000000000000004.
        json!({"mixingRate": (mix * 10_000.0).round() / 100.0})
    } else if mix == PATHFINDER_MIX {
        json!({})
    } else {
        return None;
    };
    Some(Effect { id: (*id).into(), params, visible: f.visible })
}

/// The fill or stroke basic filter `f` paints, `Ok(None)` when it paints nothing, `Err` when it
/// paints in a way this doesn't read.
fn paint_item(f: &Filter) -> Result<Option<StyleItem>, ()> {
    match f.name.as_str() {
        FILL if !f.paints.1 => Ok(None),
        FILL => match (&f.pattern, &f.gradient_of, &f.fill) {
            (Some((name, matrix)), ..) => Ok(Some(StyleItem::Pattern { name: name.clone(), matrix: *matrix })),
            (None, Some((name, angle)), _) => Ok(Some(StyleItem::Gradient { name: name.clone(), angle: *angle })),
            (None, None, _) if f.gradient => Ok(Some(StyleItem::Gradient { name: None, angle: 0.0 })),
            (None, None, Some(p)) => Ok(Some(StyleItem::Fill(p.clone()))),
            (None, None, None) => Err(()),
        },
        STROKE if !f.paints.0 => Ok(None),
        // Dashes aligned to corners and path ends draw as the look's pieces: a solid end at each end
        // and the pattern, fitted, between them.
        STROKE if f.params.get("BSDashAdj").is_some_and(|v| *v != 0.0) => Err(()),
        STROKE => match &f.stroke {
            Some(p) if !f.gradient && f.pattern.is_none() => Ok(Some(StyleItem::Stroke {
                paint: p.clone(),
                width: f.width,
                cap: f.cap,
                join: f.join,
                miter: f.miter,
                dash: f.dash.clone(),
                profile: width_profile(&f.arrays),
            })),
            _ => Err(()),
        },
        _ => Err(()),
    }
}

/// Builds an [`ArtStyle`] from a style's filter tree.
struct Builder<'a> {
    filters: &'a [Filter],
    plain: bool,
}

impl Builder<'_> {
    fn children(&self, parent: usize) -> Vec<usize> {
        self.filters.iter().enumerate().filter(|(_, f)| f.parent == Some(parent)).map(|(i, _)| i).collect()
    }

    /// The fill or stroke a stack holds as `id`: a basic one, or a chain of one with its effects
    /// and transparency.
    fn item(&mut self, id: usize) -> Option<(StyleItem, ItemLook)> {
        let filters = self.filters;
        let f = &filters[id];
        if !f.compound {
            return match paint_item(f) {
                Ok(item) => item.map(|i| (i, ItemLook { visible: f.visible, ..ItemLook::default() })),
                Err(()) => {
                    self.plain = false;
                    None
                }
            };
        }
        if f.name != CHAIN {
            self.plain = false;
            return None;
        }
        let mut look = ItemLook { visible: f.visible, ..ItemLook::default() };
        let mut painted: Option<StyleItem> = None;
        let mut paints = 0;
        for k in self.children(id) {
            let c = &filters[k];
            match c.name.as_str() {
                _ if c.compound => self.plain = false,
                FILL | STROKE => {
                    paints += 1;
                    look.visible &= c.visible;
                    match paint_item(c) {
                        Ok(item) => painted = item,
                        Err(()) => self.plain = false,
                    }
                }
                BLEND => {
                    if let Some((mode, opacity)) = c.xy {
                        (look.blend, look.opacity) = (super::read::blend_mode(mode), opacity);
                    }
                }
                // A fill's or stroke's own Pathfinder effect isn't read.
                _ => match effect(c) {
                    Some(e) if !is_pathfinder(&e.id) => look.effects.push(e),
                    _ => self.plain = false,
                },
            }
        }
        if paints != 1 {
            self.plain = false;
        }
        painted.map(|p| (p, look))
    }

    /// A stack's fills and strokes, bottom first, and how many of them paint below its contents.
    fn stack(&mut self, id: usize, st: &mut ArtStyle) {
        // Listed top first.
        let mut top_first = vec![];
        let mut contents_at = None;
        for k in self.children(id) {
            if self.filters[k].name == CONDUIT {
                contents_at = Some(top_first.len());
                continue;
            }
            if let Some(item) = self.item(k) {
                top_first.push(item);
            }
        }
        let below = contents_at.map(|at| top_first.len() - at);
        for (item, look) in top_first.into_iter().rev() {
            st.items.push(item);
            st.looks.push(look);
        }
        st.contents_index = below.filter(|n| *n > 0);
    }
}

fn style(filters: &[Filter]) -> ArtStyle {
    let mut st = ArtStyle { plain: true, ..ArtStyle::default() };
    let Some(root) = filters.iter().position(|f| f.parent.is_none()) else { return st };
    let mut b = Builder { filters, plain: true };
    let chained = filters[root].compound && filters[root].name == CHAIN;
    let top = if filters[root].compound { b.children(root) } else { vec![root] };
    for id in top {
        let f = &filters[id];
        match f.name.as_str() {
            STACK if f.compound => b.stack(id, &mut st),
            FILL | STROKE | CHAIN => {
                if let Some((item, look)) = b.item(id) {
                    st.items.push(item);
                    st.looks.push(look);
                }
            }
            CONDUIT => {}
            // The object's own transparency, which its look carries too.
            BLEND => {}
            // The object's Pathfinder effects are read only in its own chain.
            _ => match effect(f) {
                Some(e) if chained || !is_pathfinder(&e.id) => st.effects.push(e),
                _ => b.plain = false,
            },
        }
    }
    st.plain = b.plain;
    st.pathfinder = st.effects.iter().any(|e| is_pathfinder(&e.id));
    let count = |stroke: bool| st.items.iter().filter(|i| matches!(i, StyleItem::Stroke { .. }) == stroke).count();
    st.simple = st.plain
        && count(true) <= 1
        && count(false) <= 1
        && st.items.iter().all(|i| matches!(i, StyleItem::Fill(_) | StyleItem::Stroke { .. } | StyleItem::Gradient { .. }))
        && st.looks.iter().all(|l| l.visible && l.opacity >= 1.0 && l.blend == BlendMode::Normal && l.effects.is_empty())
        && st.effects.iter().all(|e| matches!(e.id.as_str(), "stylize.outerGlow" | "stylize.feather" | "stylize.dropShadow") || is_pathfinder(&e.id))
        // Pathfinder effects on a group with no fills or strokes of its own (`styled` checks the group).
        && (!st.pathfinder || st.items.is_empty());
    st
}

#[cfg(test)]
mod tests;
