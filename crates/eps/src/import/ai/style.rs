//! Art styles (`%AI9_BeginArtStyles`; not publicly documented): the appearance an object's
//! `1 (style) XW` names, a chain of filters — a stack of fills and strokes round the object's
//! contents (`Conduit Filter`), then live effects, then its transparency (`Blend Style Filter`):
//!
//! ```text
//! /KnownStyle :
//! (name) /Name ,
//! /ActiveStyle :
//! /CompoundFilter :
//! (Chain Style Filter) 0 0 /Filter ,
//! … (Stack Style Filter) … (Stroke Style Filter) … (Conduit Filter) … (Fill Style Filter) …
//! (Adobe Outer Glow) 1 0 /Filter ,
//! 1 /Visible ,
//! 2 /Int (blnd) ,
//! 0.75 /Real (opac) ,
//! 5 /Real (blur) ,
//! /FillStyle : 0 O
//! c m y k r g b Xa
//! /Def ;
//!  (sclr) ,
//! ```
//!
//! A style is *plain* when it is one fill and one stroke at most, in solid colours (a fill's gradient
//! is the object's own), and only effects read here (Outer Glow, Drop Shadow, Feather) after them. Such
//! objects are read as themselves with that fill, stroke and those effects; others keep their
//! drawn look. A fill's or stroke's style says whether it paints with `stroke fill Xd`.

use std::collections::HashMap;

use serde_json::json;
use vectorcraft_color::Paint;
use vectorcraft_doc::{Dash, Effect, LineCap, LineJoin, WidthProfile};

use super::lex::{Lexer, Tok};
use super::obj::V;
use super::paint;

/// The filters a plain style may chain.
const PLAIN: [&str; 8] =
    ["Chain Style Filter", "Stack Style Filter", "Stroke Style Filter", "Fill Style Filter", "Conduit Filter", "Blend Style Filter", GLOW, FEATHER];
const GLOW: &str = "Adobe Outer Glow"; // brand-ok: the filter's name in the files
const FEATHER: &str = "Adobe Feather"; // brand-ok: the filter's name in the files
const SHADOW: &str = "Adobe Drop Shadow"; // brand-ok: the filter's name in the files

/// A plain style's paint, as the object comes in.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct ArtStyle {
    /// The style reads as [`Self::items`] and [`Self::effects`].
    pub plain: bool,
    /// Its fill and stroke, bottom first.
    pub items: Vec<StyleItem>,
    /// Its live effects, in the order they apply.
    pub effects: Vec<Effect>,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) enum StyleItem {
    Fill(Paint),
    /// A fill with a gradient: the object's own.
    ObjectFill,
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

#[derive(Debug)]
struct Filter {
    name: String,
    visible: bool,
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
}

impl Default for Filter {
    fn default() -> Self {
        Self {
            name: String::new(),
            visible: true,
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
        }
    }
}

/// The styles of the section `data` (between its markers), by name.
pub(super) fn art_styles(data: &[u8]) -> HashMap<String, ArtStyle> {
    let mut styles = HashMap::new();
    let mut name: Option<String> = None;
    let mut filters: Vec<Filter> = vec![];
    let mut operands: Vec<V> = vec![];
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
                }
                b"Name" if name.is_none() && filters.is_empty() => name = operands.iter().rev().find_map(V::text),
                b"Filter" => filters.push(Filter { name: operands.iter().rev().find_map(V::text).unwrap_or_default(), ..Filter::default() }),
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
        "Bg" => f.gradient = true,
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

fn style(filters: &[Filter]) -> ArtStyle {
    let count = |name: &str| filters.iter().filter(|f| f.name == name).count();
    let painted = filters.iter().rposition(|f| matches!(f.name.as_str(), "Stroke Style Filter" | "Fill Style Filter" | "Conduit Filter"));
    let mut plain = count("Stroke Style Filter") <= 1 && count("Fill Style Filter") <= 1;
    let mut effects = vec![];
    let mut items = vec![];
    // The stack lists its fills and strokes top first.
    for f in filters.iter().rev().filter(|f| f.visible) {
        match f.name.as_str() {
            "Fill Style Filter" if f.paints.1 && f.gradient => items.push(StyleItem::ObjectFill),
            "Fill Style Filter" if f.paints.1 => match &f.fill {
                Some(p) => items.push(StyleItem::Fill(p.clone())),
                None => plain = false,
            },
            // Dashes aligned to corners and path ends draw as the look's pieces: a solid end at each end
            // and the pattern, fitted, between them.
            "Stroke Style Filter" if f.paints.0 && f.params.get("BSDashAdj").is_some_and(|v| *v != 0.0) => plain = false,
            "Stroke Style Filter" if f.paints.0 => match &f.stroke {
                Some(p) if !f.gradient => items.push(StyleItem::Stroke {
                    paint: p.clone(),
                    width: f.width,
                    cap: f.cap,
                    join: f.join,
                    miter: f.miter,
                    dash: f.dash.clone(),
                    profile: width_profile(&f.arrays),
                }),
                _ => plain = false,
            },
            _ => {}
        }
    }
    for (i, f) in filters.iter().enumerate() {
        let effect = f.name == GLOW || f.name == FEATHER || f.name == SHADOW;
        if !PLAIN.contains(&f.name.as_str()) && f.name != SHADOW {
            plain = false;
        }
        // An effect inside the stack is a fill's or a stroke's, not the object's.
        if effect && painted.is_some_and(|p| i < p) {
            plain = false;
        }
        if !f.visible {
            // A hidden fill or stroke leaves the object painting differently; a hidden effect, nothing.
            plain &= effect;
            continue;
        }
        let param = |k: &str| f.params.get(k).copied();
        let mode = |default: usize| super::read::blend_name(param("blnd").map_or(default, |v| v.max(0.0) as usize));
        let color = f.fill.as_ref().and_then(Paint::color).unwrap_or(vectorcraft_color::Color::BLACK);
        match f.name.as_str() {
            // `blur`'s scale isn't publicly documented: read as the Gaussian's σ, an effect's blur being 2σ.
            GLOW => effects.push(Effect {
                id: "stylize.outerGlow".into(),
                params: json!({"mode": mode(2), "opacity": param("opac").unwrap_or(0.75) * 100.0, "blur": 2.0 * param("blur").unwrap_or(5.0), "color": color}),
                visible: true,
            }),
            // LiveEffect XML uses `radius`; art styles abbreviate it `rad` (public scripting posts on applyEffect).
            FEATHER => effects.push(Effect {
                id: "stylize.feather".into(),
                params: json!({"radius": param("rad").or_else(|| param("radius")).filter(|v| v.is_finite()).unwrap_or(5.0)}),
                visible: true,
            }),
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
                    params["darkness"] = json!(param("dark").filter(|v| v.is_finite()).map_or(100.0, |v| v.clamp(0.0, 100.0)));
                }
                effects.push(Effect { id: "stylize.dropShadow".into(), params, visible: true });
            }
            _ => {}
        }
    }
    ArtStyle { plain, items, effects }
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
}
