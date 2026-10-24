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
//! effects; others keep their drawn look.

use std::collections::HashMap;

use serde_json::json;
use vectorcraft_color::{BlendMode, Paint};
use vectorcraft_doc::{Dash, Effect, LineCap, LineJoin, WidthProfile};

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

/// A plain style's paint, as the object comes in.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct ArtStyle {
    /// The style reads as [`Self::items`] and [`Self::effects`].
    pub plain: bool,
    /// It is one fill and one stroke at most, in solid colours or the object's own gradient, with
    /// no transparency or effects of their own, and only Outer Glow, Feather and Drop Shadow on the
    /// object: what any object is read with. Type takes the other plain styles too (its drawn look
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

#[derive(Debug)]
struct Filter {
    name: String,
    visible: bool,
    /// The compound filter it is in (`None`: the style's first).
    parent: Option<usize>,
    compound: bool,
    params: HashMap<String, f64>,
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
    let mut lex = Lexer::new(data);
    while let Some(t) = lex.next_token() {
        if let (Some(a), Tok::Num(v)) = (array.as_mut(), &t.tok) {
            a.push(V::Num(*v));
            continue;
        }
        match t.tok {
            Tok::Num(v) => operands.push(V::Num(v)),
            Tok::Str(s) | Tok::Hex(s) => operands.push(V::Str(s)),
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
                b"Int" | b"Real" | b"Bool" => {
                    let value = operands.iter().rev().find_map(V::num);
                    operands.clear();
                    if let Some(a) = dict_array.as_mut() {
                        a.extend(value);
                        continue;
                    }
                    // The key follows: `value /Real (key) ,`.
                    if let (Some(value), Some(Tok::Str(key))) = (value, lex.next_token().map(|t| t.tok))
                        && let Some(f) = filters.last_mut()
                    {
                        f.params.insert(String::from_utf8_lossy(&key).into_owned(), value);
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
        _ => return None,
    };
    Some(Effect { id: id.into(), params, visible: f.visible })
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
                _ => match effect(c) {
                    Some(e) => look.effects.push(e),
                    None => self.plain = false,
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
            _ => match effect(f) {
                Some(e) => st.effects.push(e),
                None => b.plain = false,
            },
        }
    }
    st.plain = b.plain;
    let count = |stroke: bool| st.items.iter().filter(|i| matches!(i, StyleItem::Stroke { .. }) == stroke).count();
    st.simple = st.plain
        && count(true) <= 1
        && count(false) <= 1
        && st.items.iter().all(|i| matches!(i, StyleItem::Fill(_) | StyleItem::Stroke { .. } | StyleItem::Gradient { .. }))
        && st.looks.iter().all(|l| l.visible && l.opacity >= 1.0 && l.blend == BlendMode::Normal && l.effects.is_empty())
        && st.effects.iter().all(|e| matches!(e.id.as_str(), "stylize.outerGlow" | "stylize.feather" | "stylize.dropShadow"));
    st
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_variable_width_stroke_keeps_its_profile() {
        let data = b"/KnownStyle :\n(Tapered) /Name ,\n/ActiveStyle :\n/CompoundFilter :\n(Chain Style Filter) 0 0 /Filter ,\n1 /Visible ,\n\
            /CompoundFilter :\n(Stack Style Filter) 0 0 /Filter ,\n1 /Visible ,\n/BasicFilter :\n(Stroke Style Filter) 0 0 /Filter ,\n1 /Visible ,\n\
            2 /FillOrStroke ,\n/Dictionary : /NotRecorded ,\n/StrokeStyle : 0 R\n0 0 0 0 0 0 1 XA\n0 J 0 j 30 w 10 M []0 d\n0 XR\n1 0 Xd\n/Def ;\n (StrokeStyle) ,\n\
            /Dictionary : /NotRecorded ,\n2 /Int (BSWidthSource) ,\n/Array :\n0 /Real ,\n1 /Real ,\n; (BSVariableWidthTValuesArray) ,\n\
            /Array :\n1 /Real ,\n0.1 /Real ,\n; (BSCenteredWidthsArray) ,\n; (kAIBeautifulStrokesParamsDictKey) ,\n; /Dict ;\n /Part ,\n\
            /BasicFilter :\n(Conduit Filter) 0 0 /Filter ,\n1 /Visible ,\n;\n /Part ,\n;\n";
        let styles = art_styles(data);
        let style = &styles["Tapered"];
        assert!(style.plain);
        let [StyleItem::Stroke { width, profile, .. }] = style.items.as_slice() else { panic!("{:?}", style.items) };
        assert_eq!(*width, 30.0);
        assert_eq!(profile.as_ref().map(|p| p.points.clone()), Some(vec![(0.0, 1.0, 1.0), (1.0, 0.1, 0.1)]));
    }

    #[test]
    fn dashes_aligned_to_corners_keep_the_drawn_look() {
        // GUARDS: the look holds the fitted pattern and the solid ends the style's dash array doesn't.
        let data = b"/KnownStyle :\n(Stitches) /Name ,\n/ActiveStyle :\n/CompoundFilter :\n(Chain Style Filter) 0 0 /Filter ,\n1 /Visible ,\n\
            /BasicFilter :\n(Stroke Style Filter) 0 0 /Filter ,\n1 /Visible ,\n2 /FillOrStroke ,\n/Dictionary : /NotRecorded ,\n\
            /Dictionary : /NotRecorded ,\n1 /Bool (BSDashAdj) ,\n; (kAIBeautifulStrokesParamsDictKey) ,\n\
            /StrokeStyle : 0 R\n0 0 0 0 0 0 1 XA\n1 J 0 j 2 w 10 M [5 10 ]0 d\n0 XR\n1 0 Xd\n/Def ;\n (StrokeStyle) ,\n; /Dict ;\n /Part ,\n;\n";
        assert!(!art_styles(data)["Stitches"].plain);
    }

    #[test]
    fn a_drop_shadow_offset_down_points_down() {
        // GUARDS: the document has y down; flipping `vert` like an art-space y would turn the dialog's
        // default shadow (7 pt right, 7 pt down) upwards.
        let data = b"/KnownStyle :\n(Shade) /Name ,\n/ActiveStyle :\n/CompoundFilter :\n(Chain Style Filter) 0 0 /Filter ,\n1 /Visible ,\n\
            /BasicFilter :\n(Adobe Drop Shadow) 1 0 /Filter ,\n1 /Visible ,\n/Dictionary : /NotRecorded ,\n7 /Real (horz) ,\n7 /Real (vert) ,\n; /Dict ;\n /Part ,\n;\n";
        let styles = art_styles(data);
        let [shadow] = styles["Shade"].effects.as_slice() else { panic!("{:?}", styles["Shade"]) };
        assert_eq!((shadow.params["x"].as_f64(), shadow.params["y"].as_f64()), (Some(7.0), Some(7.0)));
    }

    /// The style `Shade`: a Drop Shadow whose dictionary holds `keys`.
    fn shadow_style(keys: &str) -> ArtStyle {
        let data = format!(
            "/KnownStyle :\n(Shade) /Name ,\n/ActiveStyle :\n/CompoundFilter :\n(Chain Style Filter) 0 0 /Filter ,\n1 /Visible ,\n\
            /BasicFilter :\n(Adobe Drop Shadow) 1 0 /Filter ,\n1 /Visible ,\n/Dictionary : /NotRecorded ,\n{keys}; /Dict ;\n /Part ,\n;\n"
        );
        art_styles(data.as_bytes()).remove("Shade").unwrap_or_default()
    }

    #[test]
    fn a_drop_shadow_with_darkness_reads_its_darkness() {
        // GUARDS: a `csrc` other than 0 made the style not plain, and the object came in as its drawn look.
        let style = shadow_style("1 /Int (blnd) ,\n0.75 /Real (opac) ,\n5 /Real (blur) ,\n1 /Int (csrc) ,\n50 /Real (dark) ,\n");
        assert!(style.plain, "{style:?}");
        let [shadow] = style.effects.as_slice() else { panic!("{style:?}") };
        let p = &shadow.params;
        assert_eq!(
            (shadow.id.as_str(), &p["shade"], &p["darkness"], &p["opacity"], &p["blur"]),
            ("stylize.dropShadow", &json!("darkness"), &json!(50.0), &json!(75.0), &json!(10.0))
        );
    }

    #[test]
    fn a_drop_shadow_darkness_is_100_percent_when_missing_and_kept_within_0_to_100() {
        for (dark, want) in [("", 100.0), ("(fifty) /Real (dark) ,\n", 100.0), ("250 /Real (dark) ,\n", 100.0), ("-5 /Real (dark) ,\n", 0.0)] {
            let style = shadow_style(&format!("1 /Int (csrc) ,\n{dark}"));
            assert!(style.plain, "{dark:?}: {style:?}");
            let [shadow] = style.effects.as_slice() else { panic!("{dark:?}: {style:?}") };
            assert_eq!((&shadow.params["shade"], &shadow.params["darkness"]), (&json!("darkness"), &json!(want)), "{dark:?}");
        }
    }

    #[test]
    fn a_drop_shadow_with_color_keeps_its_params() {
        // GUARDS: a `csrc` of 0, or none, is Color; its shadow has neither `shade` nor `darkness`.
        for csrc in ["", "0 /Int (csrc) ,\n50 /Real (dark) ,\n"] {
            let style = shadow_style(&format!("2 /Int (blnd) ,\n0.5 /Real (opac) ,\n{csrc}"));
            assert!(style.plain, "{csrc:?}: {style:?}");
            let [shadow] = style.effects.as_slice() else { panic!("{csrc:?}: {style:?}") };
            let want = json!({"mode": "Screen", "opacity": 50.0, "blur": 10.0, "x": 7.0, "y": 7.0, "color": vectorcraft_color::Color::BLACK});
            assert_eq!(shadow.params, want, "{csrc:?}");
        }
    }

    #[test]
    fn a_feather_reads_its_radius_from_rad() {
        let data = b"/KnownStyle :\n(Soft) /Name ,\n/ActiveStyle :\n/CompoundFilter :\n(Chain Style Filter) 0 0 /Filter ,\n1 /Visible ,\n\
            /BasicFilter :\n(Adobe Feather) 1 0 /Filter ,\n1 /Visible ,\n/Dictionary : /NotRecorded ,\n12 /Real (rad) ,\n; /Dict ;\n /Part ,\n;\n";
        let styles = art_styles(data);
        let [feather] = styles["Soft"].effects.as_slice() else { panic!() };
        assert_eq!((feather.id.as_str(), feather.params["radius"].as_f64()), ("stylize.feather", Some(12.0)));
    }

    #[test]
    fn a_feather_falls_back_to_radius_and_defaults_to_five_points() {
        let with_name = b"/KnownStyle :\n(Soft) /Name ,\n/ActiveStyle :\n/CompoundFilter :\n(Chain Style Filter) 0 0 /Filter ,\n1 /Visible ,\n\
            /BasicFilter :\n(Adobe Feather) 1 0 /Filter ,\n1 /Visible ,\n/Dictionary : /NotRecorded ,\n3 /Real (radius) ,\n; /Dict ;\n /Part ,\n;\n";
        let styles = art_styles(with_name);
        let [feather] = styles["Soft"].effects.as_slice() else { panic!() };
        assert_eq!(feather.params["radius"].as_f64(), Some(3.0));

        let bare = b"/KnownStyle :\n(Soft) /Name ,\n/ActiveStyle :\n/CompoundFilter :\n(Chain Style Filter) 0 0 /Filter ,\n1 /Visible ,\n\
            /BasicFilter :\n(Adobe Feather) 1 0 /Filter ,\n1 /Visible ,\n/Dictionary : /NotRecorded ,\n; /Dict ;\n /Part ,\n;\n";
        let styles = art_styles(bare);
        let [feather] = styles["Soft"].effects.as_slice() else { panic!() };
        assert_eq!(feather.params["radius"].as_f64(), Some(5.0));
    }

    #[test]
    fn art_styles_ignore_filters_beyond_the_cap() {
        const UNIT: &[u8] = b"/BasicFilter :\n(Adobe Feather) 1 0 /Filter ,\n1 /Visible ,\n/Dictionary : /NotRecorded ,\n; /Dict ;\n /Part ,\n";
        let mut data =
            Vec::from(b"/KnownStyle :\n(Capped) /Name ,\n/ActiveStyle :\n/CompoundFilter :\n(Chain Style Filter) 0 0 /Filter ,\n1 /Visible ,\n");
        for _ in 0..MAX_FILTERS + 16 {
            data.extend_from_slice(UNIT);
        }
        data.extend_from_slice(b";\n");
        let style = art_styles(&data).remove("Capped").unwrap_or_default();
        assert!(style.effects.len() <= MAX_FILTERS);
    }

    #[test]
    fn hostile_dash_offsets_and_width_profiles_are_tamed() {
        let mut f = Filter::default();
        paint_op(&mut f, "d", &[V::Arr(vec![V::Num(5.0)]), V::Num(f64::NAN)]);
        assert_eq!(f.dash.map(|d| d.offset), Some(0.0));
        // Positions outside 0..=1 and out of order, a negative width, a non-finite one.
        let arrays = HashMap::from([
            ("BSVariableWidthTValuesArray".to_string(), vec![1.5, -0.2, 0.5, 0.7]),
            ("BSCenteredWidthsArray".to_string(), vec![-1.0, 2.0, 0.5, f64::INFINITY]),
        ]);
        assert_eq!(width_profile(&arrays).map(|p| p.points), Some(vec![(0.0, 2.0, 2.0), (0.5, 0.5, 0.5), (1.0, 0.0, 0.0)]));
    }
    /// A basic filter `name` whose dictionary holds `body`.
    fn basic(name: &str, body: &str) -> String {
        format!("/BasicFilter :\n({name}) 0 0 /Filter ,\n1 /Visible ,\n/Dictionary : /NotRecorded ,\n{body}; /Dict ;\n /Part ,\n")
    }

    /// A compound filter `name` holding `kids`.
    fn compound(name: &str, kids: &[String]) -> String {
        format!("/CompoundFilter :\n({name}) 0 0 /Filter ,\n1 /Visible ,\n{};\n /Part ,\n", kids.concat())
    }

    fn fill(paint: &str) -> String {
        basic(
            "Fill Style Filter",
            &format!("0 /Bool (UseEvenOdd) ,\n/FillStyle : 0 O\n{paint}\n0 1 0 0 0 Xy\n0 J 0 j 1 w 10 M []0 d0 XR\n0 1 Xd\n/Def ;\n (FillStyle) ,\n"),
        )
    }

    fn blend(mode: u8, opacity: f64) -> String {
        basic("Blend Style Filter", &format!("/BlendStyle : {mode} {opacity} 0 0 0 Xy\n0 J 0 j 1 w 10 M []0 d0 XR\n/Def ;\n (BlendStyle) ,\n"))
    }

    /// A comic-lettering style: a warp over a stack of a textured fill, a gradient, a white rim
    /// repeated by Transform, and the characters at the bottom.
    fn lettering() -> String {
        let stack = compound(
            "Stack Style Filter",
            &[
                basic("Stroke Style Filter", "/StrokeStyle : 0 1 0 0 0 Xy\n0 J 0 j 1 w 10 M []0 d0 XR\n0 0 Xd\n/Def ;\n (StrokeStyle) ,\n"),
                compound(
                    "Chain Style Filter",
                    &[fill("(Dots) 0 0 1 1 0 0 0 0 0 [1 0 0 1 0 0] p"), basic("PSAdapter_plugin_BsRl", "300 /Int (PrevDres) ,\n"), blend(5, 0.48)],
                ),
                fill("Bb\n0 0 0 0 Bh\n2 (Red ramp) 0 0 90 1 1 0 0 1 0 0 1 Bg\n0 BB"),
                compound(
                    "Chain Style Filter",
                    &[
                        basic("Adobe Offset Path", "3.5 /Real (ofst) ,\n0 /Int (jntp) ,\n4 /Real (mlim) ,\n"),
                        fill("0 0 0 0 1 1 1 Xa"),
                        basic(
                            "Adobe Transform",
                            "6 /Int (numCopies) ,\n0.75 /Real (moveH_Pts) ,\n-0.25 /Real (moveV_Pts) ,\n100 /Real (scaleH_Percent) ,\n100 /Real (scaleV_Percent) ,\n4 /Int (pinPoint) ,\n",
                        ),
                    ],
                ),
                basic("Conduit Filter", ""),
            ],
        );
        let root = compound(
            "Chain Style Filter",
            &[
                basic(
                    "Adobe Deform",
                    "0 /Bool (Rotate) ,\n0.02 /Real (DeformVert) ,\n4 /Int (DeformStyle) ,\n0.05 /Real (DeformValue) ,\n0 /Real (DeformHoriz) ,\n",
                ),
                stack,
                blend(0, 1.0),
            ],
        );
        format!("/KnownStyle :\n(Lettering) /Name ,\n/ActiveStyle :\n{root}\n/Execution ;\n /Def ;\n")
    }

    #[test]
    fn a_stack_of_fills_with_effects_of_their_own_is_read_as_itself() {
        // GUARDS: more than one fill, or an effect on a fill, made the style not plain, and type
        // with it came in as its drawn look (outlines) instead of type.
        let styles = art_styles(lettering().as_bytes());
        let st = &styles["Lettering"];
        assert!(st.plain, "{st:?}");
        // Bottom first: the rim, the gradient, the textured fill; the stroke paints nothing.
        let [StyleItem::Fill(rim), StyleItem::Gradient { name, angle }, StyleItem::Pattern { name: pattern, .. }] = st.items.as_slice() else {
            panic!("{:?}", st.items)
        };
        assert!(matches!(rim, Paint::Solid { .. }), "{rim:?}");
        assert_eq!((name.as_deref(), *angle, pattern.as_str()), (Some("Red ramp"), 90.0, "Dots"));
        let ids = |l: &ItemLook| l.effects.iter().map(|e| e.id.clone()).collect::<Vec<_>>();
        assert_eq!(ids(&st.looks[0]), ["path.offsetPath", "distort.transform"]);
        assert!(ids(&st.looks[1]).is_empty());
        assert_eq!(ids(&st.looks[2]), ["sketch.basRelief"]);
        assert_eq!((st.looks[2].blend, st.looks[2].opacity), (BlendMode::HardLight, 0.48));
        let transform = &st.looks[0].effects[1].params;
        assert_eq!((&transform["copies"], &transform["moveH"], &transform["moveV"]), (&json!(6), &json!(0.75), &json!(0.25)));
        assert_eq!(st.looks[0].effects[0].params["joins"], json!("round"));
        // The warp is the whole object's; the characters paint below every item.
        let [warp] = st.effects.as_slice() else { panic!("{:?}", st.effects) };
        assert_eq!((warp.id.as_str(), &warp.params["bend"], &warp.params["orientation"]), ("warp.arch", &json!(5.0), &json!("horizontal")));
        assert_eq!(st.contents_index, None);
    }

    #[test]
    fn contents_above_some_items_count_them() {
        let stack = compound("Stack Style Filter", &[fill("0 0 0 0 1 1 1 Xa"), basic("Conduit Filter", ""), fill("0 0 0 1 0 0 0 Xa")]);
        let data = format!("/KnownStyle :\n(Over) /Name ,\n/ActiveStyle :\n{}", compound("Chain Style Filter", &[stack]));
        let st = &art_styles(data.as_bytes())["Over"];
        assert!(st.plain, "{st:?}");
        assert_eq!((st.items.len(), st.contents_index), (2, Some(1)));
    }

    #[test]
    fn an_effect_not_read_here_keeps_the_drawn_look() {
        let item = compound("Chain Style Filter", &[fill("0 0 0 1 0 0 0 Xa"), basic("Adobe Scribble Fill", "")]);
        let data =
            format!("/KnownStyle :\n(Odd) /Name ,\n/ActiveStyle :\n{}", compound("Chain Style Filter", &[compound("Stack Style Filter", &[item])]));
        assert!(!art_styles(data.as_bytes())["Odd"].plain);
    }

    #[test]
    fn a_hidden_fill_comes_in_hidden() {
        let hidden = fill("0 0 0 1 0 0 0 Xa").replacen("1 /Visible", "0 /Visible", 1);
        let data = format!(
            "/KnownStyle :\n(Hidden) /Name ,\n/ActiveStyle :\n{}",
            compound("Chain Style Filter", &[compound("Stack Style Filter", &[fill("0 0 0 0 1 1 1 Xa"), hidden])])
        );
        let st = &art_styles(data.as_bytes())["Hidden"];
        assert!(st.plain, "{st:?}");
        assert_eq!(st.looks.iter().map(|l| l.visible).collect::<Vec<_>>(), [false, true]);
    }
}
