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
use vectorcraft_doc::{Dash, Effect, Knockout, LineCap, LineJoin, WidthProfile};
use vectorcraft_effects::is_pathfinder;

use super::lex::{Lexer, Tok};
use super::obj::V;
use super::paint;

const GLOW: &str = "Adobe Outer Glow"; // brand-ok: the filter's name in the files
const INNER_GLOW: &str = "Adobe Inner Glow"; // brand-ok: the filter's name in the files
const FEATHER: &str = "Adobe Fuzzy Mask"; // brand-ok: the filter's name in the files
const SHADOW: &str = "Adobe Drop Shadow"; // brand-ok: the filter's name in the files
const ROUND_CORNERS: &str = "Adobe Round Corners"; // brand-ok: the filter's name in the files
const SCRIBBLE: &str = "Adobe Scribble Fill"; // brand-ok: the filter's name in the files
const EXPAND_VERSION: &str = "Adobe Effect Expand Before Version"; // brand-ok: the key's name in the files
const DEFORM: &str = "Adobe Deform"; // brand-ok: the filter's name in the files
const OFFSET: &str = "Adobe Offset Path"; // brand-ok: the filter's name in the files
/// Effect › Path › Outline Object.
const OUTLINE_TYPE: &str = "Adobe Outline Type"; // brand-ok: the filter's name in the files
const OUTLINE_STROKE: &str = "Adobe Outline Stroke"; // brand-ok: the filter's name in the files
const TRANSFORM: &str = "Adobe Transform"; // brand-ok: the filter's name in the files
const FREE_DISTORT: &str = "Adobe Free Distort"; // brand-ok: the filter's name in the files
const PUCKER_BLOAT: &str = "Adobe Punk and Bloat"; // brand-ok: the filter's name in the files
const TWEAK: &str = "Adobe Scribble and Tweak"; // brand-ok: the filter's name in the files
const TWIST: &str = "Adobe Twirl"; // brand-ok: the filter's name in the files
const ZIG_ZAG: &str = "Adobe Zigzag"; // brand-ok: the filter's name in the files
const GAUSSIAN: &str = "Adobe PSL Gaussian Blur"; // brand-ok: the filter's name in the files
const BAS_RELIEF: &str = "PSAdapter_plugin_BsRl";
const SHAPE_EFFECTS: &str = "Adobe Shape Effects"; // brand-ok: the filter's name in the files
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

/// The warp style (`arc`, `arcLower`, …) numbered `n` in `DeformStyle`.
pub(super) fn warp_style(n: i64) -> Option<&'static str> {
    let id = WARPS.get(usize::try_from(n.checked_sub(1)?).ok()?)?;
    id.strip_prefix("warp.")
}

/// The keys of an Effect › Pathfinder › Add filter's dictionary besides `DisplayString`, each with
/// the kind and the one value read: `Command` 0 is Add, and the rest are its Pathfinder Options,
/// Soft Mix and Trap settings
/// (https://community.adobe.com/questions-652/liveeffect-xml-effect-is-added-under-the-object-795137).
const PATHFINDER_ADD: [(&str, Kind, f64); 13] = [
    ("Command", Kind::Int, 0.0),
    ("ConvertCustom", Kind::Bool, 1.0),
    ("ExtractUnpainted", Kind::Bool, 1.0),
    ("Mix", Kind::Real, 0.5),
    ("Precision", Kind::Real, 10.0),
    ("RemovePoints", Kind::Bool, 0.0),
    ("TrapAspect", Kind::Real, 1.0),
    ("TrapConvertCustom", Kind::Bool, 0.0),
    ("TrapMaxTint", Kind::Real, 1.0),
    ("TrapReverse", Kind::Bool, 0.0),
    ("TrapThickness", Kind::Real, 0.25),
    ("TrapTint", Kind::Real, 0.4),
    ("TrapTintTolerance", Kind::Real, 0.05),
];

/// A plain style's paint, as the object comes in.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct ArtStyle {
    /// The style reads as [`Self::items`] and [`Self::effects`].
    pub plain: bool,
    /// It is one fill and one stroke at most, in solid colours or the object's own gradient, with
    /// no transparency or effects of their own, and only Drop Shadow, Inner Glow, Outer Glow,
    /// Feather, Round Corners, Scribble, Convert to Shape, the Path effects (Offset Path, Outline Object, Outline Stroke) and Bas Relief read from its settings block on the object
    /// (and Pathfinder effects when it has no fills or strokes): what any object is read with. Type takes the other plain styles too (its drawn look
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
    /// The object's own opacity, blend mode, isolation and knockout (its blend filter), when the
    /// style has them.
    pub transparency: Option<(f32, BlendMode, bool, Knockout)>,
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
    /// A fill with pattern swatch `name`, its space taken to art space by `matrix`; `controls`, the
    /// numbers before the matrix, apply first when the pattern was defined in the version 7 form.
    Pattern {
        name: String,
        matrix: Option<[f64; 6]>,
        controls: Vec<f64>,
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

/// How a filter's dictionary writes an entry: `value /Int (key) ,`, `/Real`, `/Bool`, a fill's
/// style (`/FillStyle : … /Def ; (key) ,`), or anything else.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Int,
    Real,
    Bool,
    Fill,
    Other,
}

#[derive(Debug)]
struct Filter {
    name: String,
    visible: bool,
    /// The compound filter it is in (`None`: the style's first).
    parent: Option<usize>,
    compound: bool,
    params: HashMap<String, f64>,
    /// How each key of its dictionary is written.
    kinds: HashMap<String, Kind>,
    /// A key of its dictionary is written more than once.
    repeated: bool,
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
    /// `(name) … [matrix] p`, and the numbers between the name and the matrix.
    pattern: Option<(String, Option<[f64; 6]>, Vec<f64>)>,
    /// `mode opacity … Xy`.
    xy: Option<(usize, f32)>,
    /// `… isolate knockout … Xy`.
    xy_group: Option<(bool, Knockout)>,
    /// Its settings block's bytes (`/Binary : /ASCII85Decode ,`), `Err` when it isn't read.
    data: Option<Result<Vec<u8>, ()>>,
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
            repeated: false,
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
            xy_group: None,
            data: None,
        }
    }
}

/// Most filters read in one style.
const MAX_FILTERS: usize = 4096;

/// Most entries read from a style list.
const MAX_LISTED: usize = 4096;
/// The longest style name read from a style list, in bytes.
const MAX_LISTED_NAME: usize = 1024;

/// The style list (`%AI9_BeginArtStyleList`, between its markers): one literal string per style,
/// the Graphic Styles panel's order. Each name is its string's bytes, escapes undone; other tokens,
/// a string the data ends inside and names past [`MAX_LISTED_NAME`] are left out.
pub(super) fn art_style_list(data: &[u8]) -> Vec<Vec<u8>> {
    let mut names = vec![];
    let mut lex = Lexer::new(data);
    loop {
        let before = lex.offset();
        if names.len() >= MAX_LISTED {
            break;
        }
        let Some(t) = lex.next_token() else { break };
        // The lexer stops a string at its closing `)` or at the end of the data.
        let closed = lex.offset() < data.len() || data.get(before..).is_some_and(closes);
        if let Tok::Str(s) = t.tok
            && closed
            && s.len() <= MAX_LISTED_NAME
        {
            names.push(s);
        }
    }
    names
}

/// Does the first literal string in `data` end with its own `)` (not an escaped one or one that
/// closes a nested pair)?
fn closes(data: &[u8]) -> bool {
    let Some(start) = data.iter().position(|b| *b == b'(') else { return false };
    let mut depth = 0usize;
    let mut bytes = data.iter().skip(start + 1);
    while let Some(b) = bytes.next() {
        match b {
            b'\\' => {
                bytes.next();
            }
            b'(' => depth += 1,
            b')' if depth == 0 => return true,
            b')' => depth -= 1,
            _ => {}
        }
    }
    false
}

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
    // A paint style being read as a dictionary entry: its key follows its `/Def ;`.
    let mut style_entry: Option<Kind> = None;
    // A name of a basic filter's that may be an entry's type, with the string that follows it.
    let mut other_entry: Option<Option<String>> = None;
    // The last name was `/Binary`.
    let mut binary = false;
    let mut lex = Lexer::new(data);
    while let Some(t) = lex.next_token() {
        if let (Some(a), Tok::Num(v)) = (array.as_mut(), &t.tok) {
            a.push(V::Num(*v));
            continue;
        }
        let after_binary = binary;
        if let Tok::Name(n) = &t.tok {
            binary = *n == b"Binary";
        }
        match t.tok {
            Tok::Num(v) => operands.push(V::Num(v)),
            Tok::Str(s) | Tok::Hex(s) => {
                if let Some(key @ None) = other_entry.as_mut() {
                    *key = Some(String::from_utf8_lossy(&s).into_owned());
                }
                operands.push(V::Str(s));
            }
            // A basic filter's settings block: its bytes are on the comment lines the lexer skips.
            Tok::Name(b"ASCII85Decode") if after_binary && in_basic => {
                let block = binary_block(&mut lex, data.len()).ok_or(());
                if let Some(f) = filters.last_mut() {
                    f.data = Some(if f.data.is_some() { Err(()) } else { block });
                }
                operands.clear();
            }
            Tok::Name(n) => match n {
                b"KnownStyle" => {
                    finish(name.take(), &mut filters);
                    operands.clear();
                    open.clear();
                    (pending, in_basic, style_entry, other_entry) = (None, false, None, None);
                }
                b"FillStyle" => style_entry = Some(Kind::Fill),
                b"StrokeStyle" | b"BlendStyle" => style_entry = Some(Kind::Other),
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
                b"Int" | b"Real" | b"Bool" => {
                    // The number right before the type: an earlier one isn't this value.
                    let value = operands.last().and_then(V::num);
                    operands.clear();
                    if let Some(a) = dict_array.as_mut() {
                        a.extend(value);
                        continue;
                    }
                    let kind = match n {
                        b"Int" => Kind::Int,
                        b"Real" => Kind::Real,
                        _ => Kind::Bool,
                    };
                    // The key follows: `value /Real (key) ,`.
                    if let Some(Tok::Str(key)) = lex.next_token().map(|t| t.tok)
                        && let Some(f) = filters.last_mut()
                    {
                        let key = String::from_utf8_lossy(&key).into_owned();
                        match value {
                            Some(value) => f.params.insert(key.clone(), value),
                            None => f.params.remove(&key),
                        };
                        f.repeated |= f.kinds.insert(key, kind).is_some();
                    }
                    continue;
                }
                // Perhaps an entry of another type (`(text) /String (key) ,`): its key, if any, is
                // the string before the `,`.
                _ if in_basic => other_entry = Some(None),
                _ => {}
            },
            Tok::Word(w) => {
                let entry = other_entry.take().flatten();
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
                if op == "," {
                    let key = match style_entry.take() {
                        Some(kind) => operands.iter().rev().find_map(V::text).map(|k| (k, kind)),
                        None => entry.map(|k| (k, Kind::Other)),
                    };
                    if let (Some((key, kind)), Some(f)) = (key, filters.last_mut()) {
                        f.repeated |= f.kinds.insert(key, kind).is_some();
                    }
                }
                if let Some(f) = filters.last_mut() {
                    paint_op(f, op, &operands);
                }
                if op == "," || op == ";" || op.starts_with('X') || op.starts_with('B') || op.len() <= 2 {
                    operands.clear();
                }
                continue;
            }
            Tok::Comment(_) | Tok::Data(_) | Tok::Preview(_) | Tok::Document(_) => {}
        }
        if operands.len() > 64 {
            operands.clear();
        }
    }
    finish(name, &mut filters);
    styles
}

/// The bytes of the settings block whose `/Binary : /ASCII85Decode` `lex` (over `data_len` bytes)
/// has just read: the rest of that line is its `,`, each line after it holds ASCII85 after a `%`
/// until `~>`, and `; (data) ,` follows. `lex` is moved past the `~>` when the lines up to it are
/// the block's and fit in [`adapter::MAX_ENCODED`] bytes, and past `; (data)` when they follow.
/// `None` when the block is malformed or decodes to more than [`adapter::MAX_DECODED`] bytes.
fn binary_block(lex: &mut Lexer, data_len: usize) -> Option<Vec<u8>> {
    let from = lex.offset();
    let text = lex.slice(from, from.saturating_add(adapter::MAX_ENCODED).min(data_len));
    let mut lines = text.split(|b| matches!(b, b'\r' | b'\n'));
    let first = lines.next()?;
    if first.trim_ascii() != b"," {
        return None;
    }
    // Where the line being read starts in `text`.
    let mut at = first.len() + 1;
    let mut encoded = vec![];
    for line in lines {
        let start = at;
        at += line.len() + 1;
        if line.is_empty() {
            continue;
        }
        // A line that isn't the block's ends it unread, the lexer left before it.
        let digits = line.strip_prefix(b"%")?;
        if let Some(i) = super::super::lex::find(digits, b"~>") {
            encoded.extend_from_slice(digits.get(..i + 2)?);
            lex.seek(from + start + 1 + i + 2);
            return block_data(lex, &encoded);
        }
        encoded.extend_from_slice(digits);
    }
    None
}

/// The bytes of a settings block's ASCII85 `encoded`, when `; (data)` follows it in `lex`. When it
/// doesn't, `lex` is left before those tokens, so the next style's `/KnownStyle` is still read.
fn block_data(lex: &mut Lexer, encoded: &[u8]) -> Option<Vec<u8>> {
    let at = lex.offset();
    let data = crate::ps::ascii85_decode_within(std::str::from_utf8(encoded).ok()?, adapter::MAX_DECODED)?;
    let (close, key) = (lex.next_token().map(|t| t.tok), lex.next_token().map(|t| t.tok));
    if close == Some(Tok::Word(b";")) && key == Some(Tok::Str(b"data".to_vec())) {
        return Some(data);
    }
    lex.seek(at);
    None
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
            if let [mode, opacity, rest @ ..] = n.as_slice()
                && mode.is_finite()
                && opacity.is_finite()
            {
                f.xy = Some((mode.max(0.0) as usize, opacity.clamp(0.0, 1.0) as f32));
                // As an object's own `Xy` reads them.
                let knockout = match rest.get(1).map(|v| *v as i64) {
                    Some(1) => Knockout::On,
                    Some(2) => Knockout::Neutral,
                    _ => Knockout::Off,
                };
                f.xy_group = Some((rest.first().is_some_and(|v| *v != 0.0), knockout));
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
            let controls: Vec<f64> =
                vals.iter().skip_while(|v| !matches!(v, V::Str(_))).skip(1).take_while(|v| !matches!(v, V::Arr(_))).filter_map(V::num).collect();
            if let Some(name) = name {
                f.pattern = Some((name, matrix, controls));
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

/// Entry `key` of `f`'s dictionary, written as `kind`: `Ok(None)` when the dictionary doesn't hold
/// it, `Err` when it holds it as another kind, without a finite number, or as an `/Int` that isn't
/// whole or a `/Bool` other than 0 and 1.
fn entry(f: &Filter, key: &str, kind: Kind) -> Result<Option<f64>, ()> {
    let Some(k) = f.kinds.get(key) else { return Ok(None) };
    match f.params.get(key) {
        Some(v)
            if *k == kind
                && v.is_finite()
                && match kind {
                    Kind::Real => true,
                    Kind::Int => v.fract() == 0.0,
                    Kind::Bool => *v == 0.0 || *v == 1.0,
                    Kind::Fill | Kind::Other => false,
                } =>
        {
            Ok(Some(*v))
        }
        _ => Err(()),
    }
}

/// Entry `key` as a switch, 0 or 1 written as `kind`: `Ok(None)` when it is missing.
fn switch(f: &Filter, key: &str, kind: Kind) -> Result<Option<bool>, ()> {
    match entry(f, key, kind)? {
        None => Ok(None),
        Some(0.0) => Ok(Some(false)),
        Some(1.0) => Ok(Some(true)),
        Some(_) => Err(()),
    }
}

/// Real entry `key` within `range`, `missing` when the dictionary doesn't hold it.
fn real(f: &Filter, key: &str, missing: f64, range: std::ops::RangeInclusive<f64>) -> Option<f64> {
    entry(f, key, Kind::Real).ok()?.map_or(Some(missing), |v| range.contains(&v).then_some(v))
}

/// What an effect in a style could change the paint of, for the Transform options whose value,
/// when the dictionary leaves it out, comes from the app's settings rather than the file.
#[derive(Clone, Copy, Debug)]
struct Under {
    /// A stroke, or other effects with distances that Scale Strokes & Effects would scale.
    strokes: bool,
    /// A pattern.
    patterns: bool,
}

/// Anything: an effect on the whole object, whose contents can paint in any way.
const ANYTHING: Under = Under { strokes: true, patterns: true };

/// The Transform effect `f` (the factor and radian entries; the percent and degree ones are what
/// its dialog shows). `None` keeps the drawn look: an entry not read, Random (the file holds no
/// seed), or a missing Scale Strokes & Effects or Transform Patterns that would change `under`.
fn transform(f: &Filter, under: Under) -> Option<serde_json::Value> {
    for key in ["scaleH_Percent", "scaleV_Percent", "rotate_Degrees"] {
        entry(f, key, Kind::Real).ok()?;
    }
    // The renderer's limits on scales and moves (`vectorcraft_effects`' `MAX_SCALE` and `MAX_MOVE`).
    let scale = |key: &str| real(f, key, 1.0, -1000.0..=1000.0).map(|v| v * 100.0);
    let (h, v) = (scale("scaleH_Factor")?, scale("scaleV_Factor")?);
    let (move_h, move_v) = (real(f, "moveH_Pts", 0.0, -1e7..=1e7)?, real(f, "moveV_Pts", 0.0, -1e7..=1e7)?);
    let rotate = Some(entry(f, "rotate_Radians", Kind::Real).ok()?.unwrap_or(0.0).to_degrees()).filter(|d| d.is_finite())?;
    let copies = entry(f, "numCopies", Kind::Int).ok()?.unwrap_or(0.0);
    let reference = entry(f, "pinPoint", Kind::Int).ok()?.unwrap_or(4.0);
    if !(0.0..=1000.0).contains(&copies) || !(0.0..=8.0).contains(&reference) {
        return None;
    }
    let on = |key: &str| switch(f, key, Kind::Bool);
    let (reflect_x, reflect_y) = (on("reflectX").ok()?.unwrap_or(false), on("reflectY").ok()?.unwrap_or(false));
    let objects = on("transformObjects").ok()?.unwrap_or(true);
    let (strokes, patterns) = (on("scaleLines").ok()?, on("transformPatterns").ok()?);
    if on("randomize").ok()?.unwrap_or(false) {
        return None;
    }
    let moves = h != 100.0 || v != 100.0 || move_h != 0.0 || move_v != 0.0 || rotate != 0.0 || reflect_x || reflect_y;
    let scales = ((h * v).abs().sqrt() - 100.0).abs() > 1e-9;
    if (strokes.is_none() && scales && under.strokes) || (patterns.is_none() && moves && under.patterns) {
        return None;
    }
    // Art space has y up: a move up is a negative `moveV` in the document.
    Some(json!({
        "scaleH": h,
        "scaleV": v,
        "moveH": move_h,
        "moveV": 0.0 - move_v,
        "rotate": rotate,
        "copies": copies as i64,
        "reflectX": reflect_x,
        "reflectY": reflect_y,
        "reference": reference as i64,
        "objects": objects,
        "patterns": patterns.unwrap_or(false),
        "strokes": strokes.unwrap_or(false),
    }))
}

/// Free Distort's corners: the destination corners `dst0h`… measured in the source frame `src0h`…
/// (top left, top right, bottom left, bottom right, y up), as the catalog's unit-box corners (top
/// left, top right, bottom right, bottom left, y down). The source frame is only the unit the
/// corners are measured in: the distorted box is laid over the object's own bounds, as the catalog
/// effect lays it, also when the frame is another size (a unit square, or bounds with the stroke).
/// No corners is no change; some but not all, or a frame that isn't an upright rectangle, isn't read.
fn free_distort(f: &Filter) -> Option<serde_json::Value> {
    let mut corners = [[[0.0; 2]; 4]; 2];
    let mut held = 0;
    for (which, c) in ["src", "dst"].into_iter().zip(&mut corners) {
        for (i, corner) in c.iter_mut().enumerate() {
            for (axis, v) in ["h", "v"].into_iter().zip(corner.iter_mut()) {
                if let Some(x) = entry(f, &format!("{which}{i}{axis}"), Kind::Real).ok()? {
                    *v = x;
                    held += 1;
                }
            }
        }
    }
    if held == 0 {
        return Some(json!({"corners": [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]]}));
    }
    let [[tl, tr, bl, br], dst] = corners;
    let (left, top) = (tl[0], tl[1]);
    let (width, height) = (tr[0] - left, top - bl[1]);
    let tolerance = 1e-5 * width.abs().max(height.abs());
    let upright = (tr[1] - top).abs() <= tolerance
        && (br[1] - bl[1]).abs() <= tolerance
        && (bl[0] - left).abs() <= tolerance
        && (br[0] - tr[0]).abs() <= tolerance;
    let extent = |d: f64| d.is_finite() && d > 0.0;
    if held < 16 || !upright || !extent(width) || !extent(height) {
        return None;
    }
    let mut out = vec![];
    for i in [0, 1, 3, 2] {
        let [h, v] = *dst.get(i)?;
        let (x, y) = ((h - left) / width, (top - v) / height);
        if !(x.is_finite() && y.is_finite() && x.abs() <= 1e4 && y.abs() <= 1e4) {
            return None;
        }
        out.push([x, y]);
    }
    Some(json!({ "corners": out }))
}

/// A `0` relative, `1` absolute switch (`absoluteness`), `absolute` when it is missing, and the size
/// it picks: `relative_key` in percent or `absolute_key` in points, 0 when missing.
fn sized(f: &Filter, absolute: bool, relative_key: &str, absolute_key: &str) -> Option<(bool, f64)> {
    let absolute = switch(f, "absoluteness", Kind::Real).ok()?.unwrap_or(absolute);
    let relative = real(f, relative_key, 0.0, 0.0..=1e4)?;
    let points = real(f, absolute_key, 0.0, 0.0..=1e4)?;
    Some((absolute, if absolute { points } else { relative }))
}

/// Corner (`roundness` 0, or missing) or Smooth (1).
fn points(f: &Filter) -> Option<&'static str> {
    Some(if switch(f, "roundness", Kind::Real).ok()?.unwrap_or(false) { "smooth" } else { "corner" })
}

/// The Tweak effect `f` (`Adobe Scribble and Tweak`): relative unless `absoluteness` says otherwise,
/// moving anchors and both handles unless the dictionary turns them off.
fn tweak(f: &Filter) -> Option<serde_json::Value> {
    let (absolute, h) = sized(f, false, "horz", "ahor")?;
    let (_, v) = sized(f, false, "vert", "aver")?;
    let on = |key: &str| switch(f, key, Kind::Bool).map(|v| v.unwrap_or(true));
    let (anchors, ins, outs) = (on("anch").ok()?, on("in").ok()?, on("out").ok()?);
    Some(json!({"h": h, "v": v, "relative": !absolute, "anchors": anchors, "in": ins, "out": outs}))
}

/// The Zig Zag effect `f`: absolute unless `absoluteness` says otherwise, 4 ridges and Corner points
/// when they are missing.
fn zig_zag(f: &Filter) -> Option<serde_json::Value> {
    let (absolute, size) = sized(f, true, "relAmount", "amount")?;
    let ridges = real(f, "ridges", 4.0, 0.0..=100.0).filter(|r| r.fract() == 0.0)?;
    Some(json!({"size": size, "relative": !absolute, "ridges": ridges as i64, "points": points(f)?}))
}

/// Whether Tweak params `p` leave the outline as it is: its random outline isn't in the file, so any
/// other keeps the drawn look.
fn still(p: &serde_json::Value) -> bool {
    (p["h"].as_f64() == Some(0.0) && p["v"].as_f64() == Some(0.0)) || ["anchors", "in", "out"].iter().all(|k| p[k] == json!(false))
}

/// A Stylize effect's dictionary, read only as it is written: each setting a key of the kind it
/// names, with a finite value in the effect's range.
struct Settings<'a>(&'a Filter);

impl Settings<'_> {
    /// Every key is one of `known`, or one of `fixed` holding the value it names.
    /// The choices (`blnd`, `csrc`, `gtyp`) are written as `/Int`, the colors as fill styles and
    /// the other settings as `/Real`.
    fn only(&self, known: &[&str], fixed: &[(&str, Kind, f64)]) -> bool {
        let f = self.0;
        let kind_of = |key: &str| match key {
            "blnd" | "csrc" | "gtyp" => Kind::Int,
            "sclr" | "gclr" => Kind::Fill,
            _ => Kind::Real,
        };
        f.arrays.is_empty()
            && f.kinds.iter().all(|(key, kind)| {
                (known.contains(&key.as_str()) && kind_of(key) == *kind)
                    || fixed.iter().any(|(k, want, v)| k == key && want == kind && f.params.get(key.as_str()) == Some(v))
            })
    }

    fn real(&self, key: &str, range: std::ops::RangeInclusive<f64>) -> Option<f64> {
        (self.0.kinds.get(key) == Some(&Kind::Real)).then_some(())?;
        self.0.params.get(key).copied().filter(|v| v.is_finite() && range.contains(v))
    }

    /// A choice numbered from 0 to `max`.
    fn choice(&self, key: &str, max: u8) -> Option<usize> {
        (self.0.kinds.get(key) == Some(&Kind::Int)).then_some(())?;
        let v = self.0.params.get(key).copied().filter(|v| v.is_finite() && v.fract() == 0.0 && (0.0..=f64::from(max)).contains(v))?;
        Some(v as usize)
    }

    /// The blend mode in `blnd`, numbered as transparency's (`Xy`).
    fn mode(&self) -> Option<String> {
        Some(super::read::blend_name(self.choice("blnd", 15)?))
    }

    /// The color of the fill style written as `key`: a solid color, in Normal at full opacity.
    fn color(&self, key: &str) -> Option<vectorcraft_color::Color> {
        let f = self.0;
        let normal = f.xy.is_none_or(|(mode, opacity)| mode == 0 && opacity >= 1.0);
        (f.kinds.get(key) == Some(&Kind::Fill) && !f.gradient && f.pattern.is_none() && normal).then_some(())?;
        f.fill.as_ref()?.color()
    }
}

/// Drop Shadow, Inner Glow, Outer Glow, Feather, Round Corners and Scribble (`None`: another
/// filter). `Some(None)` is one whose settings aren't all read: an unknown key, a key of another
/// kind, a value out of range or a missing setting keeps the drawn look. The keys are LiveEffect
/// XML's
/// (https://community.adobe.com/questions-652/pageitem-applyeffect-liveeffectxml-769450).
fn stylize(f: &Filter) -> Option<Option<(&'static str, serde_json::Value)>> {
    let s = Settings(f);
    let opacity = || Some(s.real("opac", 0.0..=1.0)? * 100.0);
    Some(match f.name.as_str() {
        // `horz` and `vert` are the X and Y Offset, right and down
        // (https://helpx.adobe.com/illustrator/desktop/special-effects-styles/apply-filter-effects/create-drop-shadows.html);
        // `csrc` 0 is Color (`sclr`), 1 Darkness (`dark`, a percent). `blur` is half the catalog's.
        SHADOW => (|| {
            let fixed = [("pair", Kind::Bool, 1.0), ("usePSLBlur", Kind::Bool, 1.0), (EXPAND_VERSION, Kind::Int, 16.0)];
            s.only(&["blnd", "opac", "horz", "vert", "blur", "csrc", "dark", "sclr"], &fixed).then_some(())?;
            let mut params = json!({
                "mode": s.mode()?,
                "opacity": opacity()?,
                "blur": 2.0 * s.real("blur", 0.0..=500.0)?,
                "x": s.real("horz", -1e4..=1e4)?,
                "y": s.real("vert", -1e4..=1e4)?,
            });
            if s.choice("csrc", 1)? == 0 {
                params["color"] = json!(s.color("sclr")?);
            } else {
                params["shade"] = json!("darkness");
                params["darkness"] = json!(s.real("dark", 0.0..=100.0)?);
                if let Some(c) = s.color("sclr") {
                    params["color"] = json!(c);
                }
            }
            Some(("stylize.dropShadow", params))
        })(),
        // `gtyp` 0 is Center, 1 Edge (https://helpx.adobe.com/illustrator/desktop/special-effects-styles/apply-filter-effects/create-an-inner-or-outer-glow.html);
        // `blur` is the catalog's.
        INNER_GLOW => (|| {
            s.only(&["blnd", "opac", "blur", "gtyp", "gclr"], &[]).then_some(())?;
            let source = if s.choice("gtyp", 1)? == 0 { "center" } else { "edge" };
            let params = json!({
                "mode": s.mode()?,
                "opacity": opacity()?,
                "blur": s.real("blur", 0.0..=1000.0)?,
                "color": s.color("gclr")?,
                "source": source,
            });
            Some(("stylize.innerGlow", params))
        })(),
        // `blur` is half the catalog's.
        GLOW => (|| {
            s.only(&["blnd", "opac", "blur", "sclr"], &[("usePSLBlur", Kind::Bool, 1.0), (EXPAND_VERSION, Kind::Int, 16.0)]).then_some(())?;
            let params = json!({"mode": s.mode()?, "opacity": opacity()?, "blur": 2.0 * s.real("blur", 0.0..=500.0)?, "color": s.color("sclr")?});
            Some(("stylize.outerGlow", params))
        })(),
        // https://helpx.adobe.com/illustrator/desktop/special-effects-styles/apply-filter-effects/feather-object-edges.html
        FEATHER => s.only(&["Radius"], &[]).then(|| s.real("Radius", 0.0..=1000.0)).flatten().map(|r| ("stylize.feather", json!({"radius": r}))),
        ROUND_CORNERS => {
            s.only(&["radius"], &[]).then(|| s.real("radius", 0.0..=1e4)).flatten().map(|r| ("stylize.roundCorners", json!({"radius": r})))
        }
        // `Angle` in degrees; widths, spacings and overlaps in points; `Scribbliness` (Curviness)
        // and `ScribbleVariation` as fractions. The catalog's Scribble has no color, so a Scribble
        // with `sclr` keeps the drawn look.
        SCRIBBLE => (|| {
            let keys =
                ["Angle", "EdgeOverlap", "EdgeOverlapVariation", "StrokeWidth", "Scribbliness", "ScribbleVariation", "Spacing", "SpacingVariation"];
            s.only(&keys, &[]).then_some(())?;
            let params = json!({
                "angle": s.real("Angle", -360.0..=360.0)?,
                "overlap": s.real("EdgeOverlap", -1000.0..=1000.0)?,
                "overlapVariation": s.real("EdgeOverlapVariation", 0.0..=100.0)?,
                "strokeWidth": s.real("StrokeWidth", 0.01..=1000.0)?,
                "curviness": s.real("Scribbliness", 0.0..=1.0)? * 100.0,
                "curvinessVariation": s.real("ScribbleVariation", 0.0..=1.0)? * 100.0,
                "spacing": s.real("Spacing", 0.1..=1000.0)?,
                "spacingVariation": s.real("SpacingVariation", 0.0..=100.0)?,
            });
            Some(("stylize.scribble", params))
        })(),
        _ => return None,
    })
}

/// The largest size or radius Convert to Shape reads, in points.
const MAX_SHAPE_SIZE: f64 = 1e5;

/// Convert to Shape: Rectangle, Rounded Rectangle or Ellipse, with an absolute size or one relative
/// to the object, and a corner radius
/// (https://helpx.adobe.com/illustrator/desktop/manage-objects/reshape-transform-objects/distort-and-transform-effects-in-illustrator.html).
/// The keys are LiveEffect XML's: `Shape` 0 Rectangle, 1 Rounded Rectangle, 2 Ellipse
/// (https://community.adobe.com/questions-652/effect-convert-to-shape-ellipse-in-javascript-761645),
/// `Absolute` 1 for an absolute `AbsWidth` × `AbsHeight`, else `RelWidth` and `RelHeight` added to
/// each side, and `CornerRadius`; a missing size is 18 pt a side or 36 pt, and a missing radius
/// 9 pt. `None`: a key, a kind or a value that isn't read, which keeps the drawn look.
fn shape_effect(f: &Filter) -> Option<Effect> {
    const SIZES: [&str; 5] = ["AbsWidth", "AbsHeight", "RelWidth", "RelHeight", "CornerRadius"];
    let known = f.kinds.iter().all(|(key, kind)| match key.as_str() {
        "DisplayString" => *kind == Kind::Other,
        "Shape" => *kind == Kind::Int,
        key => (key == "Absolute" || SIZES.contains(&key)) && *kind == Kind::Real,
    });
    if !known || !f.arrays.is_empty() || f.fill.is_some() || f.stroke.is_some() || f.gradient || f.pattern.is_some() {
        return None;
    }
    // A written size is a finite number from `min`; a missing one is `default`.
    let size = |key: &str, min: f64, default: f64| match f.kinds.get(key) {
        None => Some(default),
        Some(_) => f.params.get(key).copied().filter(|v| v.is_finite() && (min..=MAX_SHAPE_SIZE).contains(v)),
    };
    let shape = f.params.get("Shape").copied().filter(|_| f.kinds.get("Shape") == Some(&Kind::Int))?;
    let absolute = match f.params.get("Absolute").copied() {
        Some(0.0) => false,
        Some(1.0) => true,
        _ => return None,
    };
    let (rel_w, rel_h) = (size("RelWidth", -MAX_SHAPE_SIZE, 18.0)?, size("RelHeight", -MAX_SHAPE_SIZE, 18.0)?);
    let (abs_w, abs_h, radius) = (size("AbsWidth", 0.0, 36.0)?, size("AbsHeight", 0.0, 36.0)?, size("CornerRadius", 0.0, 9.0)?);
    let id = match shape {
        0.0 => "convertToShape.rectangle",
        1.0 => "convertToShape.roundedRectangle",
        // A relative Ellipse in the file is drawn round the enlarged box, and VectorCraft's inside
        // it: it keeps its drawn look.
        2.0 if absolute => "convertToShape.ellipse",
        _ => return None,
    };
    let mut params = json!({"relative": !absolute, "extraW": 2.0 * rel_w, "extraH": 2.0 * rel_h, "width": abs_w, "height": abs_h});
    if id == "convertToShape.roundedRectangle" {
        params["radius"] = json!(radius);
    }
    Some(Effect { id: id.into(), params, visible: f.visible })
}

/// The largest offset Offset Path reads, in points (the effect's own limit).
const MAX_OFFSET: f64 = 1e5;

/// Effect › Path: Offset Path with its Offset, Joins and Miter Limit
/// (https://helpx.adobe.com/illustrator/desktop/manage-objects/edit-objects/offset-duplicate-objects.html),
/// Outline Object and Outline Stroke, which have no settings. Offset Path's keys are LiveEffect
/// XML's: `ofst` in points, `jntp` 0 round, 1 bevel or 2 miter, and `mlim`
/// (https://community.adobe.com/questions-652/illustrator-javascript-count-area-with-offset-782386).
/// `None`: a missing, unknown or out-of-range setting, or one of another kind, which keeps the
/// drawn look; there is no default to stand in for a missing one.
fn path_effect(f: &Filter) -> Option<Effect> {
    if !f.arrays.is_empty() || f.fill.is_some() || f.stroke.is_some() || f.gradient || f.pattern.is_some() {
        return None;
    }
    let (id, params) = match f.name.as_str() {
        OFFSET => {
            const KEYS: [(&str, Kind); 3] = [("ofst", Kind::Real), ("jntp", Kind::Int), ("mlim", Kind::Real)];
            if f.kinds.len() != KEYS.len() || KEYS.iter().any(|(key, kind)| f.kinds.get(*key) != Some(kind)) {
                return None;
            }
            let value = |key: &str| f.params.get(key).copied().filter(|v| v.is_finite());
            let offset = value("ofst").filter(|v| v.abs() <= MAX_OFFSET)?;
            let miter = value("mlim").filter(|v| (1.0..=500.0).contains(v))?;
            let joins = match value("jntp")? {
                0.0 => "round",
                1.0 => "bevel",
                2.0 => "miter",
                _ => return None,
            };
            ("path.offsetPath", json!({"offset": offset, "joins": joins, "miterLimit": miter}))
        }
        // Outline Stroke without a `width`: the stroke it is on, or the object's, gives it.
        OUTLINE_STROKE if f.kinds.is_empty() => ("path.outlineStroke", json!({})),
        OUTLINE_TYPE if f.kinds.is_empty() => ("path.outlineObject", json!({})),
        _ => return None,
    };
    Some(Effect { id: id.into(), params, visible: f.visible })
}

/// The live effect filter `f` is, when it is one read here; `under` is what it could change the
/// paint of.
fn effect(f: &Filter, under: Under) -> Option<Effect> {
    if let Some(stylized) = stylize(f) {
        let (id, params) = stylized?;
        return Some(Effect { id: id.into(), params, visible: f.visible });
    }
    let param = |k: &str| f.params.get(k).copied().filter(|v| v.is_finite());
    let flag = |k: &str| param(k).is_some_and(|v| v != 0.0);
    let (id, params) = match f.name.as_str() {
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
        OFFSET | OUTLINE_TYPE | OUTLINE_STROKE => return path_effect(f),
        TRANSFORM => ("distort.transform", transform(f, under)?),
        FREE_DISTORT => ("distort.freeDistort", free_distort(f)?),
        // Negative puckers (https://helpx.adobe.com/illustrator/desktop/manage-objects/reshape-transform-objects/distort-and-transform-effects-in-illustrator.html).
        PUCKER_BLOAT => ("distort.puckerBloat", json!({"amount": real(f, "d_factor", 0.0, -200.0..=200.0)?})),
        TWEAK => ("distort.tweak", tweak(f).filter(still)?),
        // A positive angle turns clockwise in both.
        TWIST => ("distort.twist", json!({"angle": real(f, "angle", 0.0, -3600.0..=3600.0)?})),
        ZIG_ZAG => ("distort.zigZag", zig_zag(f)?),
        GAUSSIAN => ("blur.gaussian", json!({"radius": param("blur").unwrap_or(5.0).clamp(0.0, 1000.0)})),
        BAS_RELIEF => (
            "sketch.basRelief",
            match &f.data {
                // Without a settings block, the effect's defaults stand in.
                None => json!({"detail": 13.0, "smoothness": 3.0, "lightDirection": "bottom"}),
                Some(data) => adapter::bas_relief(&descriptor(f, data)?)?,
            },
        ),
        SHAPE_EFFECTS => return shape_effect(f),
        PATHFINDER => return pathfinder_effect(f),
        _ => return None,
    };
    Some(Effect { id: id.into(), params, visible: f.visible })
}

/// The fields of filter `f`'s settings block `data`, when it was read and its `dataSize`, written
/// as `/Int`, is its length.
fn descriptor(f: &Filter, data: &Result<Vec<u8>, ()>) -> Option<Vec<(adapter::Key, adapter::Value)>> {
    let data = data.as_ref().ok()?;
    (f.kinds.get("dataSize") == Some(&Kind::Int)).then_some(())?;
    let size = f.params.get("dataSize").copied()?;
    if size != data.len() as f64 {
        return None;
    }
    adapter::decode(data)
}

/// Effect › Pathfinder › Add, when every key of [`PATHFINDER_ADD`] is there with its kind and value
/// and the only other key is the `DisplayString` text, each written once. `None` for anything else,
/// which keeps the drawn look.
fn pathfinder_effect(f: &Filter) -> Option<Effect> {
    let known = f.kinds.iter().all(|(key, kind)| (key == "DisplayString" && *kind == Kind::Other) || PATHFINDER_ADD.iter().any(|(k, ..)| k == key));
    if !known || f.repeated || !f.arrays.is_empty() || f.fill.is_some() || f.stroke.is_some() || f.gradient || f.pattern.is_some() {
        return None;
    }
    if !PATHFINDER_ADD.iter().all(|(key, kind, value)| f.kinds.get(*key) == Some(kind) && f.params.get(*key) == Some(value)) {
        return None;
    }
    Some(Effect { id: "pathfinder.add".into(), params: json!({}), visible: f.visible })
}

/// The fill or stroke basic filter `f` paints, `Ok(None)` when it paints nothing, `Err` when it
/// paints in a way this doesn't read.
fn paint_item(f: &Filter) -> Result<Option<StyleItem>, ()> {
    match f.name.as_str() {
        FILL if !f.paints.1 => Ok(None),
        FILL => match (&f.pattern, &f.gradient_of, &f.fill) {
            (Some((name, matrix, controls)), ..) => Ok(Some(StyleItem::Pattern { name: name.clone(), matrix: *matrix, controls: controls.clone() })),
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
        let kids = self.children(id);
        // What this item's effects could change: its own paint, and its other effects' distances.
        let mut under = Under { strokes: false, patterns: false };
        let mut effects = 0;
        for &k in &kids {
            let c = &filters[k];
            match c.name.as_str() {
                _ if c.compound => {}
                FILL | STROKE => match paint_item(c) {
                    Ok(Some(StyleItem::Stroke { .. })) => under.strokes = true,
                    Ok(Some(StyleItem::Pattern { .. })) => under.patterns = true,
                    _ => {}
                },
                BLEND => {}
                _ => effects += 1,
            }
        }
        under.strokes |= effects > 1;
        for k in kids {
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
                // Convert to Shape on one fill or stroke isn't read yet: what it draws there is
                // the drawn look's. A fill's or stroke's own Pathfinder effect isn't read either.
                _ => match effect(c, under) {
                    Some(e) if !e.id.starts_with("convertToShape.") && !is_pathfinder(&e.id) => look.effects.push(e),
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
            BLEND => {
                if let Some((mode, opacity)) = f.xy {
                    let (isolate, knockout) = f.xy_group.unwrap_or((false, Knockout::Off));
                    st.transparency = Some((opacity, super::read::blend_mode(mode), isolate, knockout));
                }
            }
            // The object's Pathfinder effects are read only in its own chain.
            _ => match effect(f, ANYTHING) {
                Some(e) if chained || !is_pathfinder(&e.id) => st.effects.push(e),
                _ => b.plain = false,
            },
        }
    }
    st.pathfinder = st.effects.iter().any(|e| is_pathfinder(&e.id));
    // A group's Pathfinder effects are drawn before its other effects: one listed after another
    // effect keeps the drawn look.
    if st.effects.iter().skip_while(|e| is_pathfinder(&e.id)).any(|e| is_pathfinder(&e.id)) {
        b.plain = false;
    }
    st.plain = b.plain;
    // Bas Relief read from its settings, not its defaults standing in.
    let bas_relief = filters.iter().all(|f| f.name != BAS_RELIEF || f.data.is_some());
    let count = |stroke: bool| st.items.iter().filter(|i| matches!(i, StyleItem::Stroke { .. }) == stroke).count();
    st.simple = st.plain
        && count(true) <= 1
        && count(false) <= 1
        && st.items.iter().all(|i| matches!(i, StyleItem::Fill(_) | StyleItem::Stroke { .. } | StyleItem::Gradient { .. }))
        && st.looks.iter().all(|l| l.visible && l.opacity >= 1.0 && l.blend == BlendMode::Normal && l.effects.is_empty())
        && st.effects.iter().all(|e| match e.id.as_str() {
            "stylize.dropShadow"
            | "stylize.innerGlow"
            | "stylize.outerGlow"
            | "stylize.feather"
            | "stylize.roundCorners"
            | "stylize.scribble"
            | "convertToShape.rectangle"
            | "convertToShape.roundedRectangle"
            | "convertToShape.ellipse"
            | "path.offsetPath"
            | "path.outlineObject"
            | "path.outlineStroke" => true,
            "sketch.basRelief" => bas_relief,
            id => is_pathfinder(id),
        })
        // Pathfinder effects on a group with no fills or strokes of its own (`styled` checks the group).
        && (!st.pathfinder || st.items.is_empty());
    st
}

mod adapter;
#[cfg(test)]
mod tests;
