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
//! usvg accepts only shapes, `<text>` and `<use>` as the children of a `<clipPath>`, so a `<text>`
//! there (and a `<use>` there that shows one) becomes a placeholder path instead, which the importer
//! finds by its id (`{prefix}{i}-…`). The importer clips by the text's glyph outlines, or uses the
//! text itself as live type. A `<text>` in a clip path that has an id also gets the usual
//! placeholder group after its path, and a `<use>` there with an id keeps a copy in a `<defs>`
//! after its path, for `<use>` elements outside the clip path.
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
use vectorcraft_text::{FontDb, PositionedGlyph, TextLayout, spacing_scale};

use super::css::{Styles, XNode};
use super::{DEFAULT_FONT_SIZE, PT_PER_IN, href, vector_effect};
use crate::xml_escape;

const SVG_NS: &str = "http://www.w3.org/2000/svg";

/// Most line breaks one absolutely positioned character adds before itself.
const MAX_LINE_GAP: f64 = 10_000.0;

/// Most blank lines (breaks past the first newline of each gap) the absolutely positioned
/// characters of one text add in all: past it, a gap is one break as wide as the gap.
const MAX_LINE_BREAKS: usize = 10_000;

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
    /// What its characters' Character Direction ([`Ctx::elem`]) leaves out of the file's
    /// `unicode-bidi`, for the import's warnings.
    bidi_notes: Vec<&'static str>,
}

/// An override (`unicode-bidi="bidi-override"`) read as an embedding.
const BIDI_OVERRIDE: &str = "unicode-bidi=\"bidi-override\" imported as an embedding; directional override is not supported";
/// An isolate (`unicode-bidi="isolate"`) read as an embedding.
const BIDI_ISOLATE: &str = "unicode-bidi=\"isolate\" imported as an embedding; isolation is not supported";
/// `unicode-bidi="isolate-override"` read as an embedding.
const BIDI_ISOLATE_OVERRIDE: &str = "unicode-bidi=\"isolate-override\" imported as an embedding; isolation and override are not supported";
/// `unicode-bidi="plaintext"` read as an embedding in the direction of its first strong character.
const BIDI_PLAINTEXT: &str =
    "unicode-bidi=\"plaintext\" imported as an embedding in the direction of its first strong character; isolation is not supported";
/// An embedding inside another, kept as an embedding of its own.
const BIDI_NESTED: &str = "a unicode-bidi embedding inside another imported one level deep; its characters may be ordered differently";
/// An override on the `<text>` itself, which keeps only its paragraph direction.
const BIDI_TEXT_OVERRIDE: &str = "unicode-bidi override on the text imported as its paragraph direction; directional override is not supported";

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
    /// The transform between a clip placeholder path's user space and its text's, by placeholder
    /// id, where the placeholder leaves it out (see [`clip_uses`]).
    inner: HashMap<String, Affine>,
    /// The glyph outlines of each text laid out so far, in text space, by index.
    outlines: RefCell<HashMap<usize, Rc<BezPath>>>,
}

impl TextSlots {
    /// The text a placeholder's main path stands for: by its id (`{prefix}{index}`, or
    /// `{prefix}{index}-{k}` for the clip placeholders), or inside `<use>` instances (where usvg
    /// drops ids) by its colour (the index) and its rectangle.
    pub fn find(&self, p: &usvg::Path) -> Option<usize> {
        if !p.id().is_empty() {
            let rest = p.id().strip_prefix(&self.prefix)?;
            return rest.split_once('-').map_or(rest, |(i, _)| i).parse().ok();
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

    /// The transform from the user space of clip placeholder path `p` to the space of the text it
    /// stands for (the identity unless [`clip_uses`] left the text's transform out of it).
    pub fn inner(&self, p: &usvg::Path) -> Affine {
        self.inner.get(p.id()).copied().unwrap_or(Affine::IDENTITY)
    }

    /// The glyph outlines of text `i` in text space, laid out once however many clips use it.
    pub fn outlines(&self, i: usize) -> Option<Rc<BezPath>> {
        if let Some(bp) = self.outlines.borrow().get(&i) {
            return Some(bp.clone());
        }
        let t = self.texts.get(i)?;
        let bp = Rc::new(vectorcraft_text::layout(FontDb::global(), &t.obj).to_bezpath());
        self.outlines.borrow_mut().insert(i, bp.clone());
        Some(bp)
    }
}

/// Replace every `<text>` usvg would draw by a placeholder group (see the module docs).
/// `original_xml` supplies text styles; `xml` supplies placeholder positions and tracking IDs.
pub(super) fn prepare<'s>(
    svg: &'s str,
    xml: &roxmltree::Document,
    original_xml: Option<&roxmltree::Document>,
    dpi: f64,
    warnings: &mut Vec<String>,
) -> (Cow<'s, str>, TextSlots) {
    let mut prefix = String::from("vectorcraft-text-");
    while xml.descendants().any(|n| n.attribute("id").is_some_and(|id| id.starts_with(&prefix))) {
        prefix.insert(0, '_');
    }
    let mut slots = TextSlots { prefix, ..TextSlots::default() };
    let style_xml = original_xml.unwrap_or(xml);
    let cx = Ctx::new(style_xml, dpi);
    // The replacements, in source order: each element's range and what takes its place.
    let mut edits: Vec<(Range<usize>, String)> = vec![];
    let mut copied = 0;
    let is_text = |n: &XNode| {
        n.is_element()
            && matches!(n.tag_name().name(), "text" | "flowRoot")
            && n.tag_name().namespace() == Some(SVG_NS)
            && !n.ancestors().any(|a| a.has_tag_name("style"))
    };
    // The slot of each text, by element, for the `<use>` elements below.
    let mut slot_of = HashMap::new();
    let mut inst = Instances::default();
    for (t, style_t) in xml.descendants().filter(is_text).zip(style_xml.descendants().filter(is_text)) {
        let label = t.attribute("id").map_or_else(|| "a text".to_string(), |id| format!("text '{id}'"));
        // CSS Masking Module Level 1 §6.1 (https://www.w3.org/TR/css-masking-1/#ClipPathElement):
        // text is clip path content only as a child of the `<clipPath>`.
        let clip_child = in_clip_path(t);
        if !clip_child && t.ancestors().skip(1).any(|a| a.is_element() && a.tag_name().name() == "clipPath") {
            warnings.push(format!("{label} in a clip path ignored"));
            continue;
        }
        let r = t.range();
        let Some(q) = qualifier(svg, t) else { continue };
        let flowed = t.tag_name().name() == "flowRoot";
        if cx.css.own(style_t, "display").as_deref() == Some("none") || r.start < copied {
            continue;
        }
        let built = if flowed { cx.build_flow(style_t, &label, warnings) } else { cx.build(style_xml, style_t, &label, warnings) };
        let Some((text, paints)) = built else { continue };
        let mut out = String::new();
        let index = slots.texts.len();
        if clip_child {
            let id = format!("{}{index}-c", slots.prefix);
            let props = clip_props(&cx.css, Some(style_t), None);
            clip_placeholder(&mut out, q, &id, &own_transform(&cx.css, style_t), &props, t, text.marker);
            // A `<use>` elsewhere can still show the text by its id: the clip path leaves out this
            // group (a group isn't clip path content), but the `<use>` finds it.
            if t.attribute("id").is_some() {
                placeholder(&mut out, q, &cx.css, t, style_t, &slots.prefix, index, text.marker, &paints, true);
            }
        } else {
            placeholder(&mut out, q, &cx.css, t, style_t, &slots.prefix, index, text.marker, &paints, true);
        }
        copied = r.end;
        edits.push((r, out));
        slot_of.insert(t.id(), index);
        slots.texts.push(text);
        inst.paints.push(paints);
    }
    if slots.texts.is_empty() {
        return (Cow::Borrowed(svg), slots);
    }
    clip_uses(svg, xml, style_xml, &cx, &slot_of, &mut slots, &mut inst, &mut edits, warnings);
    text_uses(svg, xml, style_xml, &cx, &slot_of, &mut slots, &mut inst, &mut edits, warnings);
    edits.sort_by_key(|(r, _)| r.start);
    let mut out = String::with_capacity(svg.len());
    let mut copied = 0;
    for (r, s) in edits {
        if r.start < copied {
            continue;
        }
        out.push_str(svg.get(copied..r.start).unwrap_or(""));
        out.push_str(&s);
        copied = r.end;
    }
    out.push_str(svg.get(copied..).unwrap_or(""));
    (Cow::Owned(out), slots)
}

/// Is `n` a child of a `<clipPath>`?
fn in_clip_path(n: XNode) -> bool {
    n.parent_element().is_some_and(|p| p.tag_name().name() == "clipPath" && p.tag_name().namespace() == Some(SVG_NS))
}

/// The namespace prefix of element `n` as the source writes it (empty, or `svg:` and the like), so
/// what replaces it lands in the same namespace. `None` when its source range isn't the element
/// itself (entity expansions).
fn qualifier<'s>(svg: &'s str, n: XNode) -> Option<&'s str> {
    let r = n.range();
    let qname = svg
        .get(r.start..r.end)
        .and_then(|s| s.strip_prefix('<'))
        .map(|s| s.split(|c: char| c.is_whitespace() || c == '>' || c == '/').next().unwrap_or(""))?;
    qname.strip_suffix(n.tag_name().name()).filter(|q| q.is_empty() || q.ends_with(':'))
}

/// Replace each `<use>` in a clip path that shows a text by the clip placeholder of that text,
/// placed by the `<use>` (usvg drops ids and paints clip path content black, so the text a
/// placeholder group stands for couldn't be found again inside one).
#[allow(clippy::too_many_arguments)]
fn clip_uses(
    svg: &str,
    xml: &roxmltree::Document,
    style_xml: &roxmltree::Document,
    cx: &Ctx,
    slot_of: &HashMap<roxmltree::NodeId, usize>,
    slots: &mut TextSlots,
    inst: &mut Instances,
    edits: &mut Vec<(Range<usize>, String)>,
    warnings: &mut Vec<String>,
) {
    let is_use = |n: &XNode| n.is_element() && n.tag_name().name() == "use" && n.tag_name().namespace() == Some(SVG_NS) && in_clip_path(*n);
    if !xml.descendants().any(|n| is_use(&n)) {
        return;
    }
    let (ids, style_ids) = (first_ids(xml), first_ids(style_xml));
    for (k, (u, style_u)) in xml.descendants().filter(is_use).zip(style_xml.descendants().filter(is_use)).enumerate() {
        let Some(id) = href(u).and_then(|h| h.strip_prefix('#')) else { continue };
        let Some(t) = ids.get(id) else { continue };
        let Some(&shared) = slot_of.get(&t.id()) else { continue };
        let Some(q) = qualifier(svg, u) else { continue };
        // An undisplayed `<use>` adds nothing to the clip (usvg leaves it out as it is).
        if cx.css.own(style_u, "display").is_some_and(|d| d.trim() == "none") {
            continue;
        }
        let style_t = style_ids.get(id).copied().filter(|s| s.tag_name() == t.tag_name());
        let i = style_t.map_or(shared, |s| use_slot(cx, style_xml, slots, inst, shared, s, style_u, warnings));
        let Some(marker) = slots.texts.get(i).map(|text| text.marker) else { continue };
        let placed = use_placement(cx, u, style_u);
        let text_xf = match style_t {
            Some(s) => own_transform(&cx.css, s),
            None => t.attribute("transform").unwrap_or("").to_string(),
        };
        // One element can carry one clip path. The text's own clip path is in the text's user space
        // and wins; a clip path of the `<use>` is in the `<use>`'s, so the text's transform moves
        // to the importer instead.
        let clipped = |n: XNode| cx.css.own(n, "clip-path").is_some_and(|v| v.trim() != "none");
        let id = format!("{}{i}-{k}", slots.prefix);
        let transform = if style_t.is_some_and(clipped) {
            if clipped(style_u) {
                let label = u.attribute("id").map_or_else(|| "a use of a text".to_string(), |id| format!("'{id}'"));
                warnings.push(format!("clip path of {label} in a clip path ignored: the text's own applies"));
            }
            format!("{placed} {text_xf}")
        } else {
            slots.inner.insert(id.clone(), parse_transform(Some(&text_xf)));
            placed
        };
        let props = clip_props(&cx.css, style_t, Some(style_u));
        let mut out = String::new();
        clip_placeholder(&mut out, q, &id, &transform, &props, u, marker);
        // Another `<use>` can still show this one by its id: a copy of it stays in a `<defs>`, which
        // the clip path leaves out, with the properties style rules gave it written out.
        if u.attribute("id").is_some() {
            use_copy(&mut out, q, &cx.css, u, style_u);
        }
        edits.push((u.range(), out));
    }
}

/// Replace each `<use>` outside clip paths that shows a text it passes other properties on to by a
/// group holding that text's placeholder as the `<use>` shows it ([`use_slot`]). Every other
/// `<use>` of a text shows the text's own placeholder.
#[allow(clippy::too_many_arguments)]
fn text_uses(
    svg: &str,
    xml: &roxmltree::Document,
    style_xml: &roxmltree::Document,
    cx: &Ctx,
    slot_of: &HashMap<roxmltree::NodeId, usize>,
    slots: &mut TextSlots,
    inst: &mut Instances,
    edits: &mut Vec<(Range<usize>, String)>,
    warnings: &mut Vec<String>,
) {
    let is_use = |n: &XNode| {
        n.is_element()
            && n.tag_name().name() == "use"
            && n.tag_name().namespace() == Some(SVG_NS)
            && !n.ancestors().any(|a| a.is_element() && a.tag_name().name() == "clipPath")
    };
    if !xml.descendants().any(|n| is_use(&n)) {
        return;
    }
    let (ids, style_ids) = (first_ids(xml), first_ids(style_xml));
    for (u, style_u) in xml.descendants().filter(is_use).zip(style_xml.descendants().filter(is_use)) {
        let Some(id) = href(u).and_then(|h| h.strip_prefix('#')) else { continue };
        let Some(t) = ids.get(id) else { continue };
        let Some(&shared) = slot_of.get(&t.id()) else { continue };
        let Some(style_t) = style_ids.get(id).copied().filter(|s| s.tag_name() == t.tag_name()) else { continue };
        let Some(q) = qualifier(svg, u) else { continue };
        let i = use_slot(cx, style_xml, slots, inst, shared, style_t, style_u, warnings);
        let (Some(text), Some(paints)) = (slots.texts.get(i), inst.paints.get(i)) else { continue };
        if i == shared {
            continue;
        }
        let mut out = String::new();
        use_group(&mut out, q, cx, u, style_u);
        placeholder(&mut out, q, &cx.css, *t, style_t, &slots.prefix, i, text.marker, paints, false);
        let _ = write!(out, "</{q}g>");
        edits.push((u.range(), out));
    }
}

/// A text's slot, the parent of a `<use>` showing it and the properties that `<use>` sets itself.
type UseKey = (usize, Option<roxmltree::NodeId>, Vec<(String, String)>);

/// The slots made for texts as `<use>` elements show them ([`use_slot`]).
#[derive(Default)]
struct Instances {
    /// The `url(#…)` paints of each slot, by index.
    paints: Vec<Vec<String>>,
    /// The slot made for each [`UseKey`]; `None` where the text's own slot serves.
    memo: HashMap<UseKey, Option<usize>>,
    /// The nodes, text bytes and text path elements of the texts built so far, up to
    /// [`MAX_INSTANCE_WORK`].
    work: usize,
}

/// The most nodes, text bytes and elements of the paths `<textPath>`s follow that [`use_slot`]
/// builds texts from, all `<use>` elements together (the file is untrusted). Past it, a `<use>`
/// shows its text with the text's own properties.
const MAX_INSTANCE_WORK: usize = 1 << 20;

/// The slot of text `style_t` (slot `shared`) as `<use>` `style_u` shows it. The text inherits
/// from the `<use>` there ([`Ctx::instance`]): when that changes it, it gets a new slot; otherwise
/// the result is `shared`. `<use>` elements with one parent that set the same properties share one.
#[allow(clippy::too_many_arguments)]
fn use_slot(
    cx: &Ctx,
    style_xml: &roxmltree::Document,
    slots: &mut TextSlots,
    inst: &mut Instances,
    shared: usize,
    style_t: XNode,
    style_u: XNode,
    warnings: &mut Vec<String>,
) -> usize {
    // A `<use>` inside the text it shows would make the text inherit from itself.
    if style_u.ancestors().any(|a| a == style_t) {
        return shared;
    }
    let placing = |name: &str| matches!(name, "id" | "x" | "y" | "width" | "height" | "href" | "transform" | "style");
    let mut own: Vec<(String, String)> = style_u
        .attributes()
        .filter(|a| a.namespace().is_none() && !placing(a.name()))
        .map(|a| (a.name().to_string(), a.value().to_string()))
        .collect();
    own.extend(cx.css.declared(style_u).iter().filter(|(k, _)| k.as_str() != "transform").map(|(k, v)| (format!("style {k}"), v.clone())));
    own.sort();
    let key = (shared, style_u.parent_element().map(|p| p.id()), own);
    if let Some(slot) = inst.memo.get(&key) {
        return slot.unwrap_or(shared);
    }
    let label = style_t.attribute("id").map_or_else(|| "a text".to_string(), |id| format!("text '{id}'"));
    let work = if inst.work < MAX_INSTANCE_WORK {
        // Each build lays the text out along the path its `<textPath>` follows, so the path counts too.
        style_t
            .descendants()
            .map(|n| if n.is_text() { n.text().map_or(0, str::len) } else { cx.path_elements(n).saturating_add(1) })
            .fold(0, usize::saturating_add)
    } else {
        0
    };
    inst.work = inst.work.saturating_add(work);
    if work == 0 || inst.work > MAX_INSTANCE_WORK {
        inst.work = MAX_INSTANCE_WORK;
        let w = format!(
            "more than {} KiB of text and text paths shown through <use> elements: the rest show their text without the properties the <use> passes on",
            MAX_INSTANCE_WORK >> 10
        );
        if !warnings.contains(&w) {
            warnings.push(w);
        }
        return shared;
    }
    let shown = cx.instance(style_t, style_u);
    let mut found = vec![];
    let built = if style_t.tag_name().name() == "flowRoot" {
        shown.build_flow(style_t, &label, &mut found)
    } else {
        shown.build(style_xml, style_t, &label, &mut found)
    };
    let differs = |(text, paints): &(PendingText, Vec<String>)| {
        slots.texts.get(shared).is_none_or(|old| old.obj != text.obj || old.servers != text.servers || old.under != text.under)
            || inst.paints.get(shared) != Some(paints)
    };
    let slot = built.filter(differs).map(|(text, paints)| {
        for w in found {
            if !warnings.contains(&w) {
                warnings.push(w);
            }
        }
        slots.texts.push(text);
        inst.paints.push(paints);
        slots.texts.len() - 1
    });
    inst.memo.insert(key, slot);
    slot.unwrap_or(shared)
}

/// The transform `<use>` `u` places what it shows with: its own, then its `x`/`y` offset (in its
/// font size). `style_u` supplies its styles.
fn use_placement(cx: &Ctx, u: XNode, style_u: XNode) -> String {
    let em = cx.font_size(style_u);
    let length = |a: &str, pct: f64| u.attribute(a).and_then(|v| cx.length(v, em, pct)).unwrap_or(0.0);
    let (x, y) = (length("x", cx.viewport.0), length("y", cx.viewport.1));
    format!("{} translate({x} {y})", own_transform(&cx.css, style_u))
}

/// Write the start tag of the group that takes `<use>` `u`'s place: its attributes in no namespace
/// other than its link, place and size, and as its `style` the properties rules and its `style` set
/// on it (from `style_u`), then its placement ([`use_placement`]) and its non-inherited properties
/// and `visibility`, so no rule that matches the group changes it.
fn use_group(out: &mut String, q: &str, cx: &Ctx, u: XNode, style_u: XNode) {
    let _ = write!(out, "<{q}g");
    let placing = |name: &str| matches!(name, "style" | "href" | "x" | "y" | "width" | "height" | "transform");
    for a in u.attributes().filter(|a| a.namespace().is_none() && !placing(a.name())) {
        let _ = write!(out, " {}=\"{}\"", a.name(), xml_escape(a.value()));
    }
    let declared = cx.css.declared(style_u);
    let mut props: Vec<_> = declared.iter().filter(|(k, _)| k.as_str() != "transform").collect();
    props.sort();
    let mut style: String = props.into_iter().map(|(k, v)| format!("{k}:{v};")).collect();
    let _ = write!(style, "transform:{};", use_placement(cx, u, style_u));
    for (p, initial) in [
        ("opacity", "1"),
        ("clip-path", "none"),
        ("mask", "none"),
        ("filter", "none"),
        ("mix-blend-mode", "normal"),
        ("isolation", "auto"),
        ("display", "inline"),
    ] {
        let _ = write!(style, "{p}:{};", cx.css.own(style_u, p).as_deref().unwrap_or(initial));
    }
    let _ = write!(style, "visibility:{}", cx.css.prop(style_u, "visibility").as_deref().unwrap_or("visible"));
    let _ = write!(out, " style=\"{}\">", xml_escape(&style));
}

/// Write a copy of `<use>` `u` in a `<defs>`: its attributes in no namespace, its link as `href`,
/// and the properties set on it by rules and its `style` (from `style_u`) as its `style`.
fn use_copy(out: &mut String, q: &str, css: &Styles, u: XNode, style_u: XNode) {
    let _ = write!(out, "<{q}defs><{q}use");
    for a in u.attributes().filter(|a| a.namespace().is_none() && !matches!(a.name(), "style" | "href")) {
        let _ = write!(out, " {}=\"{}\"", a.name(), xml_escape(a.value()));
    }
    if let Some(h) = href(u) {
        let _ = write!(out, " href=\"{}\"", xml_escape(h));
    }
    let declared = css.declared(style_u);
    let mut props: Vec<_> = declared.iter().collect();
    props.sort();
    let style: String = props.into_iter().map(|(k, v)| format!("{k}:{v};")).collect();
    let _ = write!(out, " style=\"{}\"/></{q}defs>", xml_escape(&style));
}

/// The identity transform, as a `transform` value usvg reads.
const IDENTITY: &str = "matrix(1 0 0 1 0 0)";

/// The transform of element `n`: a `transform` declaration in a rule or its `style` (which the
/// prepass has written in SVG syntax), else its `transform` attribute; empty when it has none.
fn own_transform(css: &Styles, n: XNode) -> String {
    css.own(n, "transform").filter(|v| !v.eq_ignore_ascii_case("none")).unwrap_or_default()
}

/// Each id of `doc` with the first element that has it (a reference goes to that one, as in usvg).
fn first_ids<'a, 'i>(doc: &'a roxmltree::Document<'i>) -> HashMap<&'a str, XNode<'a, 'i>> {
    let mut ids = HashMap::new();
    for n in doc.descendants() {
        if let Some(id) = n.attribute("id") {
            ids.entry(id).or_insert(n);
        }
    }
    ids
}

/// The properties of clip path content that the placeholder of text `t` keeps, as a `style` value:
/// its `clip-path` (or that of the `<use>` `u` showing it, when the text has none), and its
/// computed `clip-rule` and `visibility`, inherited from `u` when there is one. Every value is
/// written out, so no style rule that matches the placeholder path applies to it.
fn clip_props(css: &Styles, t: Option<XNode>, u: Option<XNode>) -> String {
    let set = |n: XNode, p: &str| css.own(n, p).filter(|v| v.trim() != "inherit");
    let clip = |n: XNode| css.own(n, "clip-path").filter(|v| v.trim() != "none");
    let mut style = format!("clip-path:{};", t.and_then(clip).or_else(|| u.and_then(clip)).unwrap_or_else(|| "none".into()));
    for (p, initial) in [("clip-rule", "nonzero"), ("visibility", "visible")] {
        let v = t.and_then(|t| set(t, p)).or_else(|| u.or(t).and_then(|n| css.prop(n, p))).unwrap_or_else(|| initial.into());
        let _ = write!(style, "{p}:{v};");
    }
    style
}

/// Write the placeholder of a text in a clip path: a path over the text's bounds `b` with id `id`,
/// transform `transform`, properties `props` ([`clip_props`]) and the conditions of element `n`.
/// usvg drops a group in a clip path and paints clip path content black whatever its fill, so the
/// placeholder is a path, and the importer finds it by its id ([`TextSlots::find`]).
fn clip_placeholder(out: &mut String, q: &str, id: &str, transform: &str, props: &str, n: XNode, b: Rect) {
    let _ = write!(out, "<{q}path id=\"{}\"", xml_escape(id));
    for a in ["systemLanguage", "requiredFeatures", "requiredExtensions"] {
        if let Some(v) = n.attribute(a) {
            let _ = write!(out, " {a}=\"{}\"", xml_escape(v));
        }
    }
    // In `style`, so no rule that matches the path moves it.
    let transform = if transform.trim().is_empty() { IDENTITY } else { transform };
    let _ = write!(out, " style=\"transform:{};{}display:inline\"", xml_escape(transform), xml_escape(props));
    let _ = write!(out, " d=\"M{} {}H{}V{}H{}Z\"/>", b.x0, b.y0, b.x1, b.y1, b.x0);
}

/// Write the placeholder group for text `t` (namespace prefix `q`), slot `index`, with the text's
/// id when `with_id`.
#[allow(clippy::too_many_arguments)]
fn placeholder(
    out: &mut String,
    q: &str,
    css: &Styles,
    t: XNode,
    style_t: XNode,
    prefix: &str,
    index: usize,
    b: Rect,
    paints: &[String],
    with_id: bool,
) {
    let _ = write!(out, "<{q}g");
    for a in ["id", "systemLanguage", "requiredFeatures", "requiredExtensions"].into_iter().skip(usize::from(!with_id)) {
        if let Some(v) = t.attribute(a) {
            let _ = write!(out, " {a}=\"{}\"", xml_escape(v));
        }
    }
    // The text's own non-inherited properties, overriding any rule that matches the `<g>`. The
    // transform too: a `transform` declaration in a rule or `style` beats the attribute.
    let transform = own_transform(css, style_t);
    let mut style = format!("transform:{};", if transform.is_empty() { IDENTITY } else { &transform });
    for (p, initial) in
        [("opacity", "1"), ("clip-path", "none"), ("mask", "none"), ("filter", "none"), ("mix-blend-mode", "normal"), ("isolation", "auto")]
    {
        let _ = write!(style, "{p}:{};", css.own(style_t, p).as_deref().unwrap_or(initial));
    }
    if let Some(v) = css.own(style_t, "visibility") {
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
    css: Rc<Styles>,
    /// Pixels per inch absolute lengths convert at ([`super::RootUnits::dpi`]).
    dpi: f64,
    /// Viewport size in user units (the base of `x`/`y` percentages).
    viewport: (f64, f64),
    /// For the styles of one `<use>` instance: the element the `<use>` shows and the `<use>`. The
    /// shown element inherits from the `<use>` rather than from its parent (SVG 2 §5.6,
    /// https://www.w3.org/TR/SVG2/struct.html#UseElement).
    host: Option<(roxmltree::NodeId, roxmltree::NodeId)>,
    sizes: RefCell<HashMap<roxmltree::NodeId, f64>>,
    weights: RefCell<HashMap<roxmltree::NodeId, u16>>,
    elems: RefCell<HashMap<roxmltree::NodeId, Rc<ElemStyle>>>,
    /// The first strong characters of each `<text>` read so far, by the `<text>`
    /// ([`Self::first_strong_rtl`]).
    strong: RefCell<HashMap<roxmltree::NodeId, Rc<FirstStrong>>>,
    /// The path each `<textPath>` read so far follows ([`Self::text_path`]); `None` where it has
    /// no usable one. The styles of a `<use>` instance share it, so each path is found and parsed
    /// once however many `<use>` elements show its text.
    paths: Rc<RefCell<HashMap<roxmltree::NodeId, Option<Rc<BezPath>>>>>,
}

#[cfg(test)]
thread_local! {
    /// How many times [`Ctx::read_text_path`] has read a `<textPath>`'s path on this thread.
    static TEXT_PATH_READS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

impl Ctx {
    fn new(xml: &roxmltree::Document, dpi: f64) -> Self {
        let mut cx = Self {
            css: Rc::new(Styles::new(xml)),
            dpi,
            viewport: (100.0, 100.0),
            host: None,
            sizes: RefCell::default(),
            weights: RefCell::default(),
            elems: RefCell::default(),
            strong: RefCell::default(),
            paths: Rc::default(),
        };
        let root = xml.root_element();
        let dim = |a: &str| root.attribute(a).and_then(|v| cx.length(v, DEFAULT_FONT_SIZE, 0.0)).filter(|v| *v > 0.0);
        cx.viewport = match root.attribute("viewBox").and_then(|v| svgtypes::ViewBox::from_str(v).ok()) {
            Some(vb) if vb.w > 0.0 && vb.h > 0.0 => (vb.w, vb.h),
            _ => (dim("width").unwrap_or(100.0), dim("height").unwrap_or(100.0)),
        };
        cx
    }

    /// The styles of element `shown` as `<use>` `u` shows it.
    fn instance(&self, shown: XNode, u: XNode) -> Self {
        Self {
            css: self.css.clone(),
            dpi: self.dpi,
            viewport: self.viewport,
            host: Some((shown.id(), u.id())),
            sizes: RefCell::default(),
            weights: RefCell::default(),
            elems: RefCell::default(),
            strong: RefCell::default(),
            paths: self.paths.clone(),
        }
    }

    /// The element `n` inherits from: its parent, or the `<use>` for the element it shows.
    fn parent<'a, 'i>(&self, n: XNode<'a, 'i>) -> Option<XNode<'a, 'i>> {
        match self.host {
            Some((shown, u)) if n.id() == shown => n.document().get_node(u),
            _ => n.parent_element(),
        }
    }

    /// Element `n` and the elements it inherits from, nearest first.
    fn lineage<'a, 'i>(&self, n: XNode<'a, 'i>) -> impl Iterator<Item = XNode<'a, 'i>> {
        std::iter::successors(Some(n), |a| self.parent(*a))
    }

    /// An inherited property: the nearest specified value other than `inherit`.
    fn prop(&self, n: XNode, name: &str) -> Option<String> {
        self.lineage(n).find_map(|a| self.css.own(a, name).filter(|v| v != "inherit"))
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
        let parent = self.parent(n).map_or(DEFAULT_FONT_SIZE, |p| self.font_size(p));
        let s = self.css.own(n, "font-size").and_then(|v| self.parse_font_size(&v, parent)).unwrap_or(parent);
        self.sizes.borrow_mut().insert(n.id(), s);
        s
    }

    /// Computed `font-weight` of an element (100–900).
    fn weight(&self, n: XNode) -> u16 {
        if let Some(w) = self.weights.borrow().get(&n.id()) {
            return *w;
        }
        let parent = self.parent(n).map_or(400, |p| self.weight(p));
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
        let Some(v) = self.prop(n, prop) else { return (initial, None) };
        match svgtypes::Paint::from_str(&v) {
            Ok(svgtypes::Paint::None) => (Paint::None, None),
            Ok(svgtypes::Paint::Color(c)) => (solid(c), None),
            Ok(svgtypes::Paint::CurrentColor) => {
                (self.prop(n, "color").and_then(|c| svgtypes::Color::from_str(&c).ok()).map_or(Paint::solid(Color::BLACK), solid), None)
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
        if let Some(f) = self.prop(n, "font-family") {
            let fam = f.split(',').next().unwrap_or("").trim().trim_matches(|c| c == '\'' || c == '"').to_string();
            if !fam.is_empty() {
                st.font_family = fam;
            }
        }
        let size = self.font_size(n);
        st.size = size;
        let italic = self.prop(n, "font-style").is_some_and(|s| s == "italic" || s == "oblique");
        st.font_style = style_name(self.weight(n), italic);
        // Inherited: `inherit`, `unset`, `revert` and a value CSS ignores leave the one inherited.
        let inherits = |v: &String| ["inherit", "unset", "revert", "revert-layer"].iter().any(|k| v.trim().eq_ignore_ascii_case(k));
        let own = |a: XNode| css.own(a, "font-variation-settings").filter(|v| !inherits(v)).and_then(|v| variation_settings(&v));
        st.font_axes = self.lineage(n).find_map(own).unwrap_or_default();
        let (fill, mut fill_url) = self.paint(n, "fill");
        st.fill = fill;
        let (stroke, mut stroke_url) = self.paint(n, "stroke");
        if !stroke.is_none() || stroke_url.is_some() {
            st.stroke = stroke;
            st.stroke_width = self.prop(n, "stroke-width").and_then(|w| self.length(&w, size, size)).unwrap_or(1.0);
            st.stroke_cap = match self.prop(n, "stroke-linecap").as_deref() {
                Some("round") => LineCap::Round,
                Some("square") => LineCap::Square,
                _ => LineCap::Butt,
            };
            st.stroke_join = match self.prop(n, "stroke-linejoin").as_deref() {
                Some("round") => LineJoin::Round,
                Some("bevel") => LineJoin::Bevel,
                _ => LineJoin::Miter,
            };
            st.stroke_miter_limit =
                self.prop(n, "stroke-miterlimit").and_then(|m| m.trim().parse::<f64>().ok()).filter(|m| m.is_finite()).map_or(4.0, |m| m.max(1.0));
            let pattern: Vec<f64> =
                self.prop(n, "stroke-dasharray").map_or(vec![], |d| list(&d).filter_map(|v| self.length(v, size, size)).collect());
            let offset = self.prop(n, "stroke-dashoffset").and_then(|o| self.length(&o, size, size)).unwrap_or(0.0);
            st.stroke_dash = Some(Dash { pattern, offset, align_corners: false }).filter(Dash::is_dashed);
        }
        if let Some(v) = self.prop(n, "letter-spacing").and_then(|v| self.length(&v, size, size))
            && size > 0.0
        {
            st.tracking = v / size * 1000.0;
        }
        if let Some(v) = self.prop(n, "kerning").and_then(|v| self.length(&v, size, size))
            && size > 0.0
        {
            st.kerning = Some(v / size * 1000.0);
        }
        if let Some(d) = self.prop(n, "text-decoration") {
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
        let (char_direction, bidi_notes) = self.char_direction(n);
        st.char_direction = char_direction;
        if self.prop(n, "visibility").is_some_and(|v| v == "hidden" || v == "collapse") {
            (st.fill, st.stroke, fill_url, stroke_url) = (Paint::None, Paint::None, None, None);
        }
        let word_spacing = self.prop(n, "word-spacing").and_then(|v| self.length(&v, size, size)).unwrap_or(0.0);
        let order = self.prop(n, "paint-order").and_then(|v| svgtypes::PaintOrder::from_str(&v).ok()).unwrap_or_default().order;
        let at = |k| order.iter().position(|o| *o == k);
        let stroke_first = at(svgtypes::PaintOrderKind::Stroke) < at(svgtypes::PaintOrderKind::Fill);
        let e = Rc::new(ElemStyle { style: st, fill: fill_url, stroke: stroke_url, word_spacing, stroke_first, bidi_notes });
        self.elems.borrow_mut().insert(n.id(), e.clone());
        e
    }

    /// The computed `unicode-bidi` of element `n`, in lowercase. It is not inherited: `normal`
    /// unless `n` sets it, and its parent's value for `inherit`.
    fn unicode_bidi(&self, n: XNode) -> String {
        let mut at = Some(n);
        while let Some(a) = at {
            match self.css.own(a, "unicode-bidi").map(|v| v.trim().to_ascii_lowercase()) {
                Some(v) if v == "inherit" => at = self.parent(a),
                Some(v) => return v,
                None => break,
            }
        }
        "normal".into()
    }

    /// The Character Direction of the characters of element `n` inside a `<text>`, and what it
    /// leaves out (SVG 1.1 §10.7.4,
    /// https://www.w3.org/TR/SVG11/text.html#RelationshipWithBi-directionality, and CSS Writing
    /// Modes 3 §2.2, https://www.w3.org/TR/css-writing-modes-3/#unicode-bidi). A `<tspan>`,
    /// `<textPath>` or `<a>` whose `unicode-bidi` opens an embedding gives its characters an
    /// embedding in its `direction`; one that opens none (`normal`) leaves them in its parent's.
    /// The text model holds one level of embedding and no override or isolation: overrides and
    /// isolates become embeddings, `plaintext` an embedding in the direction of its first strong
    /// character, and an embedding inside another an embedding of its own, each with a note. The
    /// `<text>` itself sets the paragraph direction ([`pin_direction`]) and no Character Direction.
    fn char_direction(&self, n: XNode) -> (Option<ParaDirection>, Vec<&'static str>) {
        let name = n.tag_name().name();
        let bidi = self.unicode_bidi(n);
        if !matches!(name, "tspan" | "textPath" | "a") {
            let notes =
                if name == "text" && matches!(bidi.as_str(), "bidi-override" | "isolate-override") { vec![BIDI_TEXT_OVERRIDE] } else { vec![] };
            return (None, notes);
        }
        let parent = n.parent_element().map(|p| self.elem(p));
        let inherited = parent.as_ref().and_then(|p| p.style.char_direction);
        let mut notes = parent.map_or_else(Vec::new, |p| p.bidi_notes.clone());
        let rtl = |rtl: bool| Some(if rtl { ParaDirection::RightToLeft } else { ParaDirection::LeftToRight });
        let own = || rtl(self.prop(n, "direction").is_some_and(|d| d.trim() == "rtl"));
        let (dir, note) = match bidi.as_str() {
            "embed" => (own(), None),
            "bidi-override" => (own(), Some(BIDI_OVERRIDE)),
            "isolate" => (own(), Some(BIDI_ISOLATE)),
            "isolate-override" => (own(), Some(BIDI_ISOLATE_OVERRIDE)),
            "plaintext" => (rtl(self.first_strong_rtl(n) == Some(true)), Some(BIDI_PLAINTEXT)),
            // `normal`, and a value CSS ignores.
            _ => return (inherited, notes),
        };
        for note in note.into_iter().chain(inherited.map(|_| BIDI_NESTED)) {
            if !notes.contains(&note) {
                notes.push(note);
            }
        }
        (dir, notes)
    }

    /// Whether the first strong character (UAX #9 P2) of the characters [`Self::collect`] reads
    /// from element `n` is right to left; `None` when they hold none. The characters of the whole
    /// `<text>` are read once, as one sequence, so an isolate that opens in one text node and
    /// closes in a later one is skipped whole ([`FirstStrong`]).
    fn first_strong_rtl(&self, n: XNode) -> Option<bool> {
        // The `<text>` (or outermost element) whose characters hold those of `n`.
        let mut root = n;
        while let Some(p) = root.parent_element().filter(|_| root.tag_name().name() != "text" && self.is_inline_child(root)) {
            root = p;
        }
        let strong = self.strong.borrow().get(&root.id()).cloned();
        let strong = strong.unwrap_or_else(|| {
            let mut chars = vec![];
            let mut spans = HashMap::new();
            self.inline_chars(root, &mut chars, &mut spans);
            let s = Rc::new(FirstStrong::new(&chars, spans));
            self.strong.borrow_mut().insert(root.id(), s.clone());
            s
        });
        strong.of(n.id())
    }

    /// Appends the characters of element `n` and of its shown inline children to `chars`, in
    /// document order, and the range each of those elements holds to `spans`.
    fn inline_chars(&self, n: XNode, chars: &mut Vec<char>, spans: &mut HashMap<roxmltree::NodeId, Range<usize>>) {
        let start = chars.len();
        for c in n.children() {
            if c.is_text() {
                chars.extend(c.text().unwrap_or("").chars());
            } else if self.is_inline_child(c) {
                self.inline_chars(c, chars, spans);
            }
        }
        spans.insert(n.id(), start..chars.len());
    }

    /// Whether child `c` of a `<text>` content element holds characters of the text: a shown
    /// `<tspan>`, `<textPath>` or `<a>`.
    fn is_inline_child(&self, c: XNode) -> bool {
        c.is_element()
            && c.tag_name().namespace() == Some(SVG_NS)
            && matches!(c.tag_name().name(), "tspan" | "textPath" | "a")
            && self.css.own(c, "display").as_deref() != Some("none")
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
            } else if self.is_inline_child(c) {
                if c.tag_name().name() == "textPath" {
                    out.path.get_or_insert(c);
                }
                self.collect(c, preserve, out);
            }
        }
        out.spans[slot].1 = start..out.chars.len();
    }

    /// `textLength` and `lengthAdjust` (SVG 1.1 §10.9.1,
    /// https://www.w3.org/TR/SVG11/text.html#TextElementTextLengthAttribute): the characters of
    /// each element with a `textLength` are fitted to that advance, by adding the difference evenly
    /// between them (`spacing`, the default; it becomes kerning) or by stretching them
    /// (`spacingAndGlyphs`; it becomes horizontal scale, kept within 1–10000%). Innermost elements
    /// first, so an element's length takes in its children's. Vertical type is spaced out only.
    fn fit_lengths(&self, spans: &[(XNode, Range<usize>)], cells: &mut [Cell], vertical: bool, label: &str, warnings: &mut Vec<String>) {
        let n = cells.len();
        for (el, r) in spans.iter().rev() {
            let Some(want) = el.attribute("textLength").and_then(|v| self.length(v, self.font_size(*el), self.viewport.0)).filter(|w| *w >= 0.0)
            else {
                continue;
            };
            // Past the largest coordinate, the characters would only be out of reach.
            let want = want.min(vectorcraft_doc::MAX_COORD);
            let Some(part) = cells.get_mut(r.start.min(n)..r.end.min(n)) else { continue };
            let Some(gaps) = part.len().checked_sub(1) else { continue };
            let stretch = el.attribute("lengthAdjust").map(str::trim) == Some("spacingAndGlyphs");
            if stretch && vertical {
                warnings.push(format!("{label}: lengthAdjust=\"spacingAndGlyphs\" on vertical text spaces the characters out instead"));
            }
            // Kerning and scale change shaping (manual kerning turns pair kerning off): measure again.
            for _ in 0..4 {
                let have = advance(&point_text(part));
                if !have.is_finite() || (want - have).abs() < 0.01 {
                    break;
                }
                if stretch && !vertical {
                    let k = want / have;
                    if !k.is_finite() || k <= 0.0 {
                        break;
                    }
                    for c in part.iter_mut() {
                        c.style.h_scale = (c.style.h_scale * k).clamp(1.0, 10_000.0);
                    }
                } else if gaps > 0 {
                    let d = (want - have) / gaps as f64;
                    for c in part.iter_mut().take(gaps) {
                        c.kern += d;
                    }
                } else {
                    break;
                }
            }
        }
    }

    /// The path a `<textPath>` follows, in the text's user space. It is read from the file once
    /// ([`Self::paths`]).
    fn text_path(&self, xml: &roxmltree::Document, tp: XNode) -> Option<Rc<BezPath>> {
        if let Some(known) = self.paths.borrow().get(&tp.id()) {
            return known.clone();
        }
        let path = self.read_text_path(xml, tp).map(Rc::new);
        self.paths.borrow_mut().insert(tp.id(), path.clone());
        path
    }

    /// How many elements the path of `<textPath>` `tp` has, when a text built so far follows it
    /// ([`Self::text_path`]); 0 for any other node.
    fn path_elements(&self, tp: XNode) -> usize {
        self.paths.borrow().get(&tp.id()).and_then(|p| p.as_deref()).map_or(0, |p| p.elements().len())
    }

    /// Find and parse the path `<textPath>` `tp` follows.
    fn read_text_path(&self, xml: &roxmltree::Document, tp: XNode) -> Option<BezPath> {
        #[cfg(test)]
        TEXT_PATH_READS.set(TEXT_PATH_READS.get() + 1);
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
        // What the Character Direction leaves out of the file's `unicode-bidi`, once for the text.
        let mut bidi_notes: Vec<&str> = vec![];
        for (el, _) in cs.spans.iter().filter(|(_, r)| !r.is_empty() && r.start < n) {
            for note in &self.elem(*el).bidi_notes {
                if !bidi_notes.contains(note) {
                    bidi_notes.push(note);
                }
            }
        }
        warnings.extend(bidi_notes.into_iter().map(|note| format!("{label}: {note}")));
        let under = stroke_under(&mut cells, label, warnings);

        let path = cs.path.and_then(|tp| match self.text_path(xml, tp) {
            Some(p) => Some((tp, p)),
            None => {
                warnings.push(format!("{label}: textPath has no usable path; imported as point type"));
                None
            }
        });
        let vertical = self.prop(t, "writing-mode").is_some_and(|m| m.starts_with("tb") || m.starts_with("vertical"));
        // With `unicode-bidi: plaintext`, the text runs in the direction of its first strong
        // character, and each paragraph in that of its own ([`pin_first_strong`]).
        let plaintext = self.unicode_bidi(t) == "plaintext";
        let rtl = if plaintext { self.first_strong_rtl(t) == Some(true) } else { self.prop(t, "direction").is_some_and(|d| d.trim() == "rtl") };
        let pin = |obj: &mut TextObject| {
            pin_direction(obj, rtl);
            if plaintext {
                pin_first_strong(obj);
            }
        };
        // Where the chunk starting at character `i` sits on its anchor: 0 its left (or top) edge,
        // 0.5 its middle, 1 its right (or bottom) edge; on a path, its edge toward the path's
        // start, its middle or its edge toward the path's end. In horizontal type `start` and
        // `end` follow the `direction` of the element holding that character: right-to-left text
        // starts at its right edge, and on a path at the edge toward the path's end (type on a
        // path lays its glyphs out in visual order from its start).
        let anchor_of = |i: usize| {
            let Some(&(_, el)) = cs.chars.get(i) else { return 0.0 };
            let a = match self.prop(el, "text-anchor").as_deref() {
                Some("middle") => 0.5,
                Some("end") => 1.0,
                _ => 0.0,
            };
            // With `unicode-bidi: plaintext` on the `<text>`, a chunk runs in the direction of its
            // first strong character, left to right without one ([`pin_first_strong`]).
            let chunk_rtl = || {
                let end = (i + 1..n).find(|&j| x[j].is_some() || y[j].is_some()).unwrap_or(n);
                let chunk: String = cs.chars[i..end].iter().map(|c| c.0).collect();
                text_first_strong_rtl(&chunk) == Some(true)
            };
            let rtl = if plaintext { chunk_rtl() } else { self.prop(el, "direction").is_some_and(|d| d.trim() == "rtl") };
            if !vertical && rtl { 1.0 - a } else { a }
        };
        let anchor = anchor_of(0);
        self.fit_lengths(&cs.spans, &mut cells, vertical, label, warnings);
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
            // Text that would start before the path's beginning starts at it, with all its glyphs
            // on the path (SVG does not draw the glyphs that fall off it).
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
            let (justify, left, starts, unplaced) = lines(&mut cells, &x, &y, &dx, &dy, &anchor_of, &pin, origin.x, &mut breaks);
            if unplaced > 0 {
                warnings.push(format!(
                    "{label}: {unplaced} absolutely placed character(s) between characters of the other direction keep their place in the line"
                ));
            }
            obj.para.justify = justify;
            obj.xf = Affine::translate((finite(left), finite(origin.y)));
            // Lines anchored differently or at different x (paragraphs aligned or indented
            // differently): each line's paragraph takes its alignment, and indents that put it
            // where it was.
            if starts.iter().any(|&(_, a, ax)| a != starts[0].1 || ax != starts[0].2) {
                for (k, &(_, a, ax)) in starts.iter().enumerate() {
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
        pin(&mut obj);
        keep_reader_anchors(&mut obj);
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

/// The alignment of an SVG text chunk anchored at `anchor` (0 left edge, 0.5 middle, 1 right edge).
fn justify_of(anchor: f64) -> Justify {
    match anchor {
        a if a >= 1.0 => Justify::Right,
        a if a > 0.0 => Justify::Center,
        _ => Justify::Left,
    }
}

/// How far a line's leading may differ from its spacing and still count as it: 0.01 absolute, but
/// never more than a thousandth of the spacing itself (tiny type has tiny gaps).
fn leading_tolerance(spacing: f64) -> f64 {
    0.01_f64.min(spacing.abs() * 1e-3)
}

/// Point type: line breaks, baseline shifts and kerning from the positioning lists. Returns the
/// justification, the x of the first line's left (or anchor) edge and each line's first
/// character, alignment (as for [`justify_of`]) and anchor x. `anchor_of` gives where the chunk
/// starting at a character sits on its anchor, and `pin` sets the paragraph directions of the
/// text ([`pin_direction`]). When characters are placed on their own, every line is left-aligned
/// and its anchor x is its left edge. Also returns how many absolutely placed characters kerning
/// cannot put on their x.
#[allow(clippy::too_many_arguments)]
fn lines(
    cells: &mut [Cell],
    x: &[Option<f64>],
    y: &[Option<f64>],
    dx: &[f64],
    dy: &[f64],
    anchor_of: &dyn Fn(usize) -> f64,
    pin: &dyn Fn(&mut TextObject),
    x0: f64,
    breaks: &mut Vec<Break>,
) -> (Justify, f64, Vec<(usize, f64, f64)>, usize) {
    let n = cells.len();
    let y0 = y[0].unwrap_or(0.0) + dy[0];
    let (mut base, mut cur) = (y0, y0);
    // Line starts (first character, anchor x) and absolutely placed characters (index, x).
    let mut lines = vec![(0, x0)];
    let mut targets: Vec<(usize, f64)> = vec![];
    let mut added = 0usize;
    for i in 1..n {
        let ny = y[i].unwrap_or(cur) + dy[i];
        let lead = cells[i].style.effective_leading();
        let gap = ny - base;
        if let Some(ax) = x[i].filter(|_| gap.is_finite() && gap > lead * 0.5) {
            // Whole multiples of the auto leading add blank lines (not without limit: a far-off y
            // mustn't ask for billions); other spacing is one break.
            let count = (gap / lead).round().clamp(1.0, MAX_LINE_GAP);
            // The first newline just ends the previous line: only the others are blank lines.
            let count = if (gap / count - lead).abs() <= lead * 0.05 && added + (count as usize - 1) <= MAX_LINE_BREAKS { count } else { 1.0 };
            added += count as usize - 1;
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
        let tolerance = leading_tolerance(b.leading);
        if cells[b.at..end].iter().any(|c| (c.style.effective_leading() - b.leading).abs() > tolerance) {
            for c in &mut cells[b.at..end] {
                c.style.leading = Some(b.leading);
            }
        }
    }
    if targets.is_empty() {
        let starts = lines.iter().map(|&(s, lx)| (s, anchor_of(s), lx)).collect();
        return (justify_of(anchor_of(0)), x0, starts, 0);
    }
    // Absolutely placed characters: lay the text out left-aligned and kern each one onto its x.
    // SVG anchors every absolutely placed chunk on its own, so a chunk not anchored at its left
    // edge moves back by its share of its own width. A chunk's edges are those of its glyphs as
    // they are drawn: the first character of a right-to-left chunk is at its right.
    let line_of = |i: usize| lines.partition_point(|(s, _)| *s <= i) - 1;
    // Chunk starts in order: line starts and absolutely placed characters.
    let mut starts: Vec<usize> = lines.iter().map(|l| l.0).chain(targets.iter().map(|t| t.0)).collect();
    starts.sort_unstable();
    // The left and right edges of the chunk starting at character `s`, and the character drawn at its left.
    let edges = |s: usize, pens: &[Option<Pen>]| {
        let e = starts.get(starts.partition_point(|&c| c <= s)).copied().unwrap_or(n);
        let (mut left, mut right, mut at) = (f64::INFINITY, f64::NEG_INFINITY, s);
        for (j, p) in pens.iter().enumerate().take(e).skip(s) {
            let Some(p) = p else { continue };
            if p.start < left {
                (left, at) = (p.start, j);
            }
            right = right.max(p.end);
        }
        (left.is_finite() && right.is_finite()).then_some((left, right, at))
    };
    let mut line_left: Vec<f64> = lines.iter().map(|l| l.1).collect();
    // Where each line's first chunk's left edge goes.
    let mut first_left: Vec<f64> = vec![];
    // Where each absolutely placed character's pen goes, from the left edge of its line's first chunk.
    let mut wants: Vec<(usize, f64)> = vec![];
    // Absolutely placed characters drawn right next to a placed neighbor with no kerning between
    // them (a left-to-right character between right-to-left ones): they stay where the layout puts them.
    let mut unplaced = 0;
    // Kerning changes shaping (pair kerning is off for manually kerned characters): repeat until
    // every character sits on its x.
    for round in 0..4 {
        let (runs, bytes) = assemble(cells, breaks);
        let mut text = with_runs(runs.0);
        pin(&mut text);
        let pens = pens(&measure(&text), &bytes, cells);
        // The edges of each line's first chunk, found once a round.
        let first: Vec<Option<(f64, f64, usize)>> = lines.iter().map(|&(s, _)| edges(s, &pens)).collect();
        if round == 0 {
            // The chunks as they lie before any is moved: each one's left edge on its anchor.
            let left_at = |s: usize, x: f64, chunk: Option<(f64, f64, usize)>| chunk.map_or(x, |(left, right, _)| x - anchor_of(s) * (right - left));
            first_left = lines.iter().zip(&first).map(|(&(s, lx), &chunk)| left_at(s, lx, chunk)).collect();
            wants = targets
                .iter()
                .map(|&(i, x)| {
                    let chunk = edges(i, &pens);
                    // The pen of a right-to-left chunk's first character is right of the chunk's left edge.
                    let pen = pens[i].zip(chunk).map_or(0.0, |(p, (left, ..))| p.start - left);
                    (i, left_at(i, x, chunk) + pen - first_left[line_of(i)])
                })
                .collect();
        }
        // A line's layout starts at the line's left edge, and its first chunk's left edge is `left`
        // from there (right-to-left kerning can move that chunk).
        line_left = first_left.iter().zip(&first).map(|(&x, chunk)| x - chunk.map_or(0.0, |(left, ..)| left)).collect();
        // Kerning a character widens its advance to the right: the glyphs drawn right of it move
        // right, and so does a right-to-left glyph's own left edge (its kerning is at its left). In
        // drawing order the kerning before an absolutely placed character is a cut after the
        // character before it (before that character when it runs right to left), and what is drawn
        // after the cut moves. Places in drawing order are doubled (from 2) to put cuts between them.
        // Each gap between two placed characters drawn one after the other takes the kerning of one
        // cut in it, so placing a character moves none placed before it. Kerning is drawn at the
        // character's horizontal scale, so each correction is divided by it.
        let mut worst = 0.0f64;
        let mut from = 0;
        unplaced = 0;
        while from < wants.len() {
            let l = line_of(wants[from].0);
            let to = from + wants[from..].partition_point(|&(i, _)| line_of(i) == l);
            let on_line = &wants[from..to];
            from = to;
            let Some((p0, _, at)) = first[l] else { continue };
            let Some(home) = pens[at].map(|p| 2 * p.rank + 2) else { continue };
            // A character drawn as part of the glyph before it (a mark) is kerned where that glyph is.
            // The kerning stays on the character just before the placed one: a mark whose kerning
            // differs from its base's is drawn as a glyph of its own, and kerning the base would put
            // the space between the base and its mark.
            // Each search stops at the nearest earlier character with a glyph, at the latest the
            // placed character before it, so the searches of a line read each character once.
            let mut cuts: Vec<(usize, usize)> = on_line
                .iter()
                .filter(|&&(i, _)| pens[i].is_some())
                .filter_map(|&(i, _)| (lines[l].0..i).rev().find_map(|j| pens[j]).map(|k| (2 * k.rank + if k.rtl { 1 } else { 3 }, i - 1)))
                .collect();
            cuts.sort_unstable();
            let mut points: Vec<(usize, f64, f64)> =
                on_line.iter().filter_map(|&(i, want)| pens[i].map(|p| (2 * p.rank + 2, p.start, want))).collect();
            points.sort_by_key(|p| p.0);
            // The character kerned at the first cut between places `a` and `b`.
            let cut = |a: usize, b: usize| cuts.get(cuts.partition_point(|c| c.0 <= a)).filter(|c| c.0 < b).map(|c| c.1);
            let split = points.partition_point(|p| p.0 < home);
            // Drawn right of the line's first chunk: kerning moves the character away from it.
            let (mut prev, mut moved) = (home, 0.0);
            for &(place, start, want) in &points[split..] {
                if let Some(k) = cut(prev, place) {
                    let d = want - (start - p0 + moved);
                    cells[k].kern += d / spacing_scale(&cells[k].style, false);
                    moved += d;
                    worst = worst.max(d.abs());
                } else {
                    unplaced += 1;
                }
                prev = place;
            }
            // Drawn left of it: kerning moves the chunk (and what lies between) away from the character.
            (prev, moved) = (home, 0.0);
            for &(place, start, want) in points[..split].iter().rev() {
                if let Some(k) = cut(place, prev) {
                    let d = start - p0 - moved - want;
                    cells[k].kern += d / spacing_scale(&cells[k].style, false);
                    moved += d;
                    worst = worst.max(d.abs());
                } else {
                    unplaced += 1;
                }
                prev = place;
            }
        }
        if worst < 0.01 {
            break;
        }
    }
    let starts = lines.iter().zip(&line_left).map(|(&(s, _), &lx)| (s, 0.0, lx)).collect();
    (Justify::Left, line_left[0], starts, unplaced)
}

/// Centred and right-aligned lines of point type `obj` where a reader draws them. `text-anchor`
/// aligns a line's advance, which ends after the letter spacing of its last character (SVG
/// export anchors lines so); VectorCraft aligns a line without that tracking
/// ([`vectorcraft_text::LineInfo::trailing`]). Each such line moves back by its share of it: the
/// type moves when every line moves alike, otherwise each paragraph's right indent takes the
/// tracking (a line of imported point type is a paragraph). A right-to-left paragraph's lines
/// keep all their tracking in the layout (`trailing` is 0), so they stay where they are and their
/// whole advance sits on the anchor. Browsers add that letter spacing
/// today; CSS WG issue 1518 and Firefox bugs 1907305 and 1913554 may drop it at a line's end.
fn keep_reader_anchors(obj: &mut TextObject) {
    let share = |j: Justify| match j {
        Justify::Center => 0.5,
        Justify::Right => 1.0,
        _ => 0.0,
    };
    // Laid out only when a line may move: tracked type with a centred or right-aligned paragraph.
    if !matches!(obj.kind, TextKind::Point)
        || obj.vertical
        || obj.runs.iter().all(|r| r.style.tracking == 0.0)
        || obj.paragraph_styles().iter().all(|p| share(p.justify) == 0.0)
    {
        return;
    }
    let plain = obj.plain_text();
    // Each line's paragraph, its tracking left out and how far that moves it.
    let lines: Vec<(usize, f64, f64)> = measure(obj)
        .lines
        .iter()
        .map(|l| {
            let para = plain.get(..l.start).map_or(0, |before| before.matches('\n').count());
            let trailing = if l.trailing.is_finite() { l.trailing } else { 0.0 };
            (para, trailing, share(obj.para_at(para).justify) * trailing)
        })
        .collect();
    let Some(&(_, _, first)) = lines.first() else { return };
    if first == 0.0 && lines.iter().all(|l| l.2 == 0.0) {
        return;
    }
    if lines.iter().all(|l| (l.2 - first).abs() < 1e-9) {
        obj.xf *= Affine::translate((-first, 0.0));
        return;
    }
    let mut styles = obj.paragraph_styles();
    for &(para, trailing, moves) in &lines {
        if let Some(style) = styles.get_mut(para).filter(|_| moves != 0.0) {
            style.right_indent += trailing;
        }
    }
    obj.set_paragraph_styles(styles);
}

/// SVG text runs left to right unless `direction: rtl` on the `<text>` says otherwise (`rtl`),
/// whatever its first strong character: pinned (on every paragraph of `obj`) when that would read
/// as right to left.
fn pin_direction(obj: &mut TextObject, rtl: bool) {
    if rtl {
        obj.edit_paras(None, |pa| pa.direction = Some(ParaDirection::RightToLeft));
    } else if obj.plain_text().split('\n').any(|p| vectorcraft_text::paragraph_is_rtl(p, None)) {
        obj.edit_paras(None, |pa| pa.direction = Some(ParaDirection::LeftToRight));
    }
}

/// `unicode-bidi: plaintext` on a `<text>` (CSS Writing Modes 3 §2.2,
/// https://www.w3.org/TR/css-writing-modes-3/#unicode-bidi): each paragraph of `obj` is pinned to
/// the direction of its first strong character, and left to right without one (UAX #9 P2, P3).
fn pin_first_strong(obj: &mut TextObject) {
    let text = obj.plain_text();
    let mut styles = obj.paragraph_styles();
    for (para, style) in text.split('\n').zip(styles.iter_mut()) {
        let rtl = text_first_strong_rtl(para) == Some(true);
        style.direction = Some(if rtl { ParaDirection::RightToLeft } else { ParaDirection::LeftToRight });
    }
    obj.set_paragraph_styles(styles);
}

/// Whether the first strong character (UAX #9 P2) of `text` is right to left; `None` when it holds
/// none. Paragraph separators count as spaces, so all of `text` is searched.
fn text_first_strong_rtl(text: &str) -> Option<bool> {
    let line: String = text.chars().map(|c| if matches!(c, '\n' | '\r' | '\u{1C}'..='\u{1E}' | '\u{85}' | '\u{2029}') { ' ' } else { c }).collect();
    if vectorcraft_text::paragraph_is_rtl(&line, None) {
        return Some(true);
    }
    // Without a strong character of its own, a right-to-left letter after it is the first one.
    (!vectorcraft_text::paragraph_is_rtl(&format!("{line}\u{5D0}"), None)).then_some(false)
}

/// The first strong characters (UAX #9 P2) of the elements of one `<text>`, found for each element
/// in constant time from one pass over the text's characters. An isolate initiator is skipped to
/// its matching PDI (UAX #9 BD9), and one without a matching PDI to the end; paragraph separators
/// count as spaces, as in [`text_first_strong_rtl`].
struct FirstStrong {
    /// The characters each element holds: a range of indices into `next`.
    spans: HashMap<roxmltree::NodeId, Range<usize>>,
    /// For each character and for the end: the index of the first strong character from there on,
    /// isolates skipped, and whether it is right to left; `None` when there is none.
    next: Vec<Option<(usize, bool)>>,
}

impl FirstStrong {
    fn new(chars: &[char], spans: HashMap<roxmltree::NodeId, Range<usize>>) -> Self {
        use unicode_bidi::BidiClass::{AL, FSI, L, LRI, PDI, R, RLI};
        let class = |c: char| unicode_bidi::bidi_class(c);
        // The matching PDI of each isolate initiator.
        let mut pdi = vec![None; chars.len()];
        let mut open = vec![];
        for (i, &c) in chars.iter().enumerate() {
            match class(c) {
                LRI | RLI | FSI => open.push(i),
                PDI => {
                    if let Some(at) = open.pop() {
                        pdi[at] = Some(i);
                    }
                }
                _ => {}
            }
        }
        let mut next = vec![None; chars.len() + 1];
        for (i, &c) in chars.iter().enumerate().rev() {
            next[i] = match class(c) {
                L => Some((i, false)),
                R | AL => Some((i, true)),
                LRI | RLI | FSI => pdi[i].and_then(|p| next[p + 1]),
                _ => next[i + 1],
            };
        }
        Self { spans, next }
    }

    /// Whether the first strong character of element `n` is right to left; `None` when it holds
    /// none. An isolate that opens inside `n` and closes after it hides the rest of `n`.
    fn of(&self, n: roxmltree::NodeId) -> Option<bool> {
        let span = self.spans.get(&n)?;
        self.next.get(span.start).copied().flatten().filter(|&(at, _)| at < span.end).map(|(_, rtl)| rtl)
    }
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

/// Where the glyph that starts a character is drawn on a line of point type.
#[derive(Clone, Copy)]
struct Pen {
    /// Its left edge. Manual kerning on a right-to-left glyph is at its left, and the glyph is
    /// drawn right of it: its left edge is past that kerning.
    start: f64,
    /// The right edge of its advance.
    end: f64,
    /// Its place in the order its line is drawn in, from the left.
    rank: usize,
    rtl: bool,
}

/// The [`Pen`] of the glyph that starts at each character's byte offset (`bytes`, parallel to `cells`).
fn pens(lay: &TextLayout, bytes: &[usize], cells: &[Cell]) -> Vec<Option<Pen>> {
    // A line's glyphs are laid out in the order they are drawn.
    let mut at: HashMap<usize, (usize, &PositionedGlyph)> = HashMap::with_capacity(lay.glyphs.len());
    for (rank, g) in lay.glyphs.iter().enumerate() {
        at.entry(g.byte).or_insert((rank, g));
    }
    bytes
        .iter()
        .zip(cells)
        .map(|(b, c)| {
            let &(rank, g) = at.get(b)?;
            let mut start = g.origin.x;
            if g.rtl {
                let st = c.final_style();
                let kern = st.kerning.unwrap_or(0.0) / 1000.0 * st.size * spacing_scale(&st, false);
                start += finite(kern);
            }
            Some(Pen { start, end: g.origin.x + g.advance, rank, rtl: g.rtl })
        })
        .collect()
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
            if (blank.effective_leading() - b.leading).abs() > leading_tolerance(b.leading) {
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

#[cfg(test)]
mod tests;
