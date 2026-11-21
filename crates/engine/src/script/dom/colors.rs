//! The color classes of the script object model, as the help pages
//! <https://ai-scripting.docsforadobe.dev/jsobjref/RGBColor/>,
//! <https://ai-scripting.docsforadobe.dev/jsobjref/CMYKColor/>,
//! <https://ai-scripting.docsforadobe.dev/jsobjref/GrayColor/>,
//! <https://ai-scripting.docsforadobe.dev/jsobjref/LabColor/> and
//! <https://ai-scripting.docsforadobe.dev/jsobjref/NoColor/> describe them.
//!
//! A color is plain data: `new RGBColor()` gives an object with `typename` and its components,
//! which the script can change, and reading `fillColor` or `strokeColor` gives a new one each time.
//! Components are checked when a color is assigned to an object, and a value out of range throws a
//! `RangeError` with nothing changed.
//!
//! VectorCraft's own rules, which the help pages do not state: components left out count as 0;
//! `new cmykColor()` is the same as `new CMYKColor()`; `LabColor.l` is 0 to 100 and `a` and `b`
//! are -128 to 128. `GrayColor.gray` runs from 0 (black) to 100 (white), as the GrayColor page
//! says. `GradientColor`, `SpotColor` and `PatternColor` can be read from an object but not
//! assigned yet.

use std::rc::Rc;

use serde_json::Value;
use vectorcraft_color::{Color, Paint};
use vectorcraft_js::{Data, HostClass, HostError, HostFunction};

use super::{Link, current, num, text};

/// The classes whose objects can be read from a fill or stroke but not assigned yet.
const LATER: &[&str] = &["GradientColor", "SpotColor", "PatternColor"];

/// The components of each color class, with the value `new` gives them.
const CLASSES: &[(&str, &[&str])] = &[
    ("RGBColor", &["red", "green", "blue"]),
    ("CMYKColor", &["cyan", "magenta", "yellow", "black"]),
    ("GrayColor", &["gray"]),
    ("LabColor", &["l", "a", "b"]),
    ("NoColor", &[]),
    ("GradientColor", &[]),
    ("SpotColor", &["tint"]),
    ("PatternColor", &[]),
];

fn color_object(class: &str, values: &[(&str, f64)]) -> Data {
    let mut items = vec![("typename".to_string(), text(class))];
    items.extend(values.iter().map(|(k, v)| ((*k).to_string(), num(*v))));
    Data::Object(items)
}

/// A component as the script sees it: `exact` rounded to the fewest decimals, from 4 to 9, that
/// `back` turns into the stored `v` again, else `exact` itself, so that assigning a color read
/// from an object gives that object the same color.
fn shown_as(exact: f64, v: f32, back: impl Fn(f64) -> f32) -> f64 {
    let mut scale = 1e4;
    for _ in 4..=9 {
        let r = (exact * scale).round() / scale;
        if back(r) == v {
            return r;
        }
        scale *= 10.0;
    }
    exact
}

/// A component stored as `v`, from 0 to 1, that the script sees times `scale`.
fn shown(v: f32, scale: f64) -> f64 {
    shown_as(f64::from(v) * scale, v, |r| unit(r, scale))
}

/// The color object for a solid color.
fn of_color(c: &Color) -> Data {
    match *c {
        Color::Rgb { r, g, b } => color_object("RGBColor", &[("red", shown(r, 255.0)), ("green", shown(g, 255.0)), ("blue", shown(b, 255.0))]),
        Color::Cmyk { c, m, y, k } => color_object(
            "CMYKColor",
            &[("cyan", shown(c, 100.0)), ("magenta", shown(m, 100.0)), ("yellow", shown(y, 100.0)), ("black", shown(k, 100.0))],
        ),
        Color::Gray { k } => color_object("GrayColor", &[("gray", shown_as((1.0 - f64::from(k)) * 100.0, k, |g| unit(100.0 - g, 100.0)))]),
        Color::Lab { l, a, b } => color_object("LabColor", &[("l", shown(l, 1.0)), ("a", shown(a, 1.0)), ("b", shown(b, 1.0))]),
    }
}

/// What reading `fillColor` or `strokeColor` gives for `paint`: a new color object, a `SpotColor`
/// with its `tint` for a color linked to a global or spot swatch.
pub(super) fn of_paint(paint: &Paint) -> Data {
    match paint {
        Paint::None => color_object("NoColor", &[]),
        Paint::Solid { swatch: Some(_), tint, .. } => color_object("SpotColor", &[("tint", shown(*tint, 100.0))]),
        Paint::Solid { color, .. } => of_color(color),
        Paint::Gradient(_) => color_object("GradientColor", &[]),
        Paint::Pattern { .. } => color_object("PatternColor", &[]),
    }
}

/// The component `key` of color object `o`, from `min` to `max` (0 when it is left out).
fn component(o: &[(String, Data)], class: &str, key: &str, min: f64, max: f64) -> std::result::Result<f64, HostError> {
    match o.iter().find(|(k, _)| k == key).map(|(_, v)| v) {
        None | Some(Data::Undefined) => Ok(0.0),
        Some(Data::Number(n)) if n.is_finite() && (min..=max).contains(n) => Ok(*n),
        Some(Data::Number(_)) => Err(HostError::Range(format!("{class}.{key} must be from {min} to {max}"))),
        Some(_) => Err(HostError::Type(format!("{class}.{key} must be a number"))),
    }
}

#[allow(clippy::cast_possible_truncation)]
fn unit(v: f64, scale: f64) -> f32 {
    (v / scale) as f32
}

/// The color a script assigned, checked: `None` for a `NoColor`. `what` names the property.
pub(super) fn assigned(d: &Data, what: &str) -> std::result::Result<Option<Color>, HostError> {
    let Data::Object(o) = d else { return Err(HostError::Type(format!("{what} must be a color object"))) };
    let class = match o.iter().find(|(k, _)| k == "typename").map(|(_, v)| v) {
        Some(Data::String(s)) => s.as_str(),
        _ => return Err(HostError::Type(format!("{what} must be a color object"))),
    };
    let c = |key: &str, min: f64, max: f64| component(o, class, key, min, max);
    Ok(Some(match class {
        "NoColor" => return Ok(None),
        "RGBColor" => {
            Color::Rgb { r: unit(c("red", 0.0, 255.0)?, 255.0), g: unit(c("green", 0.0, 255.0)?, 255.0), b: unit(c("blue", 0.0, 255.0)?, 255.0) }
        }
        "CMYKColor" => Color::Cmyk {
            c: unit(c("cyan", 0.0, 100.0)?, 100.0),
            m: unit(c("magenta", 0.0, 100.0)?, 100.0),
            y: unit(c("yellow", 0.0, 100.0)?, 100.0),
            k: unit(c("black", 0.0, 100.0)?, 100.0),
        },
        "GrayColor" => Color::Gray { k: unit(100.0 - c("gray", 0.0, 100.0)?, 100.0) },
        "LabColor" => Color::Lab { l: unit(c("l", 0.0, 100.0)?, 1.0), a: unit(c("a", -128.0, 128.0)?, 1.0), b: unit(c("b", -128.0, 128.0)?, 1.0) },
        l if LATER.contains(&l) => return Err(HostError::Error(format!("assigning a {l} is not available yet"))),
        _ => return Err(HostError::Type(format!("{what} must be a color object"))),
    }))
}

/// `color` as `paint.setFill` and `paint.setStroke` take it.
pub(super) fn param(color: &Color) -> Value {
    serde_json::to_value(color).unwrap_or(Value::Null)
}

/// The constructors of the color classes, and `cmykColor`.
pub(super) fn classes(link: &Link) -> Vec<HostClass> {
    let make = |name: &str, components: &'static [&'static str]| {
        let l = link.clone();
        let class = name.to_string();
        let f: HostFunction = Rc::new(move |_| {
            current(&l)?.charge_call(64)?;
            let zeros: Vec<(&str, f64)> = components.iter().map(|k| (*k, if *k == "tint" { 100.0 } else { 0.0 })).collect();
            Ok(color_object(&class, &zeros))
        });
        f
    };
    let mut out: Vec<HostClass> = CLASSES
        .iter()
        .map(|(name, components)| HostClass {
            name: (*name).into(),
            parent: None,
            members: Vec::new(),
            index: None,
            construct: Some(make(name, components)),
        })
        .collect();
    out.push(HostClass {
        name: "cmykColor".into(),
        parent: None,
        members: Vec::new(),
        index: None,
        construct: Some(make("CMYKColor", CLASSES[1].1)),
    });
    out
}
