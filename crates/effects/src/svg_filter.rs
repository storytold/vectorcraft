//! Effect › SVG Filters (`svg.filter`): an SVG `<filter>` element applied as a live raster effect.
//! The filter runs on the object's pixels as SVG 1.1 describes it
//! (<https://www.w3.org/TR/SVG11/filters.html>), drawn by resvg. Its lengths are in points times
//! the effect's `scale`, its user space starts at the effect's `origin`, its object bounding box is the box of the drawn pixels, and it runs at the
//! Document Raster Effects resolution when the view shows the object at a finer one.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use resvg::{tiny_skia, usvg};
use serde_json::Value;
use usvg::roxmltree;
use vectorcraft_doc::SvgFilter;
use vectorcraft_geom::{Affine, Rect, Vec2};

use crate::pixel::PixelSpace;
use crate::util::{num, text};

#[cfg(test)]
mod tests;

/// Effect › SVG Filters' effect id.
pub const SVG_FILTER: &str = "svg.filter";

/// Most primitives a filter holds.
pub const MAX_PRIMITIVES: usize = 64;

/// Most octaves an `feTurbulence` adds up.
pub const MAX_OCTAVES: f64 = 10.0;

/// The largest `feConvolveMatrix` kernel, in cells along a side.
pub const MAX_KERNEL: f64 = 9.0;

/// Longest filter name, in characters.
pub const MAX_NAME: usize = 255;

/// The largest `feMorphology` radius, in user units.
pub const MAX_RADIUS: f64 = 100.0;

/// The largest `stdDeviation` of a blur or a drop shadow, in user units.
pub const MAX_STD_DEVIATION: f64 = 500.0;

/// The largest `kernelUnitLength` of a convolution or a lighting primitive, in user units.
pub const MAX_KERNEL_UNIT: f64 = 100.0;

/// The most pixels of the raster a filter runs on that an `feMorphology` radius or a
/// `kernelUnitLength` may span, and that a `stdDeviation` may span: past them the filter runs on
/// coarser pixels, whatever the effect's `scale`.
const MAX_WINDOW_PX: f64 = 32.0;
const MAX_BLUR_PX: f64 = 512.0;

/// `v` as an SVG number list, `None` when it isn't one.
fn numbers(v: &str) -> Option<Vec<f64>> {
    svgtypes::NumberListParser::from(v).collect::<Result<Vec<f64>, _>>().ok()
}

/// Filter `xml` as it is drawn and written ([`rebuilt`]) when it passes [`check`], `None` when it
/// doesn't. Remembered for the filters seen last, so drawing does not check a filter again each time.
fn checked(xml: &str) -> Option<Arc<str>> {
    use std::hash::BuildHasher;
    type Seen = (std::collections::hash_map::RandomState, std::collections::HashMap<(u64, usize), Option<Arc<str>>>, usize);
    static SEEN: std::sync::OnceLock<std::sync::Mutex<Seen>> = std::sync::OnceLock::new();
    let seen = SEEN.get_or_init(Default::default);
    // Keyed by a hash that a file can't aim at (randomly seeded) and the length.
    let key = seen.lock().ok().map(|s| (s.0.hash_one(xml), xml.len()));
    if let Some(hit) = key.and_then(|k| seen.lock().ok().and_then(|s| s.1.get(&k).cloned())) {
        return hit;
    }
    let form: Option<Arc<str>> = checked_form(xml).ok().map(|(_, form)| Arc::from(form));
    if let (Some(k), Ok(mut s)) = (key, seen.lock())
        && !s.1.contains_key(&k)
    {
        let size = |f: &Option<Arc<str>>| f.as_ref().map_or(0, |f| f.len()) + 64;
        // One result makes room for another, so a document with many filters keeps most of them.
        while s.1.len() >= MAX_CHECKED || s.2 + size(&form) > MAX_CHECKED_BYTES {
            let Some(old) = s.1.keys().next().copied() else { break };
            let freed = s.1.remove(&old).map_or(0, |f| size(&f));
            s.2 = s.2.saturating_sub(freed);
        }
        s.2 += size(&form);
        s.1.insert(k, form.clone());
    }
    form
}

/// The most check results [`checked`] keeps, and the most bytes of filters it keeps.
const MAX_CHECKED: usize = 65_536;
const MAX_CHECKED_BYTES: usize = 32 << 20;

/// The most pixels an image of a filter (`feImage`) may have, by its header, and the most all the
/// images of one filter may have together.
pub const MAX_IMAGE_PIXELS: u64 = 4096 * 4096;

/// The most `<filter>` elements of one SVG file that Import SVG Filter reads ([`filters_of`]);
/// the others are left out.
pub const MAX_FILTERS_READ: usize = 256;

/// The attributes a filter keeps when it is drawn and written ([`rebuilt`]): the filter's region
/// and units, and its primitives' own attributes.
const FILTER_ATTRIBUTES: &[&str] = &[
    "id",
    "x",
    "y",
    "width",
    "height",
    "filterUnits",
    "primitiveUnits",
    "color-interpolation-filters",
    "in",
    "in2",
    "result",
    "mode",
    "type",
    "values",
    "operator",
    "k1",
    "k2",
    "k3",
    "k4",
    "order",
    "kernelMatrix",
    "divisor",
    "bias",
    "targetX",
    "targetY",
    "edgeMode",
    "kernelUnitLength",
    "preserveAlpha",
    "surfaceScale",
    "diffuseConstant",
    "specularConstant",
    "specularExponent",
    "lighting-color",
    "scale",
    "xChannelSelector",
    "yChannelSelector",
    "dx",
    "dy",
    "stdDeviation",
    "flood-color",
    "flood-opacity",
    "preserveAspectRatio",
    "radius",
    "baseFrequency",
    "numOctaves",
    "seed",
    "stitchTiles",
    "tableValues",
    "slope",
    "intercept",
    "amplitude",
    "exponent",
    "offset",
    "azimuth",
    "elevation",
    "z",
    "pointsAtX",
    "pointsAtY",
    "pointsAtZ",
    "limitingConeAngle",
    "href",
    "color",
];

/// The style properties a filter keeps from a `style` attribute, written as attributes.
const STYLE_PROPERTIES: [&str; 5] = ["color", "color-interpolation-filters", "flood-color", "flood-opacity", "lighting-color"];

/// The `image-rendering` values a filter keeps.
const IMAGE_RENDERING: [&str; 7] = ["auto", "optimizeQuality", "optimizeSpeed", "smooth", "high-quality", "crisp-edges", "pixelated"];

/// The image types a filter's `data:` URLs may hold.
const IMAGE_TYPES: [&str; 5] = ["image/png", "image/jpeg", "image/jpg", "image/gif", "image/webp"];

/// Is `v` a `data:` URL of a PNG, JPEG, GIF or WebP image, by its media type?
fn image_data_url(v: &str) -> bool {
    let v = v.trim_start();
    let Some(rest) = v.get(..5).filter(|s| s.eq_ignore_ascii_case("data:")).and_then(|_| v.get(5..)) else { return false };
    let media = rest.split([';', ',']).next().unwrap_or_default().trim().to_ascii_lowercase();
    IMAGE_TYPES.contains(&media.as_str())
}

/// `v` written as an XML attribute value.
fn escaped(v: &str) -> String {
    v.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// Filter element `f` (checked) rebuilt from what it holds that a filter draws with: its filter
/// elements other than `desc` and `title`, without text, comments or processing instructions, and
/// on each of them only the attributes in [`FILTER_ATTRIBUTES`] (with no prefix, from no namespace
/// or the SVG namespace, and `href` from XLink too) and the [`STYLE_PROPERTIES`] of its `style`.
fn rebuilt(f: roxmltree::Node) -> String {
    let mut out = String::new();
    write_rebuilt(f, &mut out);
    out
}

fn write_rebuilt(n: roxmltree::Node, out: &mut String) {
    let name = n.tag_name().name();
    let mut attrs: Vec<(&str, String)> = vec![];
    // An attribute without a prefix comes first, as usvg takes it.
    for ns in [None, Some(SVG_NS), Some(XLINK_NS)] {
        for a in n.attributes().filter(|a| a.namespace() == ns && (ns != Some(XLINK_NS) || a.name() == "href")) {
            if FILTER_ATTRIBUTES.contains(&a.name()) && !attrs.iter().any(|(k, _)| *k == a.name()) {
                attrs.push((a.name(), a.value().to_string()));
            }
        }
    }
    // Style declarations win over attributes.
    let style = n.attributes().find(|a| a.name() == "style" && matches!(a.namespace(), None | Some(SVG_NS))).map(|a| a.value()).unwrap_or_default();
    for (k, v) in style.split(';').filter_map(|d| d.split_once(':')) {
        let v = v.trim().trim_end_matches("!important").trim();
        if let Some(&k) = STYLE_PROPERTIES.iter().find(|p| p.eq_ignore_ascii_case(k.trim()))
            && !v.to_ascii_lowercase().contains("url(")
        {
            attrs.retain(|(name, _)| *name != k);
            attrs.push((k, v.to_string()));
        }
    }
    // `image-rendering`, from its attribute or its style declaration, as a style declaration (usvg
    // reads `pixelated` and `crisp-edges` only there), and only as one of its keywords.
    let rendering = style
        .split(';')
        .filter_map(|d| d.split_once(':'))
        .filter(|(k, _)| k.trim().eq_ignore_ascii_case("image-rendering"))
        .map(|(_, v)| v.trim().trim_end_matches("!important").trim())
        .next_back()
        .or_else(|| n.attributes().find(|a| a.name() == "image-rendering" && matches!(a.namespace(), None | Some(SVG_NS))).map(|a| a.value().trim()))
        .and_then(|v| IMAGE_RENDERING.iter().find(|k| k.eq_ignore_ascii_case(v)));
    out.push('<');
    out.push_str(name);
    for (k, v) in &attrs {
        out.push_str(&format!(" {k}=\"{}\"", escaped(v)));
    }
    if let Some(v) = rendering {
        out.push_str(&format!(" style=\"image-rendering:{v}\""));
    }
    let children: Vec<roxmltree::Node> = n
        .children()
        .filter(|c| c.is_element() && c.tag_name().namespace() == Some(SVG_NS) && !matches!(c.tag_name().name(), "desc" | "title"))
        .filter(|c| FILTER_ELEMENTS.contains(&c.tag_name().name()))
        .collect();
    if children.is_empty() {
        out.push_str("/>");
        return;
    }
    out.push('>');
    for c in children {
        write_rebuilt(c, out);
    }
    out.push_str(&format!("</{name}>"));
}

/// The primitives a filter runs, as usvg reads them: the first elements of [`FILTER_ELEMENTS`].
const PRIMITIVES: [&str; 17] = [
    "feBlend",
    "feColorMatrix",
    "feComponentTransfer",
    "feComposite",
    "feConvolveMatrix",
    "feDiffuseLighting",
    "feDisplacementMap",
    "feDropShadow",
    "feFlood",
    "feGaussianBlur",
    "feImage",
    "feMerge",
    "feMorphology",
    "feOffset",
    "feSpecularLighting",
    "feTile",
    "feTurbulence",
];

/// The `feBlend` modes of SVG 1.1, and the ones resvg also draws that SVG 1.1 lacks.
const SVG11_BLEND_MODES: [&str; 5] = ["normal", "multiply", "screen", "darken", "lighten"];
const LATER_BLEND_MODES: [&str; 11] =
    ["overlay", "color-dodge", "color-burn", "hard-light", "soft-light", "difference", "exclusion", "hue", "saturation", "color", "luminosity"];

/// The `feComposite` operators of SVG 1.1.
const SVG11_OPERATORS: [&str; 6] = ["over", "in", "out", "atop", "xor", "arithmetic"];

/// The longest form SVG output writes ([`svg11`]), leaving room within [`MAX_DRAWN_XML`] for the
/// region and id written into it.
const MAX_WRITTEN_XML: usize = MAX_DRAWN_XML - 1024;

/// Filter `xml` (a rebuilt one, [`rebuilt`]) as SVG 1.1 output writes it, in the form an SVG 1.1
/// viewer draws as resvg draws `xml`:
/// - `feImage`'s `href` as `xlink:href`;
/// - each `feDropShadow` (from Filter Effects Module Level 1,
///   <https://www.w3.org/TR/filter-effects-1/#feDropShadowElement>) as the SVG 1.1 primitives that
///   page gives as its equivalent: its input blurred by its `stdDeviation`, offset by its `dx` and
///   `dy` (2, 2 and 2 when absent), a flood of its `flood-color` and `flood-opacity` composited
///   `in` that, and a merge of the result over the input. The merge keeps its `result` and its
///   subregion, and the results it adds take names no other primitive of the filter uses;
/// - an `feComposite` operator SVG 1.1 lacks (`lighter`) and an `feBlend` mode resvg doesn't read
///   left out, so the default that resvg draws (`over`, `normal`) applies;
/// - colors in forms SVG 1.1 lacks (`rgba()`, `hsl()`, `transparent`, `#rgba` and the like) as
///   `#rrggbb`, with a flood color's alpha multiplied into `flood-opacity` and a lighting color's
///   alpha dropped, as resvg draws them, and `flood-opacity` as a number;
/// - `image-rendering` as `optimizeSpeed` for `pixelated` and `crisp-edges`, and as
///   `optimizeQuality` for `auto`, `smooth` and `high-quality`;
/// - `edgeMode` only on `feConvolveMatrix`, the one SVG 1.1 primitive that has it;
/// - each `in` and `in2` (also of `feMergeNode`) naming the primitive usvg reads: a result name
///   only usvg gives (`result<n>` for a primitive without a `result`) as that primitive's written
///   result, a name no earlier primitive has as the previous primitive's result (`SourceGraphic`
///   for the first), and `BackgroundImage`, `BackgroundAlpha`, `FillPaint` and `StrokePaint`,
///   which usvg draws as `SourceGraphic`, as `SourceGraphic`.
///
/// `None` when SVG 1.1 can't hold the filter: an `feBlend` mode SVG 1.1 lacks (`overlay` and the
/// others in [`LATER_BLEND_MODES`]); an `feDropShadow` whose `flood-color` isn't a color, such as
/// `currentColor` (resvg draws it black), or in a filter with `primitiveUnits="objectBoundingBox"`
/// (where resvg fills an `feFlood` without a subregion over the object's box alone); more than
/// [`MAX_PRIMITIVES`] primitives once the shadows are expanded; or more than [`MAX_WRITTEN_XML`]
/// bytes. SVG output then leaves the filter out like other pixel effects, and exports that turn
/// such effects into images draw the object as an image.
fn svg11(xml: &str) -> Option<String> {
    use std::collections::HashSet;
    let (wrapped, _) = wrap(xml);
    let doc = roxmltree::Document::parse(&wrapped).ok()?;
    let f = filter_element(&doc).ok()?;
    let primitives: Vec<roxmltree::Node> = f.children().filter(|c| c.is_element() && PRIMITIVES.contains(&c.tag_name().name())).collect();
    let is_shadow = |n: &roxmltree::Node| n.tag_name().name() == "feDropShadow";
    let shadows = primitives.iter().filter(|n| is_shadow(n)).count();
    if primitives.len() + 4 * shadows > MAX_PRIMITIVES
        || (shadows > 0 && f.attribute("primitiveUnits").is_some_and(|u| u.trim() == "objectBoundingBox"))
        || primitives.iter().any(|n| is_shadow(n) && n.attribute("flood-color").is_some_and(|c| color(c).is_none()))
    {
        return None;
    }
    // Every name an input or a result uses, so the added results take others.
    let taken: HashSet<String> =
        f.descendants().flat_map(|n| ["result", "in", "in2"].into_iter().filter_map(move |a| n.attribute(a))).map(str::to_string).collect();
    let named = |n: &roxmltree::Node| n.attribute("result").filter(|r| !r.trim().is_empty()).map(str::to_string);
    let mut names = Names { taken, written: primitives.iter().map(named).collect() };
    // The result names usvg gives the primitives: their own, or `result<n>` generated in order.
    let mut drawn: Vec<String> = Vec::with_capacity(primitives.len());
    let (mut own, mut next) = (HashSet::new(), 1usize);
    for p in &primitives {
        match p.attribute("result") {
            Some(r) => {
                own.insert(r.to_string());
                next += 1;
                drawn.push(r.to_string());
            }
            None => loop {
                let name = format!("result{next}");
                next += 1;
                if !own.contains(&name) {
                    drawn.push(name);
                    break;
                }
            },
        }
    }
    // Each input written as the primitive it reads when drawn: a result only usvg names (or one
    // with no name) as that primitive's written result, an unknown result as the previous
    // primitive's (`SourceGraphic` for the first), and an input usvg draws as `SourceGraphic` as
    // that. A shadow names its input twice (the blur and the merge), so an implicit one is named.
    let mut inputs = Inputs::new();
    let mut shadow_inputs: Vec<Option<String>> = vec![None; primitives.len()];
    for (i, p) in primitives.iter().enumerate() {
        let previous = |names: &mut Names| i.checked_sub(1).map_or_else(|| "SourceGraphic".to_string(), |j| names.of(j));
        let resolve = |v: &str, names: &mut Names| match v {
            "SourceGraphic" | "SourceAlpha" => v.to_string(),
            "BackgroundImage" | "BackgroundAlpha" | "FillPaint" | "StrokePaint" => "SourceGraphic".to_string(),
            _ => match (0..i).rev().find(|&j| drawn.get(j).is_some_and(|d| d == v)) {
                Some(j) if !v.trim().is_empty() && primitives.get(j).and_then(|q| q.attribute("result")) == Some(v) => v.to_string(),
                Some(j) => names.of(j),
                None => previous(names),
            },
        };
        let mut elements = vec![*p];
        if p.tag_name().name() == "feMerge" {
            elements.extend(p.children().filter(|c| c.is_element() && c.tag_name().name() == "feMergeNode"));
        }
        for e in elements {
            for a in ["in", "in2"] {
                if let Some(v) = e.attribute(a) {
                    let r = resolve(v, &mut names);
                    inputs.insert((e.id(), a), r);
                }
            }
        }
        if is_shadow(p) {
            shadow_inputs[i] = Some(match p.attribute("in") {
                Some(v) => resolve(v, &mut names),
                None => previous(&mut names),
            });
        }
    }
    let mut out = format!("<filter{}", svg11_attributes(f, None, &inputs)?);
    let children: Vec<roxmltree::Node> = f.children().filter(roxmltree::Node::is_element).collect();
    if children.is_empty() {
        out.push_str("/>");
    } else {
        out.push('>');
        let mut i = 0;
        for c in children {
            if !PRIMITIVES.contains(&c.tag_name().name()) {
                write_svg11(c, None, &inputs, &mut out)?;
                continue;
            }
            let result = names.written.get(i).cloned().flatten();
            match shadow_inputs.get(i).cloned().flatten() {
                Some(input) => {
                    let added = (names.fresh(), names.fresh());
                    write_drop_shadow(c, &input, result.as_deref(), added, &mut out);
                }
                // A result that differs from the element's own is one added for a later reference.
                None => write_svg11(c, result.as_deref().filter(|r| Some(*r) != c.attribute("result")), &inputs, &mut out)?,
            }
            i += 1;
        }
        out.push_str("</filter>");
    }
    (out.len() <= MAX_WRITTEN_XML).then_some(out)
}

/// The result names of a filter's SVG 1.1 form ([`svg11`]): the names its elements use, and the
/// result each primitive is written with.
struct Names {
    taken: std::collections::HashSet<String>,
    written: Vec<Option<String>>,
}

impl Names {
    /// A result name no element of the filter uses.
    fn fresh(&mut self) -> String {
        let name = (0..).map(|k| format!("vcShadow{k}")).find(|n| !self.taken.contains(n)).unwrap_or_default();
        self.taken.insert(name.clone());
        name
    }

    /// The result primitive `j` is written with, given a new name when it has none.
    fn of(&mut self, j: usize) -> String {
        if let Some(Some(r)) = self.written.get(j) {
            return r.clone();
        }
        let r = self.fresh();
        if let Some(w) = self.written.get_mut(j) {
            *w = Some(r.clone());
        }
        r
    }
}

/// `v` as a color, as usvg reads it: `None` for anything else, `currentColor` included.
fn color(v: &str) -> Option<svgtypes::Color> {
    <svgtypes::Color as std::str::FromStr>::from_str(v).ok()
}

/// `v` as an opacity, as usvg reads it: a number or a percentage, clamped to 0 to 1.
fn opacity(v: &str) -> Option<f64> {
    let l = <svgtypes::Length as std::str::FromStr>::from_str(v).ok()?;
    let o = match l.unit {
        svgtypes::LengthUnit::None => l.number,
        svgtypes::LengthUnit::Percent => l.number / 100.0,
        _ => return None,
    };
    o.is_finite().then(|| o.clamp(0.0, 1.0))
}

/// Number `v` written with at most six decimals.
fn decimal(v: f64) -> String {
    let s = format!("{v:.6}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// Color `c` as `#rrggbb`, without its alpha.
fn hex(c: svgtypes::Color) -> String {
    format!("#{:02x}{:02x}{:02x}", c.red, c.green, c.blue)
}

/// The `flood-color` and `flood-opacity` of element `n` as SVG 1.1 writes them ([`svg11`]): a color
/// as `#rrggbb` with its alpha multiplied into the opacity, and the opacity as a number. A
/// `flood-color` that isn't a color is written as it is.
fn flood_attributes(n: roxmltree::Node) -> String {
    let mut out = String::new();
    let flood = n.attribute("flood-color");
    let parsed = flood.and_then(color);
    match (flood, parsed) {
        (_, Some(c)) => out.push_str(&format!(" flood-color=\"{}\"", hex(c))),
        (Some(v), None) => out.push_str(&format!(" flood-color=\"{}\"", escaped(v))),
        (None, None) => {}
    }
    let alpha = parsed.map_or(1.0, |c| f64::from(c.alpha) / 255.0);
    let given = n.attribute("flood-opacity");
    if given.is_some() || alpha < 1.0 {
        let o = given.and_then(opacity).unwrap_or(1.0);
        out.push_str(&format!(" flood-opacity=\"{}\"", decimal(alpha * o)));
    }
    out
}

/// Inputs written in place of an element's own `in` and `in2` ([`svg11`]).
type Inputs<'a> = std::collections::HashMap<(roxmltree::NodeId, &'a str), String>;

/// The attributes of element `n` of a rebuilt filter as SVG 1.1 writes them ([`svg11`]), with
/// `result` in place of its own when given and its inputs as `inputs` gives them. `None` for an
/// `feBlend` mode SVG 1.1 lacks.
fn svg11_attributes(n: roxmltree::Node, result: Option<&str>, inputs: &Inputs) -> Option<String> {
    let name = n.tag_name().name();
    let mut out = String::new();
    let mut push = |k: &str, v: &str| out.push_str(&format!(" {k}=\"{}\"", escaped(v)));
    for a in n.attributes() {
        let v = a.value();
        match a.name() {
            "href" => push("xlink:href", v),
            k @ ("in" | "in2") => push(k, inputs.get(&(n.id(), k)).map_or(v, String::as_str)),
            "result" if result.is_some() => {}
            "flood-color" | "flood-opacity" => {}
            "edgeMode" if name != "feConvolveMatrix" => {}
            "mode" if name == "feBlend" => {
                if LATER_BLEND_MODES.contains(&v) {
                    return None;
                }
                if SVG11_BLEND_MODES.contains(&v) {
                    push("mode", v);
                }
            }
            "operator" if name == "feComposite" => {
                if SVG11_OPERATORS.contains(&v) {
                    push("operator", v);
                }
            }
            k @ ("lighting-color" | "color") => match color(v) {
                Some(c) => push(k, &hex(c)),
                None => push(k, v),
            },
            "style" => {
                let speed = v.ends_with(":pixelated") || v.ends_with(":crisp-edges") || v.ends_with(":optimizeSpeed");
                push("style", if speed { "image-rendering:optimizeSpeed" } else { "image-rendering:optimizeQuality" });
            }
            k => push(k, v),
        }
    }
    out.push_str(&flood_attributes(n));
    if let Some(r) = result {
        out.push_str(&format!(" result=\"{}\"", escaped(r)));
    }
    Some(out)
}

/// Element `n` of a rebuilt filter, and what it holds, as SVG 1.1 writes them ([`svg11`]), with
/// `result` in place of its own when given and inputs as `inputs` gives them.
fn write_svg11(n: roxmltree::Node, result: Option<&str>, inputs: &Inputs, out: &mut String) -> Option<()> {
    let name = n.tag_name().name();
    out.push_str(&format!("<{name}{}", svg11_attributes(n, result, inputs)?));
    let children: Vec<roxmltree::Node> = n.children().filter(roxmltree::Node::is_element).collect();
    if children.is_empty() {
        out.push_str("/>");
        return Some(());
    }
    out.push('>');
    for c in children {
        write_svg11(c, None, inputs, out)?;
    }
    out.push_str(&format!("</{name}>"));
    Some(())
}

/// `feDropShadow` `n` as its SVG 1.1 equivalent ([`svg11`]) on `input`, giving `result` when set,
/// with `names` (unused elsewhere in the filter) for the offset shadow and the colored one.
fn write_drop_shadow(n: roxmltree::Node, input: &str, result: Option<&str>, names: (String, String), out: &mut String) {
    let attr = |k: &str, default: &str| escaped(n.attribute(k).unwrap_or(default));
    // An offset that isn't a number is the shadow's default 2, as usvg reads it (an `feOffset`'s is 0).
    let offset_by = |k: &str| {
        let v = n.attribute(k).filter(|v| <svgtypes::Number as std::str::FromStr>::from_str(v).is_ok()).unwrap_or("2");
        escaped(v)
    };
    let space = n.attribute("color-interpolation-filters").map(|v| format!(" color-interpolation-filters=\"{}\"", escaped(v))).unwrap_or_default();
    let (input, offset, shadow) = (escaped(input), escaped(&names.0), escaped(&names.1));
    let mut region = String::new();
    for k in ["x", "y", "width", "height"] {
        if let Some(v) = n.attribute(k) {
            region.push_str(&format!(" {k}=\"{}\"", escaped(v)));
        }
    }
    let result = result.map(|r| format!(" result=\"{}\"", escaped(r))).unwrap_or_default();
    out.push_str(&format!(
        "<feGaussianBlur in=\"{input}\" stdDeviation=\"{}\"{space}/><feOffset dx=\"{}\" dy=\"{}\" result=\"{offset}\"{space}/><feFlood{}{space}/><feComposite in2=\"{offset}\" operator=\"in\" result=\"{shadow}\"{space}/><feMerge{region}{result}{space}><feMergeNode in=\"{shadow}\"/><feMergeNode in=\"{input}\"/></feMerge>",
        attr("stdDeviation", "2"),
        offset_by("dx"),
        offset_by("dy"),
        flood_attributes(n),
    ));
}

/// `data` as an image a filter draws, with its pixel count: a PNG, JPEG, GIF or WebP, told by its
/// first bytes whatever its `data:` URL says, of at most [`MAX_IMAGE_PIXELS`] pixels by its header.
/// `None` for anything else, SVG and compressed SVG included.
fn raster_image_sized(data: Arc<Vec<u8>>) -> Option<(usvg::ImageKind, u64)> {
    let format = image::guess_format(&data).ok()?;
    let (w, h) = image::ImageReader::with_format(std::io::Cursor::new(data.as_slice()), format).into_dimensions().ok()?;
    let pixels = u64::from(w) * u64::from(h);
    if pixels == 0 || pixels > MAX_IMAGE_PIXELS {
        return None;
    }
    let kind = match format {
        image::ImageFormat::Png => usvg::ImageKind::PNG(data),
        image::ImageFormat::Jpeg => usvg::ImageKind::JPEG(data),
        image::ImageFormat::Gif => usvg::ImageKind::GIF(data),
        image::ImageFormat::WebP => usvg::ImageKind::WEBP(data),
        _ => return None,
    };
    Some((kind, pixels))
}

/// The elements a filter may hold: its primitives, what they hold (transfer functions, merge
/// nodes, light sources), and descriptions.
const FILTER_ELEMENTS: [&str; 27] = [
    "feBlend",
    "feColorMatrix",
    "feComponentTransfer",
    "feComposite",
    "feConvolveMatrix",
    "feDiffuseLighting",
    "feDisplacementMap",
    "feDropShadow",
    "feFlood",
    "feGaussianBlur",
    "feImage",
    "feMerge",
    "feMorphology",
    "feOffset",
    "feSpecularLighting",
    "feTile",
    "feTurbulence",
    "feFuncR",
    "feFuncG",
    "feFuncB",
    "feFuncA",
    "feMergeNode",
    "feDistantLight",
    "fePointLight",
    "feSpotLight",
    "desc",
    "title",
];

const SVG_NS: &str = "http://www.w3.org/2000/svg";
const XLINK_NS: &str = "http://www.w3.org/1999/xlink";

/// The id the filter takes while it is drawn.
const DRAWN_ID: &str = "vcSvgFilter";

/// `xml` inside an `<svg>` element that declares the SVG and XLink namespaces, and the length of
/// what comes before it.
fn wrap(xml: &str) -> (String, usize) {
    let head = format!("<svg xmlns=\"{SVG_NS}\" xmlns:xlink=\"{XLINK_NS}\">");
    (format!("{head}{xml}</svg>"), head.len())
}

/// The `<filter>` element of a wrapped filter ([`wrap`]): its only element, with no text beside it.
fn filter_element<'a, 'i>(doc: &'a roxmltree::Document<'i>) -> Result<roxmltree::Node<'a, 'i>, String> {
    let root = doc.root_element();
    let stray = root.children().any(|n| n.is_text() && n.text().is_some_and(|t| !t.trim().is_empty()));
    let mut elements = root.children().filter(roxmltree::Node::is_element);
    match (elements.next(), elements.next()) {
        (Some(f), None) if !stray && f.tag_name().name() == "filter" && f.tag_name().namespace() == Some(SVG_NS) => Ok(f),
        _ => Err("give one <filter> element".into()),
    }
}

/// Check filter `xml` and return its name (its `id`): one `<filter>` element with an `id`, up to
/// [`SvgFilter::MAX_XML`] bytes, 1 to [`MAX_PRIMITIVES`] primitives, `feTurbulence` octaves up to
/// [`MAX_OCTAVES`], `feConvolveMatrix` kernels up to [`MAX_KERNEL`] cells a side, morphology
/// radii, standard deviations and kernel units up to [`MAX_RADIUS`], [`MAX_STD_DEVIATION`] and
/// [`MAX_KERNEL_UNIT`], images only as PNG, JPEG, GIF or WebP `data:` URLs, that resvg can draw.
pub fn check(xml: &str) -> Result<String, String> {
    checked_form(xml).map(|(name, _)| name)
}

/// [`check`]: the filter's name and its form as it is drawn and written ([`rebuilt`]). The caps
/// apply to that form, whose attributes are the ones usvg reads.
fn checked_form(xml: &str) -> Result<(String, String), String> {
    within_size(xml)?;
    let (wrapped, _) = wrap(xml);
    let doc = read(xml, &wrapped)?;
    let f = filter_element(&doc)?;
    let name = f.attribute("id").map(str::trim).filter(|n| !n.is_empty()).ok_or("the <filter> element needs an id, the filter's name")?;
    if name.chars().count() > MAX_NAME {
        return Err(format!("an SVG filter's id is at most {MAX_NAME} characters"));
    }
    for n in f.descendants().filter(roxmltree::Node::is_element) {
        // Filter elements only, without scripts, event attributes or links out of the filter.
        if n != f && !(n.tag_name().namespace() == Some(SVG_NS) && FILTER_ELEMENTS.contains(&n.tag_name().name())) {
            return Err(format!("<{}> isn't an element of an SVG filter", n.tag_name().name()));
        }
        if let Some(a) = n.attributes().find(|a| a.name().get(..2).is_some_and(|on| on.eq_ignore_ascii_case("on"))) {
            return Err(format!("an SVG filter has no event attributes such as {}", a.name()));
        }
        if n.attributes().any(|a| a.name() == "href" && !image_data_url(a.value())) {
            return Err("an SVG filter's images are PNG, JPEG, GIF or WebP data: URLs".into());
        }
    }
    let form = rebuilt(f);
    let (wrapped, _) = wrap(&form);
    let doc = roxmltree::Document::parse(&wrapped).map_err(|e| format!("the SVG filter isn't valid XML: {e}"))?;
    let f = filter_element(&doc)?;
    for n in f.descendants().filter(roxmltree::Node::is_element) {
        if n.attribute("numOctaves").and_then(|v| v.trim().parse::<f64>().ok()).is_some_and(|v| v > MAX_OCTAVES) {
            return Err(format!("feTurbulence's numOctaves is at most {MAX_OCTAVES}"));
        }
        // Read as usvg reads them: SVG number lists.
        for (attr, max, what) in [
            ("order", MAX_KERNEL, "feConvolveMatrix's order"),
            ("radius", MAX_RADIUS, "feMorphology's radius"),
            ("stdDeviation", MAX_STD_DEVIATION, "a blur's stdDeviation"),
            ("kernelUnitLength", MAX_KERNEL_UNIT, "kernelUnitLength"),
        ] {
            if let Some(v) = n.attribute(attr)
                && !numbers(v).is_some_and(|s| s.iter().all(|v| v.is_finite() && v.abs() <= max))
            {
                return Err(format!("{what} is at most {max}"));
            }
        }
    }
    // (Not its `desc` and `title`.)
    let primitives = f.children().filter(|n| n.is_element() && n.tag_name().name().starts_with("fe")).count();
    if primitives == 0 {
        return Err("the SVG filter has no primitives".into());
    }
    if primitives > MAX_PRIMITIVES {
        return Err(format!("an SVG filter has at most {MAX_PRIMITIVES} primitives"));
    }
    let drawn = with_id(&form, DRAWN_ID).ok_or("the SVG filter is too large once its attributes are written out")?;
    let svg = format!(
        "<svg xmlns=\"{SVG_NS}\" xmlns:xlink=\"{XLINK_NS}\" width=\"10\" height=\"10\"><defs>{drawn}</defs><rect width=\"10\" height=\"10\" filter=\"url(#{DRAWN_ID})\"/></svg>"
    );
    let over = Arc::new(AtomicBool::new(false));
    let drawn = usvg::Tree::from_str(&svg, &options_with(None, Some(Arc::clone(&over))));
    if over.load(Ordering::Relaxed) {
        return Err(format!("an SVG filter's images have at most {MAX_IMAGE_PIXELS} pixels in all"));
    }
    match drawn {
        Ok(tree) if has_filter(tree.root()) => Ok((name.to_string(), form)),
        _ => Err("the SVG filter can't be drawn: check its primitives and their inputs".into()),
    }
}

/// Can SVG output keep the SVG filter of `effects` (an object's own) as a filter: is it the last
/// visible effect? When other effects follow it, SVG output draws it as an image.
pub fn writes_live(effects: &[vectorcraft_doc::Effect]) -> bool {
    effects.iter().rfind(|e| e.visible).is_some_and(|e| e.id == SVG_FILTER)
}

/// How deep a filter's elements may nest. A filter needs three levels: the filter, a primitive, and
/// a primitive's transfer functions, merge nodes or lights.
const MAX_DEPTH: usize = 16;

/// How long the form of a filter that is drawn may be: a checked filter (at most
/// [`SvgFilter::MAX_XML`] bytes) with the region attributes and id that drawing writes into it.
const MAX_DRAWN_XML: usize = 2 * SvgFilter::MAX_XML;

/// Filter `xml`, `wrapped` by [`wrap`], read as XML: refused past [`SvgFilter::MAX_XML`] bytes and,
/// before the parser (which recurses) sees it, when its elements nest deeper than [`MAX_DEPTH`].
fn read<'a>(xml: &str, wrapped: &'a str) -> Result<roxmltree::Document<'a>, String> {
    within_size(xml)?;
    read_unsized(xml, wrapped)
}

/// [`read`] without the size check, for a drawn form or a caller that checks its input separately.
fn read_unsized<'a>(xml: &str, wrapped: &'a str) -> Result<roxmltree::Document<'a>, String> {
    if nests_too_deep(xml) {
        return Err(format!("an SVG filter's elements nest at most {MAX_DEPTH} deep"));
    }
    roxmltree::Document::parse(wrapped).map_err(|e| format!("the SVG filter isn't valid XML: {e}"))
}

/// Do the elements of `xml` nest deeper than [`MAX_DEPTH`]? Read from the text: comments, CDATA
/// sections, declarations and processing instructions are skipped, and a `>` inside a quoted
/// attribute value doesn't end a tag. Text that ends inside markup is left to the parser.
fn nests_too_deep(xml: &str) -> bool {
    let (mut i, mut depth) = (0usize, 0usize);
    while let Some(rest) = xml.get(i..).and_then(|r| r.find('<').and_then(|at| r.get(at..))) {
        let start = xml.len() - rest.len();
        let past = |end: &str| rest.find(end).map(|e| e + end.len());
        let step = if rest.starts_with("<!--") {
            past("-->")
        } else if rest.starts_with("<![CDATA[") {
            past("]]>")
        } else if rest.starts_with("<!") || rest.starts_with("<?") {
            past(">")
        } else if rest.starts_with("</") {
            depth = depth.saturating_sub(1);
            past(">")
        } else {
            let mut quote = None;
            let end = rest.char_indices().skip(1).find_map(|(k, c)| match (quote, c) {
                (Some(q), c) if c == q => {
                    quote = None;
                    None
                }
                (None, '"' | '\'') => {
                    quote = Some(c);
                    None
                }
                (None, '>') => Some(k),
                _ => None,
            });
            let Some(end) = end else { return false };
            if !rest.get(..end).is_some_and(|tag| tag.ends_with('/')) {
                depth += 1;
                if depth > MAX_DEPTH {
                    return true;
                }
            }
            Some(end + 1)
        };
        let Some(step) = step else { return false };
        i = start + step;
    }
    false
}

/// What a document's style sheets can reach, read once for the whole document
/// ([`depends_on_styles`]): whether a type selector names a `filter` or `fe…` element or they use
/// `*`, or they set `color-interpolation-filters` (which a filter inherits), and the ids (`#id`)
/// and attribute names (`[name]`) their selectors name. Other declarations (`filter: url(#a)` on a
/// shape) don't count.
#[derive(Clone, Debug, Default)]
pub struct Styles {
    reach_filters: bool,
    ids: std::collections::HashSet<String>,
    /// The attribute names their attribute selectors name (`[data-x]`, `[svg|x="1"]`), without a
    /// namespace prefix: an element of a filter with one of them can be styled.
    attrs: std::collections::HashSet<String>,
}

impl Styles {
    /// The style sheets (`<style>` elements) of `doc`.
    pub fn of(doc: &roxmltree::Document) -> Self {
        let sheets: String = doc
            .descendants()
            .filter(|e| e.is_element() && e.tag_name().name() == "style")
            .flat_map(|e| e.descendants().filter_map(|t| t.text()))
            .collect();
        let selectors = selectors_of(&sheets);
        let ids = selectors
            .split('#')
            .skip(1)
            .filter_map(|t| t.split(|c: char| !(c.is_alphanumeric() || matches!(c, '-' | '_'))).next())
            .filter(|t| !t.is_empty())
            .map(str::to_string)
            .collect();
        // A filter inherits `color-interpolation-filters` from whatever a sheet sets it on.
        let inherited = sheets.contains("color-interpolation-filters");
        let attrs = selectors
            .split('[')
            .skip(1)
            .filter_map(|t| t.split(|c: char| matches!(c, ']' | '=' | '~' | '^' | '$' | '*') || c.is_whitespace()).next())
            .map(|name| name.rsplit('|').next().unwrap_or(name).trim())
            .filter(|t| !t.is_empty())
            .map(str::to_string)
            .collect();
        Self { reach_filters: inherited || selects_filter_elements(&selectors), ids, attrs }
    }
}

/// Can CSS `selectors` select a `filter` element or one of its elements ([`FILTER_ELEMENTS`]):
/// a type selector naming one, in any case and with or without a namespace prefix, or the
/// universal selector `*`. Class, id, attribute and pseudo-class names don't count; type
/// selectors inside pseudo-class arguments (`:is(feFlood)`) do.
fn selects_filter_elements(selectors: &str) -> bool {
    let mut chars = selectors.char_indices().peekable();
    let (mut prev, mut in_attr) = (' ', false);
    while let Some((i, c)) = chars.next() {
        if in_attr {
            in_attr = c != ']';
        } else if c == '[' {
            in_attr = true;
        } else if c == '*' {
            return true;
        } else if c.is_alphabetic() || c == '_' {
            let mut end = i + c.len_utf8();
            while let Some(&(j, d)) = chars.peek() {
                if !(d.is_alphanumeric() || matches!(d, '-' | '_')) {
                    break;
                }
                end = j + d.len_utf8();
                chars.next();
            }
            let word = selectors.get(i..end).unwrap_or_default();
            let names_filter = word.eq_ignore_ascii_case("filter") || FILTER_ELEMENTS.iter().any(|n| n.eq_ignore_ascii_case(word));
            if names_filter && !matches!(prev, '.' | '#' | ':' | '-') {
                return true;
            }
            prev = word.chars().last().unwrap_or(c);
            continue;
        }
        prev = c;
    }
    false
}

/// The selectors of CSS `sheet`, outside comments: at any nesting depth, the text that opens a
/// block (`… {`), at-rules' preludes (`@scope (#a) {`) included. Declarations, the text a `;` or
/// `}` ends, aren't selectors.
fn selectors_of(sheet: &str) -> String {
    let (mut out, mut prelude, mut rest) = (String::new(), String::new(), sheet);
    while let Some(c) = rest.chars().next() {
        if rest.starts_with("/*") {
            rest = rest.find("*/").and_then(|e| rest.get(e + 2..)).unwrap_or_default();
            continue;
        }
        rest = rest.get(c.len_utf8()..).unwrap_or_default();
        // A quoted string (backslash escapes included) as a whole: its braces and `/*` are text.
        if c == '"' || c == '\'' {
            let mut end = rest.len();
            let mut escaped = false;
            for (k, q) in rest.char_indices() {
                match (escaped, q) {
                    (true, _) => escaped = false,
                    (false, '\\') => escaped = true,
                    (false, q) if q == c => {
                        end = k + q.len_utf8();
                        break;
                    }
                    _ => {}
                }
            }
            prelude.push(c);
            prelude.push_str(rest.get(..end).unwrap_or_default());
            rest = rest.get(end..).unwrap_or_default();
            continue;
        }
        match c {
            // An escaped character (`\;`) is text.
            '\\' => {
                prelude.push(c);
                if let Some(next) = rest.chars().next() {
                    prelude.push(next);
                    rest = rest.get(next.len_utf8()..).unwrap_or_default();
                }
            }
            '{' => {
                out.push_str(&prelude);
                out.push(',');
                prelude.clear();
            }
            ';' | '}' => prelude.clear(),
            _ => prelude.push(c),
        }
    }
    out
}

/// The attributes of a filter's elements that can take `currentColor`.
const COLOR_ATTRS: [&str; 7] = ["flood-color", "lighting-color", "fill", "stroke", "stop-color", "color", "style"];

/// Does attribute `a` take its color from `currentColor`, in any case? Only attributes that carry
/// a color are looked at (an `id` named "currentColor" doesn't), searched in place without a copy
/// of the value.
fn names_current_color(a: &roxmltree::Attribute) -> bool {
    COLOR_ATTRS.contains(&a.name()) && a.value().as_bytes().windows(b"currentcolor".len()).any(|w| w.eq_ignore_ascii_case(b"currentcolor"))
}

/// Does filter element `n` look different outside its document: does an element of the filter
/// have a `class` or use `currentColor`, can the document's style sheets (`styles`, [`Styles::of`]
/// its document) reach it, or does an ancestor set `color-interpolation-filters` or a `style`?
pub fn depends_on_styles(n: roxmltree::Node, styles: &Styles) -> bool {
    styles.reach_filters
        || n.descendants().filter_map(|e| e.attribute("id")).any(|id| styles.ids.contains(id))
        || n.descendants().any(|e| e.attributes().any(|a| styles.attrs.contains(a.name())))
        || n.descendants().any(|e| e.attribute("class").is_some() || e.attributes().any(|a| names_current_color(&a)))
        || n.ancestors().skip(1).any(|a| a.attribute("color-interpolation-filters").is_some() || a.attribute("style").is_some())
}

/// The SVG filters of SVG document `doc` (read from `text`) that [`check`] takes, as (name, the
/// `<filter>` element's XML from [`element_xml`]), in document order, up to `room` of them, and how
/// many `<filter>` elements it didn't take. A filter named as `taken` says (a name the caller has),
/// one past `room` or past the first [`MAX_FILTERS_READ`] filters, and one whose look depends on
/// the document's styles ([`depends_on_styles`]) are not checked. The caller reads the document
/// with its own limits on depth and entities.
pub fn filters_of(doc: &roxmltree::Document, text: &str, room: usize, taken: impl Fn(&str) -> bool) -> (Vec<(String, String)>, usize) {
    let (mut found, mut skipped): (Vec<(String, String)>, usize) = (vec![], 0);
    let styles = Styles::of(doc);
    let filters = doc.descendants().filter(|n| n.is_element() && n.tag_name().name() == "filter" && n.tag_name().namespace() == Some(SVG_NS));
    for (k, n) in filters.enumerate() {
        let id = n.attribute("id").map(str::trim).unwrap_or_default();
        if k >= MAX_FILTERS_READ || found.len() >= room || taken(id) || found.iter().any(|(name, _)| name == id) || depends_on_styles(n, &styles) {
            skipped += 1;
            continue;
        }
        match element_xml(n, text).map(|xml| check(&xml).map(|name| (name, xml))) {
            Some(Ok(f)) => found.push(f),
            _ => skipped += 1,
        }
    }
    (found, skipped)
}

/// The XML of element `n` of document `text`, as it is written there, with the namespace prefixes
/// it uses that an ancestor declares (other than `xlink`, which [`check`] declares) declared on it.
pub fn element_xml(n: roxmltree::Node, text: &str) -> Option<String> {
    let xml = text.get(n.range())?;
    let name_end = 1 + xml.get(1..)?.find(|c: char| c.is_whitespace() || c == '>' || c == '/')?;
    let own = attribute_names(xml.get(name_end..)?);
    let mut declared = String::new();
    for ns in n.namespaces() {
        let Some(prefix) = ns.name() else { continue };
        let attr = format!("xmlns:{prefix}");
        if prefix == "xml" || (prefix == "xlink" && ns.uri() == XLINK_NS) || !xml.contains(&format!("{prefix}:")) || own.contains(&attr.as_str()) {
            continue;
        }
        let uri = ns.uri().replace('&', "&amp;").replace('<', "&lt;").replace('"', "&quot;");
        declared.push_str(&format!(" {attr}=\"{uri}\""));
    }
    Some(format!("{}{declared}{}", xml.get(..name_end)?, xml.get(name_end..)?))
}

/// The attribute names of a start tag, `rest` the text after its element name: names, then `=`
/// and a quoted value, with white space allowed around the `=`.
fn attribute_names(rest: &str) -> Vec<&str> {
    let mut names = vec![];
    let mut s = rest;
    loop {
        s = s.trim_start();
        if s.is_empty() || s.starts_with('>') || s.starts_with('/') {
            return names;
        }
        let end = s.find(|c: char| c.is_whitespace() || c == '=' || c == '>' || c == '/').unwrap_or(s.len());
        let (name, after) = s.split_at(end);
        let Some(after) = after.trim_start().strip_prefix('=') else { return names };
        let after = after.trim_start();
        let Some(quote) = after.chars().next().filter(|c| *c == '"' || *c == '\'') else { return names };
        let Some(close) = after.get(1..).and_then(|v| v.find(quote)) else { return names };
        names.push(name);
        s = after.get(close + 2..).unwrap_or_default();
    }
}

/// The primitives of filter `xml` as usvg reads them (their `Debug` forms), to tell filters with
/// the same primitives apart.
pub fn primitive_forms(xml: &str) -> Option<Vec<String>> {
    let drawn = with_id(xml, DRAWN_ID)?;
    let svg = format!(
        "<svg xmlns=\"{SVG_NS}\" xmlns:xlink=\"{XLINK_NS}\" width=\"10\" height=\"10\"><defs>{drawn}</defs><rect width=\"10\" height=\"10\" filter=\"url(#{DRAWN_ID})\"/></svg>"
    );
    let tree = usvg::Tree::from_str(&svg, &options()).ok()?;
    fn first(g: &usvg::Group) -> Option<Arc<usvg::filter::Filter>> {
        g.filters().first().cloned().or_else(|| {
            g.children().iter().find_map(|n| match n {
                usvg::Node::Group(g) => first(g),
                _ => None,
            })
        })
    }
    Some(first(tree.root())?.primitives().iter().map(|p| format!("{:?}", p.kind())).collect())
}

/// Is filter `xml` at most [`SvgFilter::MAX_XML`] bytes? Checked before the text is copied.
fn within_size(xml: &str) -> Result<(), String> {
    if xml.len() > SvgFilter::MAX_XML {
        return Err(format!("an SVG filter is at most {} KB", SvgFilter::MAX_XML / 1024));
    }
    Ok(())
}

/// Filter `xml` with its `id` changed to `id` (written escaped), up to [`MAX_DRAWN_XML`] bytes.
/// Callers check user input with [`check`] before using the result as a filter.
pub fn with_id(xml: &str, id: &str) -> Option<String> {
    if xml.len() > MAX_DRAWN_XML {
        return None;
    }
    let (wrapped, head) = wrap(xml);
    let doc = read_unsized(xml, &wrapped).ok()?;
    let f = filter_element(&doc).ok()?;
    let at = f.attributes().find(|a| a.name() == "id" && a.namespace().is_none())?.range_value();
    let (start, end) = (at.start.checked_sub(head)?, at.end.checked_sub(head)?);
    // The value may sit in single quotes: those are escaped too.
    let id = escaped(id).replace('\'', "&apos;");
    Some(format!("{}{}{}", xml.get(..start)?, id, xml.get(end..)?))
}

/// usvg's options for drawing a filter: images only from `data:` URLs, and only the ones
/// [`raster_image_sized`] takes.
fn options() -> usvg::Options<'static> {
    options_with(None, None)
}

/// Take `pixels` from the count `left`: false, leaving it as it is, when fewer are left.
fn take(left: &AtomicU64, pixels: u64) -> bool {
    let mut now = left.load(Ordering::Relaxed);
    loop {
        let Some(rest) = now.checked_sub(pixels) else { return false };
        match left.compare_exchange_weak(now, rest, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => return true,
            Err(seen) => now = seen,
        }
    }
}

/// [`options`], also taking the PNG `source` (the object's pixels that [`SvgFilterFx::apply`]
/// gives the filter) whatever its size. The filter's own images share [`MAX_IMAGE_PIXELS`]. An
/// image that exceeds the remaining budget is left out and `over` is set.
fn options_with(source: Option<Arc<Vec<u8>>>, over: Option<Arc<AtomicBool>>) -> usvg::Options<'static> {
    let left = AtomicU64::new(MAX_IMAGE_PIXELS);
    usvg::Options {
        image_href_resolver: usvg::ImageHrefResolver {
            resolve_data: Box::new(move |_, data, _| match &source {
                Some(s) if **s == *data => Some(usvg::ImageKind::PNG(data)),
                _ => {
                    let (kind, pixels) = raster_image_sized(data)?;
                    if !take(&left, pixels) {
                        if let Some(o) = &over {
                            o.store(true, Ordering::Relaxed);
                        }
                        return None;
                    }
                    Some(kind)
                }
            }),
            resolve_string: Box::new(|_, _| None),
        },
        ..Default::default()
    }
}

/// Filter `xml` (a rebuilt one, [`rebuilt`]) with attributes `set` on its `<filter>` element.
fn with_attributes(xml: &str, set: &[(&str, f64)]) -> Option<String> {
    let (wrapped, head) = wrap(xml);
    let doc = roxmltree::Document::parse(&wrapped).ok()?;
    let f = filter_element(&doc).ok()?;
    let name_end = f.range().start.checked_sub(head)? + 1 + f.tag_name().name().len();
    let mut edits: Vec<(usize, usize, String)> = vec![];
    for (k, v) in set {
        if !v.is_finite() {
            return None;
        }
        match f.attributes().find(|a| a.name() == *k && a.namespace().is_none()) {
            Some(a) => {
                let r = a.range_value();
                edits.push((r.start.checked_sub(head)?, r.end.checked_sub(head)?, format!("{v}")));
            }
            None => edits.push((name_end, name_end, format!(" {k}=\"{v}\""))),
        }
    }
    edits.sort_by_key(|e| std::cmp::Reverse(e.0));
    let mut out = xml.to_string();
    for (start, end, text) in edits {
        out = format!("{}{text}{}", out.get(..start)?, out.get(end..)?);
    }
    Some(out)
}

/// Does `g` or a group in it carry a filter?
fn has_filter(g: &usvg::Group) -> bool {
    !g.filters().is_empty() || g.children().iter().any(|n| matches!(n, usvg::Node::Group(g) if has_filter(g)))
}

/// A length of a filter region.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Length {
    /// In user units.
    User(f64),
    /// A fraction of the object's box.
    Box(f64),
}

/// A length of a filter region, whose default is the fraction `default`: in the object's bounding
/// box (`user` false), a fraction or a percentage of the box, the default for anything else; in
/// user space, a number or an absolute length (px, pt, pc, in, cm, mm) in user units. In user
/// space a percentage, a length in em or ex, and a missing or unreadable length are measured
/// against the object's box as well (the effect has no viewport or font of its own): a
/// percentage as that share of the box, the others as the default.
fn region_length(v: Option<&str>, default: f64, user: bool) -> Length {
    let Some(v) = v.map(str::trim) else { return Length::Box(default) };
    // Relative to the font (before the number is read: `e` also starts an exponent).
    if v.ends_with("em") || v.ends_with("ex") {
        return Length::Box(default);
    }
    let split = v.find(|c: char| !(c.is_ascii_digit() || matches!(c, '.' | '-' | '+' | 'e' | 'E'))).unwrap_or(v.len());
    let (number, unit) = v.split_at(split);
    let Some(number) = number.parse::<f64>().ok().filter(|n| n.is_finite()) else { return Length::Box(default) };
    let per_unit = match (unit.trim(), user) {
        ("", false) => return Length::Box(number),
        ("%", _) => return Length::Box(number * 0.01),
        (_, false) => return Length::Box(default),
        ("" | "px", true) => 1.0,
        ("pt", true) => 96.0 / 72.0,
        ("pc", true) => 16.0,
        ("in", true) => 96.0,
        ("cm", true) => 96.0 / 2.54,
        ("mm", true) => 96.0 / 25.4,
        _ => return Length::Box(default),
    };
    Length::User(number * per_unit)
}

/// An SVG filter effect: the filter's XML, how many points one of its units is, where its user
/// space's origin lies in the document, and its region.
#[derive(Clone, Debug, PartialEq)]
pub struct SvgFilterFx {
    /// The filter as it is drawn and written ([`rebuilt`]).
    xml: Arc<str>,
    scale: f64,
    origin: Vec2,
    /// Its region (filter effects region): x, y, width, height.
    region: [Length; 4],
    /// Is the region in user space with a length measured against the object's box? It is then
    /// written into the filter before it is drawn or written ([`Self::drawn_xml`]).
    measured: bool,
    /// The largest `feMorphology` radius or `kernelUnitLength`, and the largest `stdDeviation`, in
    /// user units.
    window: f64,
    blur: f64,
    /// `primitiveUnits="objectBoundingBox"`: those lengths are fractions of the object's box.
    box_units: bool,
}

/// The `origin` of an SVG filter effect's parameters `p`: `[x, y]` in points, `[0, 0]` when absent.
fn origin_param(p: &Value) -> Vec2 {
    let at = |i: usize| p.get("origin").and_then(|o| o.get(i)).and_then(Value::as_f64).filter(|v| v.is_finite()).unwrap_or(0.0);
    Vec2::new(at(0), at(1))
}

impl SvgFilterFx {
    /// The effect's parameters `p` (defaults merged in): `xml`, the `<filter>` element, `scale`
    /// and `origin`. `None` when `xml` isn't a filter [`check`] takes, however the effect came to
    /// hold it (an opened file, a script): the object is then drawn, printed and exported without it.
    pub(crate) fn parse(p: &Value) -> Option<Self> {
        let xml = checked(text(p, "xml", ""))?;
        let (wrapped, _) = wrap(&xml);
        let doc = roxmltree::Document::parse(&wrapped).ok()?;
        let f = filter_element(&doc).ok()?;
        let user = f.attribute("filterUnits").is_some_and(|u| u.trim() == "userSpaceOnUse");
        let region = [("x", -0.1), ("y", -0.1), ("width", 1.2), ("height", 1.2)].map(|(k, d)| region_length(f.attribute(k), d, user));
        let measured = user && region.iter().any(|l| matches!(l, Length::Box(_)));
        let largest = |attrs: &[&str]| {
            f.descendants()
                .flat_map(|n| attrs.iter().filter_map(move |a| n.attribute(*a)))
                .filter_map(numbers)
                .flatten()
                .map(f64::abs)
                .fold(0.0, f64::max)
        };
        let (window, blur) = (largest(&["radius", "kernelUnitLength"]), largest(&["stdDeviation"]));
        let box_units = f.attribute("primitiveUnits").is_some_and(|u| u.trim() == "objectBoundingBox");
        let scale = num(p, "scale", 1.0).clamp(1e-3, 1e3);
        Some(Self { xml, scale, origin: origin_param(p), region, measured, window, blur, box_units })
    }

    /// The filter's region in its user space for an object whose box there is `b`: lengths in user
    /// units as they are, the others as fractions of the box (x and y from its corner).
    fn region_in(&self, b: Rect) -> Rect {
        let [x, y, w, h] = self.region;
        let at = |l: Length, from: f64, size: f64| match l {
            Length::User(v) => v,
            Length::Box(k) => from + k * size,
        };
        let len = |l: Length, size: f64| match l {
            Length::User(v) => v,
            Length::Box(k) => k * size,
        };
        let (x0, y0) = (at(x, b.x0, b.width()), at(y, b.y0, b.height()));
        Rect::new(x0, y0, x0 + len(w, b.width()), y0 + len(h, b.height()))
    }

    /// The filter as it is drawn for an object whose box in its user space is `b`: a region
    /// measured against the box is written into it in user units.
    fn drawn_xml(&self, b: Rect) -> Option<String> {
        self.with_region(&self.xml, b)
    }

    /// Filter `xml` (this one's, in a form) with a region measured against the box `b` written
    /// into it in user units.
    fn with_region(&self, xml: &str, b: Rect) -> Option<String> {
        if !self.measured {
            return Some(xml.to_string());
        }
        let r = self.region_in(b);
        with_attributes(xml, &[("x", r.x0), ("y", r.y0), ("width", r.width()), ("height", r.height())])
    }

    /// The filter as SVG output writes it (one unit a point, [`Self::in_points`]) for an object
    /// whose box in the document is `bounds`: in its SVG 1.1 form ([`svg11`]), `None` when SVG 1.1
    /// can't hold it ([`Self::writes_as_svg11`]).
    pub fn written_xml(&self, bounds: Rect) -> Option<String> {
        self.with_region(&svg11(&self.xml)?, (bounds - self.origin).scale_from_origin(1.0 / self.scale))
    }

    /// Can SVG 1.1 output write the filter as a `<filter>` element ([`svg11`])? When it can't, SVG
    /// output treats it as it treats the other pixel effects.
    pub fn writes_as_svg11(&self) -> bool {
        svg11(&self.xml).is_some()
    }

    /// How finely the filter runs on a raster whose pixels are `space.px` points wide, for an
    /// object whose box is `box_px` raster pixels at its widest: the share of a raster pixel one of
    /// its pixels is (at most 1). It is no finer than the Document Raster Effects resolution, and
    /// coarse enough that a morphology radius or kernel unit spans at most 32 of its pixels and a
    /// blur's standard deviation at most 512.
    pub fn resolution(&self, space: &PixelSpace, box_px: f64) -> f64 {
        if !(space.px.is_finite() && space.px > 0.0) {
            return 1.0;
        }
        let mut s: f64 = if space.line.is_finite() && space.line > 0.0 { (space.px / space.line).min(1.0) } else { 1.0 };
        // A length spans length × scale / px raster pixels, or a fraction of the box.
        for (length, most) in [(self.window, MAX_WINDOW_PX), (self.blur, MAX_BLUR_PX)] {
            let px = if self.box_units { length * box_px.max(0.0) } else { length * self.scale / space.px };
            if px * s > most {
                s = most / px;
            }
        }
        // (However small: a coarser raster keeps the caps.)
        if s.is_finite() { s.clamp(f64::MIN_POSITIVE, 1.0) } else { 1.0 }
    }

    /// Where the filter's user space has its origin in the document, in points.
    pub fn origin(&self) -> Vec2 {
        self.origin
    }

    /// The `<filter>` element's XML as it is drawn and written: rebuilt from its filter elements and
    /// their filter attributes alone.
    pub fn xml(&self) -> &str {
        &self.xml
    }

    /// Is its region in user space measured against the object's box ([`region_length`])? SVG
    /// output keeps such a filter as a filter only when no other raster effect comes before it.
    pub fn measured(&self) -> bool {
        self.measured
    }

    /// Does one unit of the filter's user space measure one point (its `scale` is 1)?
    pub fn in_points(&self) -> bool {
        self.scale == 1.0
    }

    /// How far beyond `bounds` (document units) the filter can draw: its region's reach past them.
    pub fn outset(&self, bounds: Rect) -> f64 {
        let b = (bounds - self.origin).scale_from_origin(1.0 / self.scale);
        let region = self.region_in(b).scale_from_origin(self.scale) + self.origin;
        let reach = [bounds.x0 - region.x0, bounds.y0 - region.y0, region.x1 - bounds.x1, region.y1 - bounds.y1].into_iter().fold(0.0, f64::max);
        if reach.is_finite() { reach.min(MAX_OUTSET) } else { 0.0 }
    }

    /// Run the filter on `data`, `w` × `h` premultiplied RGBA8 pixels lying in the document as
    /// `space` says, its source graphic, whose object bounding box is the box of the pixels that
    /// aren't transparent (the whole raster when all are). Leaves them as they are when the sizes
    /// don't match and when the filter can't be drawn.
    pub fn apply(&self, data: &mut [u8], w: usize, h: usize, space: &PixelSpace) {
        if w == 0 || h == 0 || w.checked_mul(h).and_then(|n| n.checked_mul(4)) != Some(data.len()) {
            return;
        }
        // Art that draws no pixels still has a box: the whole raster stands for it.
        let (x0, y0, x1, y1) = drawn_box(data, w, h).unwrap_or((0, 0, w, h));
        let s = self.resolution(space, (x1 - x0).max(y1 - y0) as f64);
        let (sw, sh) = ((w as f64 * s).ceil().max(1.0) as u32, (h as f64 * s).ceil().max(1.0) as u32);
        let Some(png) = png_of(data, w, x0, y0, x1, y1).map(Arc::new) else { return };
        let k = self.scale;
        // Filter user space → the small canvas, and the image's pixels → filter user space.
        let outer = Affine::scale(s) * space.to_doc.inverse() * Affine::translate(self.origin) * Affine::scale(k);
        let place = Affine::scale(1.0 / k) * Affine::translate(-self.origin) * space.to_doc * Affine::translate(Vec2::new(x0 as f64, y0 as f64));
        let b = place.transform_rect_bbox(Rect::new(0.0, 0.0, (x1 - x0) as f64, (y1 - y0) as f64));
        let Some(drawn) = self.drawn_xml(b).and_then(|xml| with_id(&xml, DRAWN_ID)) else { return };
        let svg = format!(
            "<svg xmlns=\"{SVG_NS}\" xmlns:xlink=\"{XLINK_NS}\" width=\"{sw}\" height=\"{sh}\" viewBox=\"0 0 {sw} {sh}\"><defs>{drawn}</defs><g transform=\"{}\"><g filter=\"url(#{DRAWN_ID})\"><image width=\"{}\" height=\"{}\" preserveAspectRatio=\"none\" transform=\"{}\" xlink:href=\"data:image/png;base64,{}\"/></g></g></svg>",
            matrix(outer),
            x1 - x0,
            y1 - y0,
            matrix(place),
            base64(&png)
        );
        let Ok(tree) = usvg::Tree::from_str(&svg, &options_with(Some(png), None)) else { return };
        if !has_filter(tree.root()) {
            return;
        }
        let Some(mut small) = tiny_skia::Pixmap::new(sw, sh) else { return };
        resvg::render(&tree, tiny_skia::Transform::identity(), &mut small.as_mut());
        let out = if s < 1.0 {
            let Some(mut full) = tiny_skia::Pixmap::new(w as u32, h as u32) else { return };
            let paint = tiny_skia::PixmapPaint { quality: tiny_skia::FilterQuality::Bilinear, ..Default::default() };
            let up = (1.0 / s) as f32;
            full.draw_pixmap(0, 0, small.as_ref(), &paint, tiny_skia::Transform::from_scale(up, up), None);
            full
        } else {
            small
        };
        if out.data().len() == data.len() {
            data.copy_from_slice(out.data());
        }
    }
}

/// The farthest a filter's region reaches beyond the object, in points.
const MAX_OUTSET: f64 = 10_000.0;

/// The box of the pixels of `data` (`w` wide) that aren't transparent: columns x0..x1, rows y0..y1.
fn drawn_box(data: &[u8], w: usize, h: usize) -> Option<(usize, usize, usize, usize)> {
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0, 0);
    for (i, p) in data.as_chunks::<4>().0.iter().enumerate() {
        if p[3] > 0 {
            let (x, y) = (i % w, i / w);
            (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x + 1), y1.max(y + 1));
        }
    }
    (x1 > x0 && y1 > y0).then_some((x0, y0, x1, y1))
}

/// The pixels of `data` (premultiplied RGBA8, `w` wide) in columns x0..x1 and rows y0..y1, as a PNG.
fn png_of(data: &[u8], w: usize, x0: usize, y0: usize, x1: usize, y1: usize) -> Option<Vec<u8>> {
    use image::ImageEncoder;
    let px = data.as_chunks::<4>().0;
    let mut straight = Vec::with_capacity((x1 - x0) * (y1 - y0) * 4);
    for y in y0..y1 {
        for p in px.get(y * w + x0..y * w + x1)? {
            let a = u32::from(p[3]);
            let un = |c: u8| (u32::from(c) * 255 + a / 2).checked_div(a).map_or(0, |v| v.min(255) as u8);
            straight.extend_from_slice(&[un(p[0]), un(p[1]), un(p[2]), p[3]]);
        }
    }
    let mut png = vec![];
    let encoder = image::codecs::png::PngEncoder::new_with_quality(
        &mut png,
        image::codecs::png::CompressionType::Fast,
        image::codecs::png::FilterType::NoFilter,
    );
    encoder.write_image(&straight, (x1 - x0) as u32, (y1 - y0) as u32, image::ExtendedColorType::Rgba8).ok()?;
    Some(png)
}

/// `a` as an SVG `matrix(…)`.
fn matrix(a: Affine) -> String {
    let [a, b, c, d, e, f] = a.as_coeffs();
    format!("matrix({a} {b} {c} {d} {e} {f})")
}

/// `bytes` in base64.
fn base64(bytes: &[u8]) -> String {
    const ABC: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [0, 1, 2].map(|i| chunk.get(i).copied().unwrap_or(0));
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for (i, shift) in [18, 12, 6, 0].into_iter().enumerate() {
            out.push(if i <= chunk.len() { char::from(ABC[(n >> shift & 63) as usize]) } else { '=' });
        }
    }
    out
}
