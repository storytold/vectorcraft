//! `<text>` import.
//!
//! usvg only converts text when it has fonts, and a font database is too expensive to load (and
//! unavailable on wasm), so text is read from the XML. Before usvg runs, [`prepare`] replaces each
//! `<text>` element with a placeholder `<g>` that carries the element's id, transform, conditional
//! attributes, opacity, blending, clip, mask and filter. The group holds a rectangle over the text's
//! bounds (id `{prefix}{i}`, filled with colour `i` so it is still found inside `<use>` instances,
//! where usvg drops ids), followed by one rectangle per `url(#…)` paint, painted with it. usvg then
//! keeps the text's z-order, parent groups, clips, masks and `<use>` instances, and resolves its
//! gradients and patterns in the text's user space; the importer swaps each placeholder for the
//! [`PendingText`] it stands for.
//!
//! Characters keep SVG's per-character `x`/`y`/`dx`/`dy`/`rotate` lists: a character with an
//! absolute `x` that moves down by at least half a line starts a new line (the leading becomes the
//! line spacing), other vertical moves become baseline shift, horizontal moves become kerning (an
//! absolute `x` is measured against the laid-out text), and rotations become character rotation.
//! `<textPath>` becomes type on a path; vertical `writing-mode` becomes type on a vertical path.
//!
//! Type takes the size it draws at: the scale of the transforms and `viewBox` above a `<text>`
//! moves into its sizes ([`size_scale`], [`fold_scale`]), and the rest (a stretch, skew,
//! rotation or reflection) stays its transform.

use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt::Write as _;
use std::ops::{Range, RangeInclusive};
use std::rc::Rc;
use std::str::FromStr;

use usvg::roxmltree;
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{CharStyle, Dash, Justify, LineCap, LineJoin, ParaDirection, ParaStyle, StrokeLayer, TextKind, TextObject, TextRun};
use vectorcraft_geom::kurbo::ParamCurveArclen;
use vectorcraft_geom::{Affine, BezPath, PathData, Point, Rect};
use vectorcraft_text::{FontDb, TextLayout};

use super::css::{Styles, XNode};
use super::{DEFAULT_FONT_SIZE, PT_PER_IN, href, vector_effect};
use crate::xml_escape;

const SVG_NS: &str = "http://www.w3.org/2000/svg";

/// Most line breaks one absolutely positioned character adds before itself.
const MAX_LINE_GAP: f64 = 10_000.0;

/// A whitespace/comma separated list.
fn list(v: &str) -> impl Iterator<Item = &str> {
    v.split(|c: char| c.is_whitespace() || c == ',').filter(|t| !t.is_empty())
}

/// Style name for a CSS weight (rounded to the nearest hundred) and slant.
fn style_name(weight: u16, italic: bool) -> String {
    const NAMES: [&str; 9] = ["Thin", "ExtraLight", "Light", "Regular", "Medium", "SemiBold", "Bold", "ExtraBold", "Black"];
    let name = NAMES.get((weight.clamp(100, 900) as usize + 50) / 100 - 1).copied().unwrap_or("Regular");
    match (name, italic) {
        ("Regular", true) => "Italic".into(),
        (n, true) => format!("{n} Italic"),
        (n, false) => n.into(),
    }
}

/// The axis settings of a `font-variation-settings` value (`"wght" 650, "wdth" 87.5`), by tag:
/// none for `normal` and `initial`. `None` when an item isn't a quoted tag of four printable ASCII
/// characters and a finite number, as CSS then ignores the declaration.
fn variation_settings(v: &str) -> Option<std::collections::BTreeMap<String, f64>> {
    let mut out = std::collections::BTreeMap::new();
    if ["normal", "initial"].iter().any(|k| v.trim().eq_ignore_ascii_case(k)) {
        return Some(out);
    }
    for item in v.split(',') {
        let item = item.trim();
        let quote = item.chars().next().filter(|c| *c == '"' || *c == '\'')?;
        let (tag, value) = item.get(1..)?.split_once(quote)?;
        let value = value.trim().parse::<f64>().ok().filter(|v| v.is_finite())?;
        if tag.len() != 4 || !tag.bytes().all(|b| (0x20..=0x7e).contains(&b)) {
            return None;
        }
        out.insert(tag.to_string(), value);
    }
    Some(out)
}

fn solid(c: svgtypes::Color) -> Paint {
    Paint::solid(Color::rgb8(c.red, c.green, c.blue))
}

fn parse_transform(s: Option<&str>) -> Affine {
    s.and_then(|s| svgtypes::Transform::from_str(s).ok()).map(|t| Affine::new([t.a, t.b, t.c, t.d, t.e, t.f])).unwrap_or(Affine::IDENTITY)
}

/// Character attributes an element gives its text.
struct ElemStyle {
    style: CharStyle,
    /// `url(#…)` fill and stroke paints (resolved by usvg).
    fill: Option<String>,
    stroke: Option<String>,
    /// Extra space after each space character (`word-spacing`).
    word_spacing: f64,
    /// `paint-order` paints the stroke before the fill.
    stroke_first: bool,
}

/// One character of a text and the adjustments its position needs.
struct Cell {
    ch: char,
    style: CharStyle,
    fill: Option<usize>,
    stroke: Option<usize>,
    /// Space added after the character, in points (becomes manual kerning).
    kern: f64,
    /// Its stroke paints before its fill (`paint-order`).
    stroke_first: bool,
}

impl Cell {
    fn final_style(&self) -> CharStyle {
        let mut st = self.style.clone();
        st.baseline_shift = finite(st.baseline_shift);
        if self.kern.is_finite() && self.kern.abs() > 1e-9 && st.size > 0.0 {
            st.kerning = Some(st.kerning.unwrap_or(0.0) + self.kern / st.size * 1000.0);
        }
        st
    }
}

/// The characters of a `<text>` after white-space processing.
struct Chars<'a, 'i> {
    /// Each character and the element it comes from.
    chars: Vec<(char, XNode<'a, 'i>)>,
    /// Every element in tree order with the range of characters inside it.
    spans: Vec<(XNode<'a, 'i>, Range<usize>)>,
    /// The first `<textPath>`.
    path: Option<XNode<'a, 'i>>,
}

/// A line break before character `at`: `count` newlines, `leading` apart.
struct Break {
    at: usize,
    count: usize,
    leading: f64,
}

/// A `<text>` element read from the XML, waiting for usvg to place it.
#[derive(Clone)]
pub(super) struct PendingText {
    /// The element's `id`.
    pub name: String,
    /// The text; `xf` maps text space into the element's user space.
    pub obj: TextObject,
    /// For each run, the index of its fill's and stroke's `url(#…)` paint, if any.
    pub servers: Vec<(Option<usize>, Option<usize>)>,
    /// Number of `url(#…)` paints (the placeholder paths after the main one, in order).
    pub paints: usize,
    /// `vector-effect="non-scaling-stroke"`: the character strokes keep their width on screen.
    pub non_scaling: bool,
    /// The character stroke `paint-order` puts under the characters (in text space; it becomes
    /// the object's own stroke below the Characters row) and the index of its `url(#…)` paint.
    pub under: Option<(StrokeLayer, Option<usize>)>,
    /// The placeholder rectangle in the element's user space.
    marker: Rect,
}

/// The texts [`prepare`] replaced, by placeholder index.
#[derive(Default)]
pub(super) struct TextSlots {
    prefix: String,
    pub texts: Vec<PendingText>,
}

impl TextSlots {
    /// The text a placeholder's main path stands for: by its id, or inside `<use>` instances
    /// (where usvg drops ids) by its colour (the index) and its rectangle.
    pub fn find(&self, p: &usvg::Path) -> Option<usize> {
        if !p.id().is_empty() {
            return p.id().strip_prefix(&self.prefix)?.parse().ok();
        }
        let Some(usvg::Paint::Color(c)) = p.fill().map(|f| f.paint()) else { return None };
        if p.stroke().is_some() || p.data().segments().count() != 5 {
            return None;
        }
        let i = (c.red as usize) << 16 | (c.green as usize) << 8 | c.blue as usize;
        let m = self.texts.get(i)?.marker;
        let b = p.data().bounds();
        let near = |a: f32, b: f64| (a as f64 - b).abs() <= 1e-3 * b.abs().max(1.0);
        (near(b.left(), m.x0) && near(b.top(), m.y0) && near(b.right(), m.x1) && near(b.bottom(), m.y1)).then_some(i)
    }
}

/// Replace every `<text>` usvg would draw by a placeholder group (see the module docs).
pub(super) fn prepare<'s>(svg: &'s str, xml: &roxmltree::Document, dpi: f64, warnings: &mut Vec<String>) -> (Cow<'s, str>, TextSlots) {
    let mut prefix = String::from("vectorcraft-text-");
    while xml.descendants().any(|n| n.attribute("id").is_some_and(|id| id.starts_with(&prefix))) {
        prefix.insert(0, '_');
    }
    let mut slots = TextSlots { prefix, texts: vec![] };
    let cx = Ctx::new(xml, dpi);
    let mut out = String::new();
    let mut copied = 0;
    let is_text = |n: &XNode| n.is_element() && matches!(n.tag_name().name(), "text" | "flowRoot") && n.tag_name().namespace() == Some(SVG_NS);
    for t in xml.descendants().filter(is_text) {
        let label = t.attribute("id").map_or_else(|| "a text".to_string(), |id| format!("text '{id}'"));
        if t.ancestors().skip(1).any(|a| a.is_element() && a.tag_name().name() == "clipPath") {
            warnings.push(format!("{label} in a clip path ignored"));
            continue;
        }
        let r = t.range();
        // The element's qualified name (`text` or `svg:text`), so the placeholder lands in the same
        // namespace; skip anything whose source range isn't the element itself (entity expansions).
        let Some(qname) = svg
            .get(r.start..r.end)
            .and_then(|s| s.strip_prefix('<'))
            .map(|s| s.split(|c: char| c.is_whitespace() || c == '>' || c == '/').next().unwrap_or(""))
        else {
            continue;
        };
        let flowed = t.tag_name().name() == "flowRoot";
        let Some(q) = qname.strip_suffix(t.tag_name().name()).filter(|q| q.is_empty() || q.ends_with(':')) else { continue };
        if cx.css.own(t, "display").as_deref() == Some("none") || r.start < copied {
            continue;
        }
        let built = if flowed { cx.build_flow(t, &label, warnings) } else { cx.build(xml, t, &label, warnings) };
        let Some((text, paints)) = built else { continue };
        out.push_str(&svg[copied..r.start]);
        placeholder(&mut out, q, &cx.css, t, &slots.prefix, slots.texts.len(), text.marker, &paints);
        copied = r.end;
        slots.texts.push(text);
    }
    if slots.texts.is_empty() {
        return (Cow::Borrowed(svg), slots);
    }
    out.push_str(&svg[copied..]);
    (Cow::Owned(out), slots)
}

/// Write the placeholder group for text `t` (namespace prefix `q`), slot `index`.
#[allow(clippy::too_many_arguments)]
fn placeholder(out: &mut String, q: &str, css: &Styles, t: XNode, prefix: &str, index: usize, b: Rect, paints: &[String]) {
    let _ = write!(out, "<{q}g");
    for a in ["id", "transform", "systemLanguage", "requiredFeatures", "requiredExtensions"] {
        if let Some(v) = t.attribute(a) {
            let _ = write!(out, " {a}=\"{}\"", xml_escape(v));
        }
    }
    // The text's own non-inherited properties, overriding any rule that matches the `<g>`.
    let mut style = String::new();
    for (p, initial) in
        [("opacity", "1"), ("clip-path", "none"), ("mask", "none"), ("filter", "none"), ("mix-blend-mode", "normal"), ("isolation", "auto")]
    {
        let _ = write!(style, "{p}:{};", css.own(t, p).as_deref().unwrap_or(initial));
    }
    if let Some(v) = css.own(t, "visibility") {
        let _ = write!(style, "visibility:{v};");
    }
    let _ = write!(out, " style=\"{}display:inline\">", xml_escape(&style));
    let d = format!("M{} {}H{}V{}H{}Z", b.x0, b.y0, b.x1, b.y1, b.x0);
    const REST: &str = "stroke:none;opacity:1;display:inline;marker:none;clip-path:none;mask:none;filter:none";
    let _ = write!(out, "<{q}path id=\"{prefix}{index}\" style=\"fill:#{:06x};{REST}\" d=\"{d}\"/>", index & 0xff_ffff);
    for p in paints {
        let _ = write!(out, "<{q}path style=\"fill:{};{REST}\" d=\"{d}\"/>", xml_escape(p));
    }
    let _ = write!(out, "</{q}g>");
}

/// Computed text properties of the document's elements.
struct Ctx {
    css: Styles,
    /// Pixels per inch absolute lengths convert at ([`super::RootUnits::dpi`]).
    dpi: f64,
    /// Viewport size in user units (the base of `x`/`y` percentages).
    viewport: (f64, f64),
    sizes: RefCell<HashMap<roxmltree::NodeId, f64>>,
    weights: RefCell<HashMap<roxmltree::NodeId, u16>>,
    elems: RefCell<HashMap<roxmltree::NodeId, Rc<ElemStyle>>>,
}

impl Ctx {
    fn new(xml: &roxmltree::Document, dpi: f64) -> Self {
        let mut cx = Self {
            css: Styles::new(xml),
            dpi,
            viewport: (100.0, 100.0),
            sizes: RefCell::default(),
            weights: RefCell::default(),
            elems: RefCell::default(),
        };
        let root = xml.root_element();
        let dim = |a: &str| root.attribute(a).and_then(|v| cx.length(v, DEFAULT_FONT_SIZE, 0.0)).filter(|v| *v > 0.0);
        cx.viewport = match root.attribute("viewBox").and_then(|v| svgtypes::ViewBox::from_str(v).ok()) {
            Some(vb) if vb.w > 0.0 && vb.h > 0.0 => (vb.w, vb.h),
            _ => (dim("width").unwrap_or(100.0), dim("height").unwrap_or(100.0)),
        };
        cx
    }

    /// A length in user units: absolute units convert at [`Self::dpi`], `em` and `ex` are relative
    /// to `em` (the font size) and percentages to `pct`.
    fn length(&self, v: &str, em: f64, pct: f64) -> Option<f64> {
        use svgtypes::LengthUnit as L;
        let l = svgtypes::Length::from_str(v.trim()).ok()?;
        let n = l.number;
        let v = match l.unit {
            L::None | L::Px => n,
            L::Pt => n * self.dpi / PT_PER_IN,
            L::Pc => n * self.dpi / 6.0,
            L::In => n * self.dpi,
            L::Cm => n * self.dpi / 2.54,
            L::Mm => n * self.dpi / 25.4,
            L::Em => n * em,
            L::Ex => n * em / 2.0,
            L::Percent => n / 100.0 * pct,
        };
        v.is_finite().then_some(v)
    }

    /// A `font-size` value given the parent's computed size (CSS keywords are relative to `medium`).
    fn parse_font_size(&self, v: &str, parent: f64) -> Option<f64> {
        let k = match v.trim() {
            "xx-small" => 3.0 / 5.0,
            "x-small" => 3.0 / 4.0,
            "small" => 8.0 / 9.0,
            "medium" => 1.0,
            "large" => 6.0 / 5.0,
            "x-large" => 3.0 / 2.0,
            "xx-large" => 2.0,
            "larger" => return Some(parent * 1.2),
            "smaller" => return Some(parent / 1.2),
            v => return self.length(v, parent, parent).filter(|s| *s > 0.0),
        };
        Some(DEFAULT_FONT_SIZE * k)
    }

    /// Computed `font-size` of an element.
    fn font_size(&self, n: XNode) -> f64 {
        if let Some(s) = self.sizes.borrow().get(&n.id()) {
            return *s;
        }
        let parent = n.parent_element().map_or(DEFAULT_FONT_SIZE, |p| self.font_size(p));
        let s = self.css.own(n, "font-size").and_then(|v| self.parse_font_size(&v, parent)).unwrap_or(parent);
        self.sizes.borrow_mut().insert(n.id(), s);
        s
    }

    /// Computed `font-weight` of an element (100–900).
    fn weight(&self, n: XNode) -> u16 {
        if let Some(w) = self.weights.borrow().get(&n.id()) {
            return *w;
        }
        let parent = n.parent_element().map_or(400, |p| self.weight(p));
        let w = match self.css.own(n, "font-weight").as_deref() {
            Some("normal") => 400,
            Some("bold") => 700,
            Some("bolder") => match parent {
                ..=349 => 400,
                350..=549 => 700,
                _ => 900,
            },
            Some("lighter") => match parent {
                ..=549 => 100,
                550..=749 => 400,
                _ => 700,
            },
            Some(v) => v.parse::<f64>().ok().filter(|w| (1.0..=1000.0).contains(w)).map_or(parent, |w| w.round() as u16),
            None => parent,
        };
        self.weights.borrow_mut().insert(n.id(), w);
        w
    }

    /// A fill or stroke: a ready paint, or a `url(#…)` paint for usvg to resolve.
    fn paint(&self, n: XNode, prop: &str) -> (Paint, Option<String>) {
        let initial = if prop == "fill" { Paint::solid(Color::BLACK) } else { Paint::None };
        let Some(v) = self.css.prop(n, prop) else { return (initial, None) };
        match svgtypes::Paint::from_str(&v) {
            Ok(svgtypes::Paint::None) => (Paint::None, None),
            Ok(svgtypes::Paint::Color(c)) => (solid(c), None),
            Ok(svgtypes::Paint::CurrentColor) => {
                (self.css.prop(n, "color").and_then(|c| svgtypes::Color::from_str(&c).ok()).map_or(Paint::solid(Color::BLACK), solid), None)
            }
            Ok(svgtypes::Paint::FuncIRI(..)) => (Paint::None, Some(v)),
            _ => (initial, None),
        }
    }

    /// Character attributes of an element inside `<text>` (cached).
    fn elem(&self, n: XNode) -> Rc<ElemStyle> {
        if let Some(e) = self.elems.borrow().get(&n.id()) {
            return e.clone();
        }
        let css = &self.css;
        let mut st = CharStyle::default();
        if let Some(f) = css.prop(n, "font-family") {
            let fam = f.split(',').next().unwrap_or("").trim().trim_matches(|c| c == '\'' || c == '"').to_string();
            if !fam.is_empty() {
                st.font_family = fam;
            }
        }
        let size = self.font_size(n);
        st.size = size;
        let italic = css.prop(n, "font-style").is_some_and(|s| s == "italic" || s == "oblique");
        st.font_style = style_name(self.weight(n), italic);
        // Inherited: `inherit`, `unset`, `revert` and a value CSS ignores leave the one inherited.
        let inherits = |v: &String| ["inherit", "unset", "revert", "revert-layer"].iter().any(|k| v.trim().eq_ignore_ascii_case(k));
        let own = |a: XNode| css.own(a, "font-variation-settings").filter(|v| !inherits(v)).and_then(|v| variation_settings(&v));
        st.font_axes = n.ancestors().filter(|a| a.is_element()).find_map(own).unwrap_or_default();
        let (fill, mut fill_url) = self.paint(n, "fill");
        st.fill = fill;
        let (stroke, mut stroke_url) = self.paint(n, "stroke");
        if !stroke.is_none() || stroke_url.is_some() {
            st.stroke = stroke;
            st.stroke_width = css.prop(n, "stroke-width").and_then(|w| self.length(&w, size, size)).unwrap_or(1.0);
            st.stroke_cap = match css.prop(n, "stroke-linecap").as_deref() {
                Some("round") => LineCap::Round,
                Some("square") => LineCap::Square,
                _ => LineCap::Butt,
            };
            st.stroke_join = match css.prop(n, "stroke-linejoin").as_deref() {
                Some("round") => LineJoin::Round,
                Some("bevel") => LineJoin::Bevel,
                _ => LineJoin::Miter,
            };
            st.stroke_miter_limit =
                css.prop(n, "stroke-miterlimit").and_then(|m| m.trim().parse::<f64>().ok()).filter(|m| m.is_finite()).map_or(4.0, |m| m.max(1.0));
            let pattern: Vec<f64> = css.prop(n, "stroke-dasharray").map_or(vec![], |d| list(&d).filter_map(|v| self.length(v, size, size)).collect());
            let offset = css.prop(n, "stroke-dashoffset").and_then(|o| self.length(&o, size, size)).unwrap_or(0.0);
            st.stroke_dash = Some(Dash { pattern, offset, align_corners: false }).filter(Dash::is_dashed);
        }
        if let Some(v) = css.prop(n, "letter-spacing").and_then(|v| self.length(&v, size, size))
            && size > 0.0
        {
            st.tracking = v / size * 1000.0;
        }
        if let Some(v) = css.prop(n, "kerning").and_then(|v| self.length(&v, size, size))
            && size > 0.0
        {
            st.kerning = Some(v / size * 1000.0);
        }
        if let Some(d) = css.prop(n, "text-decoration") {
            st.underline = d.contains("underline");
            st.strikethrough = d.contains("line-through");
        }
        // `baseline-shift` isn't inherited but nests: each element shifts its parent's baseline.
        let own = match css.own(n, "baseline-shift").as_deref().map(str::trim) {
            Some("sub") => -0.2 * size,
            Some("super") => 0.4 * size,
            Some(v) => self.length(v, size, size).unwrap_or(0.0),
            None => 0.0,
        };
        let parent = if n.tag_name().name() == "text" { 0.0 } else { n.parent_element().map_or(0.0, |p| self.elem(p).style.baseline_shift) };
        st.baseline_shift = parent + own;
        if css.prop(n, "visibility").is_some_and(|v| v == "hidden" || v == "collapse") {
            (st.fill, st.stroke, fill_url, stroke_url) = (Paint::None, Paint::None, None, None);
        }
        let word_spacing = css.prop(n, "word-spacing").and_then(|v| self.length(&v, size, size)).unwrap_or(0.0);
        let order = css.prop(n, "paint-order").and_then(|v| svgtypes::PaintOrder::from_str(&v).ok()).unwrap_or_default().order;
        let at = |k| order.iter().position(|o| *o == k);
        let stroke_first = at(svgtypes::PaintOrderKind::Stroke) < at(svgtypes::PaintOrderKind::Fill);
        let e = Rc::new(ElemStyle { style: st, fill: fill_url, stroke: stroke_url, word_spacing, stroke_first });
        self.elems.borrow_mut().insert(n.id(), e.clone());
        e
    }

    /// Characters of `el` and its `<tspan>`, `<textPath>` and `<a>` children, with white space
    /// handled like `xml:space` (newlines and tabs become spaces; runs of spaces collapse unless
    /// preserved).
    fn collect<'b, 'i>(&self, el: XNode<'b, 'i>, preserve: bool, out: &mut Chars<'b, 'i>) {
        let start = out.chars.len();
        let slot = out.spans.len();
        out.spans.push((el, start..start));
        for c in el.children() {
            if c.is_text() {
                for ch in c.text().unwrap_or("").chars() {
                    let ch = if matches!(ch, '\n' | '\r' | '\t') { ' ' } else { ch };
                    if !preserve && ch == ' ' && out.chars.last().is_none_or(|(p, _)| *p == ' ') {
                        continue;
                    }
                    out.chars.push((ch, el));
                }
            } else if c.is_element()
                && c.tag_name().namespace() == Some(SVG_NS)
                && matches!(c.tag_name().name(), "tspan" | "textPath" | "a")
                && self.css.own(c, "display").as_deref() != Some("none")
            {
                if c.tag_name().name() == "textPath" {
                    out.path.get_or_insert(c);
                }
                self.collect(c, preserve, out);
            }
        }
        out.spans[slot].1 = start..out.chars.len();
    }

    /// The path a `<textPath>` follows, in the text's user space.
    fn text_path(&self, xml: &roxmltree::Document, tp: XNode) -> Option<BezPath> {
        let mut bp = match tp.attribute("path") {
            Some(d) => BezPath::from_svg(d).ok()?,
            None => {
                let id = href(tp)?.trim().strip_prefix('#')?;
                let el = xml.descendants().find(|n| n.is_element() && n.attribute("id") == Some(id))?;
                let d = match el.tag_name().name() {
                    "path" => el.attribute("d")?.to_string(),
                    _ => self.shape_path(el)?,
                };
                let mut bp = BezPath::from_svg(&d).ok()?;
                bp.apply_affine(parse_transform(el.attribute("transform")));
                bp
            }
        };
        if tp.attribute("side") == Some("right") {
            bp = bp.reverse_subpaths();
        }
        let usable = bp.segments().next().is_some();
        usable.then_some(bp)
    }

    /// The equivalent path of a basic shape (SVG 2 › Basic Shapes), as path data: what a
    /// `<textPath>` that points at it follows. Circles and ellipses start at 3 o'clock and rounded
    /// rects at the end of the top-left corner, all running clockwise; lines and polylines follow
    /// their points. `None` for other elements and for shapes that render nothing.
    fn shape_path(&self, el: XNode) -> Option<String> {
        let (vw, vh) = self.viewport;
        let len = |a: &str, pct: f64| el.attribute(a).and_then(|v| self.length(v, DEFAULT_FONT_SIZE, pct));
        let x = |a: &str| len(a, vw).unwrap_or(0.0);
        let y = |a: &str| len(a, vh).unwrap_or(0.0);
        let points = || -> Vec<f64> { el.attribute("points").map(|v| list(v).map_while(|t| t.parse().ok()).collect()).unwrap_or_default() };
        let ellipse = |cx: f64, cy: f64, rx: f64, ry: f64| {
            (rx > 0.0 && ry > 0.0).then(|| {
                format!(
                    "M{} {} A{rx} {ry} 0 0 1 {cx} {} A{rx} {ry} 0 0 1 {} {cy} A{rx} {ry} 0 0 1 {cx} {} A{rx} {ry} 0 0 1 {} {cy}Z",
                    cx + rx,
                    cy,
                    cy + ry,
                    cx - rx,
                    cy - ry,
                    cx + rx
                )
            })
        };
        match el.tag_name().name() {
            "circle" => {
                let r = len("r", ((vw * vw + vh * vh) / 2.0).sqrt())?;
                ellipse(x("cx"), y("cy"), r, r)
            }
            "ellipse" => {
                let (rx, ry) = (len("rx", vw), len("ry", vh));
                let (rx, ry) = (rx.or(ry)?, ry.or(rx)?);
                ellipse(x("cx"), y("cy"), rx, ry)
            }
            "rect" => {
                let (x0, y0, w, h) = (x("x"), y("y"), len("width", vw)?, len("height", vh)?);
                if w <= 0.0 || h <= 0.0 {
                    return None;
                }
                let (rx, ry) = (len("rx", vw).filter(|v| *v > 0.0), len("ry", vh).filter(|v| *v > 0.0));
                let (rx, ry) = (rx.or(ry).unwrap_or(0.0).min(w / 2.0), ry.or(rx).unwrap_or(0.0).min(h / 2.0));
                if rx <= 0.0 || ry <= 0.0 {
                    return Some(format!("M{x0} {y0} H{} V{} H{x0}Z", x0 + w, y0 + h));
                }
                let (x1, y1) = (x0 + w, y0 + h);
                Some(format!(
                    "M{} {y0} H{} A{rx} {ry} 0 0 1 {x1} {} V{} A{rx} {ry} 0 0 1 {} {y1} H{} A{rx} {ry} 0 0 1 {x0} {} V{} A{rx} {ry} 0 0 1 {} {y0}Z",
                    x0 + rx,
                    x1 - rx,
                    y0 + ry,
                    y1 - ry,
                    x1 - rx,
                    x0 + rx,
                    y1 - ry,
                    y0 + ry,
                    x0 + rx
                ))
            }
            "line" => Some(format!("M{} {} L{} {}", x("x1"), y("y1"), x("x2"), y("y2"))),
            tag @ ("polyline" | "polygon") => {
                let pts = points();
                let mut d = String::new();
                for (i, [x, y]) in pts.as_chunks::<2>().0.iter().enumerate() {
                    let _ = write!(d, "{}{x} {y} ", if i == 0 { 'M' } else { 'L' });
                }
                if d.is_empty() {
                    return None;
                }
                if tag == "polygon" {
                    d.push('Z');
                }
                Some(d)
            }
            _ => None,
        }
    }

    /// Read one Inkscape `<flowRoot>` (SVG 1.2 flowed text, which browsers don't draw but
    /// Inkscape writes) as area type: its frame the first `<rect>` of its `<flowRegion>`, a
    /// paragraph per `<flowPara>`, styled as `<text>` is. Other regions are left out with a warning.
    fn build_flow(&self, t: XNode, label: &str, warnings: &mut Vec<String>) -> Option<(PendingText, Vec<String>)> {
        let (vw, vh) = self.viewport;
        let region = t.children().find(|c| c.is_element() && c.tag_name().name() == "flowRegion");
        let rect = region.and_then(|r| r.children().find(|c| c.is_element() && c.tag_name().name() == "rect"));
        let size = |n: XNode, a: &str, pct: f64| n.attribute(a).and_then(|v| self.length(v, DEFAULT_FONT_SIZE, pct));
        let frame = rect.and_then(|r| {
            let (w, h) = (size(r, "width", vw)?, size(r, "height", vh)?);
            (w > 0.0 && h > 0.0).then(|| (r, Rect::new(0.0, 0.0, w, h)))
        });
        let Some((r, frame)) = frame else {
            warnings.push(format!("{label}: flowed text (flowRoot) without a rectangle region was left out"));
            return None;
        };
        let origin = Affine::translate((size(r, "x", vw).unwrap_or(0.0), size(r, "y", vh).unwrap_or(0.0)));
        let paras: Vec<XNode> = t.descendants().filter(|n| n.is_element() && n.tag_name().name() == "flowPara").collect();
        let collapse = |n: XNode| {
            n.descendants().filter(XNode::is_text).filter_map(|c| c.text()).collect::<String>().split_whitespace().collect::<Vec<_>>().join(" ")
        };
        let text = paras.iter().map(|p| collapse(*p)).collect::<Vec<_>>().join("\n");
        if text.trim().is_empty() {
            return None;
        }
        let style = self.elem(paras.first().copied().unwrap_or(t)).style.clone();
        let mut obj = TextObject::point(Point::ZERO, &text, style);
        obj.kind = TextKind::Area { frame: vectorcraft_geom::shapes::rectangle(frame) };
        obj.xf = parse_transform(r.attribute("transform")) * origin;
        let servers = vec![(None, None); obj.runs.len()];
        let b = obj.xf.transform_rect_bbox(frame);
        // A size, place or transform past what numbers hold (`width="1e999"`) has no frame to set type in.
        if ![b.x0, b.y0, b.x1, b.y1].iter().all(|v| v.is_finite()) {
            warnings.push(format!("{label}: flowed text (flowRoot) with a region too large to place was left out"));
            return None;
        }
        let marker = Rect::new(b.x0, b.y0, b.x1.max(b.x0 + 1.0), b.y1.max(b.y0 + 1.0));
        let name = t.attribute("id").unwrap_or("").to_string();
        Some((PendingText { name, obj, servers, paints: 0, non_scaling: false, under: None, marker }, Vec::new()))
    }

    /// Read one `<text>`: the pending text and its `url(#…)` paints.
    fn build(&self, xml: &roxmltree::Document, t: XNode, label: &str, warnings: &mut Vec<String>) -> Option<(PendingText, Vec<String>)> {
        let preserve = t.ancestors().filter(|a| a.is_element()).find_map(|a| a.attribute((roxmltree::NS_XML_URI, "space"))) == Some("preserve");
        let mut cs = Chars { chars: vec![], spans: vec![], path: None };
        self.collect(t, preserve, &mut cs);
        if !preserve {
            while cs.chars.last().is_some_and(|c| c.0 == ' ') {
                cs.chars.pop();
            }
        }
        let n = cs.chars.len();
        if n == 0 {
            return None;
        }

        // Positioning lists, applied in tree order so descendants override their ancestors.
        let (vw, vh) = self.viewport;
        let (mut x, mut y, mut dx, mut dy, mut rot) = (vec![None; n], vec![None; n], vec![0.0; n], vec![0.0; n], vec![None; n]);
        for (el, r) in &cs.spans {
            let r = r.start.min(n)..r.end.min(n);
            let em = self.font_size(*el);
            let apply = |name: &str, pct: f64, put: &mut dyn FnMut(usize, f64)| {
                if let Some(v) = el.attribute(name) {
                    for (i, l) in r.clone().zip(list(v).filter_map(|t| self.length(t, em, pct))) {
                        put(i, l);
                    }
                }
            };
            apply("x", vw, &mut |i, v| x[i] = Some(v));
            apply("y", vh, &mut |i, v| y[i] = Some(v));
            apply("dx", vw, &mut |i, v| dx[i] = v);
            apply("dy", vh, &mut |i, v| dy[i] = v);
            // A shorter rotate list repeats its last value.
            let angles: Vec<f64> =
                el.attribute("rotate").map(|v| list(v).filter_map(|t| t.parse().ok()).filter(|a: &f64| a.is_finite()).collect()).unwrap_or_default();
            if let Some(&last) = angles.last() {
                for (k, i) in r.enumerate() {
                    rot[i] = Some(*angles.get(k).unwrap_or(&last));
                }
            }
        }

        let mut paints: Vec<String> = vec![];
        let mut paint_index = |url: &Option<String>| {
            url.as_ref().map(|u| {
                paints.iter().position(|p| p == u).unwrap_or_else(|| {
                    paints.push(u.clone());
                    paints.len() - 1
                })
            })
        };
        let mut cells: Vec<Cell> = cs
            .chars
            .iter()
            .zip(&rot)
            .map(|((ch, el), r)| {
                let e = self.elem(*el);
                let mut style = e.style.clone();
                if let Some(r) = r {
                    style.rotation = -r;
                }
                Cell {
                    ch: *ch,
                    style,
                    fill: paint_index(&e.fill),
                    stroke: paint_index(&e.stroke),
                    kern: if *ch == ' ' { e.word_spacing } else { 0.0 },
                    stroke_first: e.stroke_first,
                }
            })
            .collect();
        let under = stroke_under(&mut cells, label, warnings);

        let anchor_of = |i: usize| match cs.chars.get(i).and_then(|c| self.css.prop(c.1, "text-anchor")).as_deref() {
            Some("middle") => 0.5,
            Some("end") => 1.0,
            _ => 0.0,
        };
        let anchor = anchor_of(0);
        let path = cs.path.and_then(|tp| match self.text_path(xml, tp) {
            Some(p) => Some((tp, p)),
            None => {
                warnings.push(format!("{label}: textPath has no usable path; imported as point type"));
                None
            }
        });
        let vertical = self.css.prop(t, "writing-mode").is_some_and(|m| m.starts_with("tb") || m.starts_with("vertical"));
        let mut obj = TextObject::point(Point::ZERO, "", CharStyle::default());
        let mut breaks: Vec<Break> = vec![];
        let mut para_styles: Vec<ParaStyle> = vec![];
        if let Some((tp, bp)) = path {
            // Along the path: dx is extra advance, dy shifts off the path.
            let mut off = 0.0;
            for i in 0..n {
                off -= dy[i];
                cells[i].style.baseline_shift += off;
                if i > 0 {
                    cells[i - 1].kern += dx[i];
                }
            }
            let len: f64 = bp.segments().map(|s| s.arclen(1e-4)).sum();
            let em = self.font_size(tp);
            let offset = tp.attribute("startOffset").and_then(|v| self.length(v, em, len)).unwrap_or(0.0) + dx[0];
            let width = if anchor > 0.0 { advance(&point_text(&cells)) } else { 0.0 };
            let start = Some((offset - anchor * width) / len).filter(|s| s.is_finite()).map_or(0.0, |s| s.clamp(0.0, 1.0));
            obj.kind = TextKind::OnPath { path: PathData::from_bezpath(&bp), start, end: None };
            obj.xf = Affine::IDENTITY;
        } else if vertical {
            // Type on a vertical path: dy is extra advance, dx shifts across the column.
            let mut off = 0.0;
            for i in 0..n {
                off += dx[i];
                cells[i].style.baseline_shift += off;
                if i > 0 {
                    cells[i - 1].kern += dy[i];
                }
            }
            let lay = measure(&point_text(&cells));
            let width: f64 = lay.glyphs.iter().map(|g| g.advance).sum();
            // Glyphs turn 90° clockwise; centre the em box on the column (SVG's central baseline).
            let centre = lay.lines.first().map_or(0.0, |l| (l.ascent - l.descent) / 2.0);
            let (x0, y0) = (finite(x[0].unwrap_or(0.0) - centre), finite(y[0].unwrap_or(0.0) + dy[0] - anchor * width));
            let mut bp = BezPath::new();
            bp.move_to((x0, y0));
            bp.line_to((x0, y0 + width + 1.0));
            obj.kind = TextKind::OnPath { path: PathData::from_bezpath(&bp), start: 0.0, end: None };
            obj.xf = Affine::IDENTITY;
        } else {
            let origin = Point::new(x[0].unwrap_or(0.0) + dx[0], y[0].unwrap_or(0.0) + dy[0]);
            let (justify, left, starts) = lines(&mut cells, &x, &y, &dx, &dy, anchor, origin.x, &mut breaks);
            obj.para.justify = justify;
            obj.xf = Affine::translate((finite(left), finite(origin.y)));
            // Lines anchored differently (paragraphs aligned differently): each line's paragraph
            // takes its alignment, and indents that put it where it was.
            if let Some(starts) = starts.filter(|st| st.iter().any(|(i, _)| anchor_of(*i) != anchor)) {
                for (k, &(i, ax)) in starts.iter().enumerate() {
                    let a = anchor_of(i);
                    let d = finite(ax - left);
                    let mut pa = ParaStyle { justify: justify_of(a), ..ParaStyle::default() };
                    match pa.justify {
                        Justify::Center if d >= 0.0 => pa.left_indent = 2.0 * d,
                        Justify::Center => pa.right_indent = -2.0 * d,
                        Justify::Right => pa.right_indent = -d,
                        _ => pa.left_indent = d,
                    }
                    let count = if k == 0 { 1 } else { breaks.get(k - 1).map_or(1, |b| b.count) };
                    para_styles.extend(std::iter::repeat_n(pa, count));
                }
            }
        }
        let (runs, servers) = assemble(&cells, &breaks).0;
        obj.runs = runs;
        if !para_styles.is_empty() {
            obj.set_paragraph_styles(para_styles);
        }
        // SVG text runs left to right unless `direction: rtl` says otherwise, whatever its first
        // strong character: pinned (on every paragraph) when that would read as right to left.
        if self.css.prop(t, "direction").is_some_and(|d| d.trim() == "rtl") {
            obj.edit_paras(None, |pa| pa.direction = Some(ParaDirection::RightToLeft));
        } else if obj.plain_text().split('\n').any(|p| vectorcraft_text::paragraph_is_rtl(p, None)) {
            obj.edit_paras(None, |pa| pa.direction = Some(ParaDirection::LeftToRight));
        }
        // The placeholder covers the text (laid out when a paint server needs its bounding box).
        let b = if paints.is_empty() { obj.bounds() } else { Some(obj.xf.transform_rect_bbox(measure(&obj).bounds)) }.unwrap_or_default();
        let marker = Rect::new(b.x0, b.y0, b.x1.max(b.x0 + 1.0), b.y1.max(b.y0 + 1.0));
        let name = t.attribute("id").unwrap_or("").to_string();
        let non_scaling = vector_effect::is_non_scaling(&self.css, t);
        Some((PendingText { name, obj, servers, paints: paints.len(), non_scaling, under, marker }, paints))
    }
}

/// `paint-order` with the stroke first: the characters' stroke moves under all of them, as the
/// object's own stroke (returned with its `url(#…)` paint index), and their own strokes go. Only
/// when every character has that same stroke under a fill; otherwise a warning.
fn stroke_under(cells: &mut [Cell], label: &str, warnings: &mut Vec<String>) -> Option<(StrokeLayer, Option<usize>)> {
    let stroked = |c: &Cell| (c.stroke.is_some() || !c.style.stroke.is_none()) && c.style.stroke_width > 0.0;
    let filled = |c: &Cell| c.fill.is_some() || !c.style.fill.is_none();
    // Spaces paint nothing.
    let mut ink = cells.iter().filter(|c| !c.ch.is_whitespace());
    if !ink.clone().any(|c| c.stroke_first && stroked(c) && filled(c)) {
        return None;
    }
    let key = |c: &Cell| (c.stroke_first && stroked(c)).then(|| (c.style.stroke_layer(), c.stroke));
    let first = ink.next().and_then(key);
    if first.is_none() || !ink.all(|c| key(c) == first) {
        warnings.push(format!("paint-order on {label} ignored: its characters' strokes differ"));
        return None;
    }
    let none = CharStyle::default().stroke_layer();
    for c in cells.iter_mut() {
        c.style.set_stroke_layer(&none);
        c.stroke = None;
    }
    first
}

/// The type sizes the Character panel takes, in points.
const SIZES: RangeInclusive<f64> = 0.1..=1296.0;

/// How much of `xf` (text space → document) [`fold_scale`] makes the type's size: the scale
/// across the baseline for point type (a stretch along it, a skew, a rotation and a reflection
/// stay in the transform), the mean scale for type on a path, whose baseline turns. Not past the
/// Character panel's sizes (the rest stays in the transform); 1 when `xf` collapses the text.
pub(super) fn size_scale(t: &TextObject, xf: Affine) -> f64 {
    let [a, b, ..] = xf.as_coeffs();
    let det = xf.determinant().abs();
    let k = match t.kind {
        TextKind::OnPath { .. } => det.sqrt(),
        _ => det / a.hypot(b),
    };
    // To 6 significant digits: usvg's transforms are single precision, and `rotate(30) scale(2)`
    // makes 7 pt type 14 pt, not 13.9999998 pt (the type draws the same either way).
    let p = 10f64.powi((5.0 - k.log10().floor()) as i32);
    let k = (k * p).round() / p;
    if !(k.is_finite() && k > 0.0) {
        return 1.0;
    }
    let (lo, hi) = t.runs.iter().map(|r| r.style.size).fold((f64::INFINITY, 0.0f64), |(lo, hi), s| (lo.min(s), hi.max(s)));
    // Sizes already outside the range don't move further out.
    if k > 1.0 { k.min((SIZES.end() / hi).max(1.0)) } else { k.max((SIZES.start() / lo).min(1.0)) }
}

/// Make text space `k` times larger and the transform `1 / k` times smaller, so the type draws
/// the same at `k` times its size (and leading, baseline shift, character strokes and path): the
/// size the Character panel shows is the size it draws at.
pub(super) fn fold_scale(t: &mut TextObject, k: f64) {
    for r in &mut t.runs {
        let st = &mut r.style;
        st.size *= k;
        st.leading = st.leading.map(|l| l * k);
        st.baseline_shift *= k;
    }
    t.scale_char_strokes(k);
    let up = Affine::scale(k);
    match &mut t.kind {
        TextKind::OnPath { path, .. } | TextKind::Area { frame: path } => path.transform(up),
        TextKind::Point => {}
    }
    t.cached_bounds = t.cached_bounds.map(|b| up.transform_rect_bbox(b));
    t.xf *= Affine::scale(1.0 / k);
}

/// `v`, or 0 when far-off positions added up past f64's range.
fn finite(v: f64) -> f64 {
    if v.is_finite() { v } else { 0.0 }
}

/// The alignment of an SVG text chunk anchored at `anchor` (0 start, 0.5 middle, 1 end).
fn justify_of(anchor: f64) -> Justify {
    match anchor {
        a if a >= 1.0 => Justify::Right,
        a if a > 0.0 => Justify::Center,
        _ => Justify::Left,
    }
}

/// Point type: line breaks, baseline shifts and kerning from the positioning lists. Returns the
/// justification, the x of the first line's left (or anchor) edge and, when no character is
/// placed on its own, each line's first character and anchor x.
#[allow(clippy::too_many_arguments)]
fn lines(
    cells: &mut [Cell],
    x: &[Option<f64>],
    y: &[Option<f64>],
    dx: &[f64],
    dy: &[f64],
    anchor: f64,
    x0: f64,
    breaks: &mut Vec<Break>,
) -> (Justify, f64, Option<Vec<(usize, f64)>>) {
    let n = cells.len();
    let y0 = y[0].unwrap_or(0.0) + dy[0];
    let (mut base, mut cur) = (y0, y0);
    // Line starts (first character, anchor x) and absolutely placed characters (index, x).
    let mut lines = vec![(0, x0)];
    let mut targets: Vec<(usize, f64)> = vec![];
    for i in 1..n {
        let ny = y[i].unwrap_or(cur) + dy[i];
        let lead = cells[i].style.effective_leading();
        let gap = ny - base;
        if let Some(ax) = x[i].filter(|_| gap.is_finite() && gap > lead * 0.5) {
            // Whole multiples of the auto leading add blank lines (not without limit: a far-off y
            // mustn't ask for billions); other spacing is one break.
            let count = (gap / lead).round().clamp(1.0, MAX_LINE_GAP);
            let count = if (gap / count - lead).abs() <= lead * 0.05 { count } else { 1.0 };
            breaks.push(Break { at: i, count: count as usize, leading: gap / count });
            lines.push((i, ax + dx[i]));
            (base, cur) = (ny, ny);
            continue;
        }
        cur = ny;
        match x[i] {
            Some(ax) => targets.push((i, ax + dx[i])),
            None => cells[i - 1].kern += dx[i],
        }
        cells[i].style.baseline_shift += base - cur;
    }
    // A line's characters take its spacing as leading unless it is their auto leading.
    for (k, b) in breaks.iter().enumerate() {
        let end = breaks.get(k + 1).map_or(n, |nb| nb.at);
        if cells[b.at..end].iter().any(|c| (c.style.effective_leading() - b.leading).abs() > 0.01) {
            for c in &mut cells[b.at..end] {
                c.style.leading = Some(b.leading);
            }
        }
    }
    if targets.is_empty() {
        return (justify_of(anchor), x0, Some(lines));
    }
    // Absolutely placed characters: lay the text out left-aligned and kern each one onto its x.
    // SVG anchors every absolutely placed chunk on its own, so with a middle/end anchor each
    // chunk's target moves back by its share of the chunk's width.
    let line_of = |i: usize| lines.partition_point(|(s, _)| *s <= i) - 1;
    let mut left = x0;
    let mut wants: Vec<(usize, f64)> = targets.clone();
    if anchor > 0.0 {
        let (runs, bytes) = assemble(cells, breaks);
        let lay = measure(&with_runs(runs.0));
        let (pens, ends) = pens(&lay, &bytes);
        // Chunk starts in order: line starts and absolutely placed characters.
        let mut starts: Vec<usize> = lines.iter().map(|l| l.0).chain(targets.iter().map(|t| t.0)).collect();
        starts.sort_unstable();
        let width = |s: usize| {
            let k = starts.partition_point(|&c| c <= s);
            let e = starts.get(k).copied().unwrap_or(n);
            let end = (s..e).filter_map(|j| ends[j]).fold(f64::NEG_INFINITY, f64::max);
            match pens[s] {
                Some(p) if end.is_finite() => end - p,
                _ => 0.0,
            }
        };
        let line_left: Vec<f64> = lines.iter().map(|&(s, lx)| lx - anchor * width(s)).collect();
        left = line_left[0];
        for w in &mut wants {
            w.1 -= anchor * width(w.0) + line_left[line_of(w.0)];
        }
    } else {
        for w in &mut wants {
            w.1 -= lines[line_of(w.0)].1;
        }
    }
    // Kerning changes shaping (pair kerning is off for manually kerned characters): repeat until
    // every character sits on its x.
    for _ in 0..4 {
        let (runs, bytes) = assemble(cells, breaks);
        let lay = measure(&with_runs(runs.0));
        let (pens, _) = pens(&lay, &bytes);
        let (mut line, mut moved, mut worst) = (usize::MAX, 0.0, 0.0f64);
        for &(i, want) in &wants {
            let l = line_of(i);
            if l != line {
                (line, moved) = (l, 0.0);
            }
            let (Some(p), Some(p0)) = (pens[i], pens[lines[l].0]) else { continue };
            let d = want - (p - p0 + moved);
            cells[i - 1].kern += d;
            moved += d;
            worst = worst.max(d.abs());
        }
        if worst < 0.01 {
            break;
        }
    }
    (Justify::Left, left, None)
}

/// A left-aligned point text of `runs` at the origin (for measuring).
fn with_runs(runs: Vec<TextRun>) -> TextObject {
    let mut t = TextObject::point(Point::ZERO, "", CharStyle::default());
    t.runs = runs;
    t
}

fn point_text(cells: &[Cell]) -> TextObject {
    with_runs(assemble(cells, &[]).0.0)
}

fn measure(t: &TextObject) -> TextLayout {
    vectorcraft_text::layout(FontDb::global(), t)
}

/// Total advance of a laid-out single line.
fn advance(t: &TextObject) -> f64 {
    measure(t).glyphs.iter().map(|g| g.advance).sum()
}

/// Pen x (start) and end x of the glyph that starts at each character's byte offset.
fn pens(lay: &TextLayout, bytes: &[usize]) -> (Vec<Option<f64>>, Vec<Option<f64>>) {
    let mut at: HashMap<usize, (f64, f64)> = HashMap::with_capacity(lay.glyphs.len());
    for g in &lay.glyphs {
        at.entry(g.byte).or_insert((g.origin.x, g.origin.x + g.advance));
    }
    bytes.iter().map(|b| at.get(b).map_or((None, None), |&(s, e)| (Some(s), Some(e)))).unzip()
}

type Key = (Option<usize>, Option<usize>);
type Runs = (Vec<TextRun>, Vec<Key>);

/// Runs (and their paint indices) from the characters, with `breaks` inserted; also the byte offset
/// of each character in the text.
fn assemble(cells: &[Cell], breaks: &[Break]) -> (Runs, Vec<usize>) {
    fn push((runs, keys): &mut Runs, ch: char, style: CharStyle, key: Key) {
        if let Some(r) = runs.last_mut()
            && keys.last() == Some(&key)
            && r.style == style
        {
            r.text.push(ch);
            return;
        }
        runs.push(TextRun { text: ch.into(), style, inline: None });
        keys.push(key);
    }
    let mut out: Runs = (vec![], vec![]);
    let mut bytes = Vec::with_capacity(cells.len());
    let mut off = 0;
    let mut next = breaks.iter().peekable();
    for (i, c) in cells.iter().enumerate() {
        if let Some(b) = next.next_if(|b| b.at == i)
            && let Some(prev) = i.checked_sub(1).map(|p| &cells[p])
        {
            // The first newline ends the previous line; the others are blank lines `leading` apart.
            let key = (prev.fill, prev.stroke);
            push(&mut out, '\n', prev.final_style(), key);
            let mut blank = prev.final_style();
            if (blank.effective_leading() - b.leading).abs() > 0.01 {
                blank.leading = Some(b.leading);
            }
            for _ in 1..b.count {
                push(&mut out, '\n', blank.clone(), key);
            }
            off += b.count;
        }
        bytes.push(off);
        off += c.ch.len_utf8();
        push(&mut out, c.ch, c.final_style(), (c.fill, c.stroke));
    }
    (out, bytes)
}
