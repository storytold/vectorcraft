//! Type as the text document of the editing data (`%AI11_BeginTextDocument`): the characters of each
//! text object, their character and paragraph attributes and where they sit, as the text document
//! reader reads them ([`crate::import`]'s `ate.rs`), and a text object stub (`/AI11Text`) in the
//! layers naming its story.
//!
//! The document is ASCII85 on comment lines of nested dictionaries with numbers for keys. Its root
//! holds the version (`/98`), the resources (`/0`: fonts, character and paragraph style sheets, the
//! frames, and empty sets of the resources the writer has none of) and the objects (`/1`: the
//! document's text settings, the stories, and the attributes stories start from). A story holds its
//! characters (paragraphs end in `\r`, a forced line break is `\x03`), its paragraph and character
//! runs (each a length in characters and its attributes) and the frames it flows through: point type
//! (a point on the canvas and, when it is turned, a matrix), area type (the frame's cubic segments,
//! rows, columns, gutters, first baseline) or type on a path (the path's cubic segments and where
//! the type starts and ends on it). Coordinates are the canvas's (y down, 16383 points a side, its
//! center the center of `%AI3_TemplateBox`). The stories carry no composed lines: the app that
//! opens the file lays them out.

use std::collections::{HashMap, HashSet};
use std::fmt::Write;
use std::sync::Arc;

use kurbo::{ParamCurveArclen, PathSeg};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::text::MAX_AKI;
use vectorcraft_doc::{
    AreaOptions, Burasagari, CharStyle, Document, FirstBaseline, Justify, Kinsoku, KinsokuShoriType, LineCap, LineJoin, Mojikumi, Node, NodeId,
    NodeKind, ParaDirection, ParaStyle, PathAlign, PathEffect, TextKind, TextObject, VerticalAlign,
};
use vectorcraft_geom::{Affine, PathData, Point};

use crate::{ai_ops, ps};

/// What the text document holds of type, and what it leaves out.
pub(in crate::ai_editing) const TYPE_ATTRIBUTES: &str = "attributes of type other than its font, size, leading, tracking, baseline shift, horizontal and vertical scale, solid fill, solid stroke and its weight, cap, join and miter limit, capitals, small capitals at the document's size, Tate-chu-yoko, Standard Vertical Roman Alignment, Insert Aki, paragraph alignment, indents, space before and after, hyphenation on or off, right-to-left direction, Kinsoku set, Kinsoku Shori Type, Burasagari and Bunri-Kinshi (for example underline, kerning, OpenType features, swatch links of its colors other than to spot colors, global process colors and Registration, character and paragraph styles, tabs, lists, optical margin alignment, area type's inset spacing, vertical alignment and flow, a minimum first baseline with the Ascent first baseline, and type on a path's effect, alignment and spacing and an end bracket at the very end of an open path) are left out of the editing data for other apps";

/// The version of the text document's form (`/98 << /0 n >>`), as the newest files users sent have it.
const STREAM_VERSION: u32 = 14;
/// The font that shows hidden characters, as the files users sent name it, last in the font set.
const HIDDEN_GLYPH_FONT: &str = "AdobeInvisFont";
/// The hidden characters that font shows: each whitespace character and the glyph it shows as.
const HIDDEN_GLYPHS: [(&str, &str); 7] =
    [(" ", "1"), ("\r", "6"), ("\t", "0"), ("\u{2029}", "5"), ("\u{3}", "5"), ("\u{3000}", "1"), ("\u{ad}", "3")];
/// Most characters a line of ASCII85 holds.
const LINE: usize = 100;

/// A value of the text document.
enum V {
    Int(i64),
    Real(f64),
    /// A real to ten decimals (the turn and scale of a matrix, which the reader divides by).
    Fine(f64),
    /// A real to eight decimals (the type's lengths and factors, which may be small; a size the reader
    /// multiplies by a matrix's scale comes back within these).
    Small(f64),
    Bool(bool),
    Str(String),
    Name(&'static str),
    List(Vec<V>),
    Dict(Vec<(u32, V)>),
}

impl V {
    /// How many values the reader counts in `self` (each value, list and dictionary one).
    fn count(&self) -> usize {
        match self {
            V::List(l) => 1 + l.iter().map(V::count).sum::<usize>(),
            V::Dict(d) => 1 + d.iter().map(|(_, v)| v.count()).sum::<usize>(),
            _ => 1,
        }
    }

    fn write(&self, out: &mut Vec<u8>) {
        match self {
            V::Int(n) => out.extend_from_slice(n.to_string().as_bytes()),
            V::Real(r) => {
                let mut s = String::new();
                ps::push_num(&mut s, if r.is_finite() { *r } else { 0.0 });
                let s = s.trim_end();
                out.extend_from_slice(s.as_bytes());
                // A real is written with a point, as the files users sent write them.
                if !s.contains(['.', 'e', 'E']) {
                    out.extend_from_slice(b".0");
                }
            }
            V::Fine(r) => decimals(*r, 10, out),
            V::Small(r) => decimals(*r, 8, out),
            V::Bool(b) => out.extend_from_slice(if *b { b"true" } else { b"false" }),
            V::Str(s) => {
                out.push(b'(');
                let mut bytes = vec![0xfe, 0xff];
                for u in s.encode_utf16() {
                    bytes.extend_from_slice(&u.to_be_bytes());
                }
                for b in bytes {
                    if matches!(b, b'(' | b')' | b'\\') {
                        out.push(b'\\');
                    }
                    out.push(b);
                }
                out.push(b')');
            }
            V::Name(n) => {
                out.push(b'/');
                out.extend_from_slice(n.as_bytes());
            }
            V::List(l) => {
                out.push(b'[');
                for v in l {
                    out.push(b' ');
                    v.write(out);
                }
                out.extend_from_slice(b" ]");
            }
            V::Dict(d) => {
                out.extend_from_slice(b"<<");
                for (k, v) in d {
                    let _ = write!(Bytes(out), " /{k} ");
                    v.write(out);
                }
                out.extend_from_slice(b" >>");
            }
        }
    }
}

/// `r` to `n` decimals (trailing zeros left out, a point always), into `out`.
fn decimals(r: f64, n: i32, out: &mut Vec<u8>) {
    let k = 10f64.powi(n);
    let r = if r.is_finite() && (r * k).is_finite() { (r * k).round() / k } else { 0.0 };
    let s = format!("{:.*}", n as usize, r + 0.0);
    let s = s.trim_end_matches('0');
    out.extend_from_slice(s.as_bytes());
    if s.ends_with('.') {
        out.push(b'0');
    }
}

/// `fmt::Write` into bytes.
struct Bytes<'a>(&'a mut Vec<u8>);

impl std::fmt::Write for Bytes<'_> {
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        self.0.extend_from_slice(s.as_bytes());
        Ok(())
    }
}

/// A length or a factor of the type's attributes, to eight decimals.
fn real(v: f64) -> V {
    V::Small(v)
}

fn reals(v: &[f64]) -> V {
    V::List(v.iter().map(|x| V::Real(*x)).collect())
}

fn fines(v: &[f64]) -> V {
    V::List(v.iter().map(|x| V::Fine(*x)).collect())
}

fn dict(pairs: Vec<(u32, V)>) -> V {
    V::Dict(pairs)
}

/// The document's Kinsoku sets, in order, each by its name: a paragraph's key 27 is its set's index
/// here (0 Hard, 1 Soft), as the reader's `kinsoku_sets` reads them.
const KINSOKU_SETS: [&str; 2] = ["PhotoshopKinsokuHard", "PhotoshopKinsokuSoft"];

/// A resource set: its resources, each `<< /0 resource >>`, and its display list.
fn set(resources: Vec<V>, display: bool) -> V {
    let mut d = vec![(0, V::List(resources.into_iter().map(|r| dict(vec![(0, r)])).collect()))];
    if display {
        d.push((1, V::List(vec![dict(vec![(0, V::Int(0))])])));
    }
    dict(d)
}

/// A paint: kind 0 gray (`[alpha, lightness]`), 1 RGB, 2 CMYK, as the reader's `paint` reads them.
/// RGB values are an RGB color's own (in the working RGB space) and a Lab color's converted into
/// the working RGB space ([`ai_ops::rgb_values`]). With `style`, the art style that links the color
/// to its swatch (key 5, [`TextDoc::link`]): the reader takes the style's color, at its tint, in
/// place of these values.
fn paint(c: &Color, style: Option<&str>) -> V {
    let (kind, values) = match *c {
        Color::Gray { k } => (0, vec![1.0 - f64::from(k.clamp(0.0, 1.0))]),
        Color::Cmyk { c, m, y, k } => (2, [c, m, y, k].iter().map(|v| f64::from(v.clamp(0.0, 1.0))).collect()),
        Color::Rgb { .. } | Color::Lab { .. } => (1, ai_ops::rgb_values(*c).iter().map(|v| f64::from(*v)).collect()),
    };
    let values: Vec<f64> = std::iter::once(1.0).chain(values).collect();
    let color = dict(vec![(0, V::Int(kind)), (1, reals(&values))]);
    match style {
        Some(name) => dict(vec![(99, V::Name("CAITextPaint")), (5, V::Str(name.to_owned())), (0, color)]),
        None => dict(vec![(99, V::Name("SimplePaint")), (0, color)]),
    }
}

/// A color linked to a swatch, by the swatch's name and its tint ([`TextDoc::links`]).
pub(in crate::ai_editing) type LinkKey = (String, u32);

/// The swatch and tint paint `p` is linked to, if it is.
pub(in crate::ai_editing) fn link_key(p: &Paint) -> Option<LinkKey> {
    match p {
        Paint::Solid { swatch: Some(name), tint, .. } => Some((name.clone(), tint.to_bits())),
        _ => None,
    }
}

/// The color of paint `p` when the text document can hold it (a solid color or none).
fn solid(p: &Paint) -> Option<Option<Color>> {
    match p {
        Paint::Solid { color, .. } => Some(Some(*color)),
        Paint::None => Some(None),
        Paint::Gradient(_) | Paint::Pattern { .. } => None,
    }
}

/// Can the text document hold type `t` of object `n` as it is: point type that isn't mirrored, or
/// area type or type on a path that is only moved ([`shape`]); characters other than inline graphics
/// and ruby; solid fills and strokes; no fills, strokes, effects or opacity mask of the object's
/// own; some text. Area type with wrap shapes only when `wrap_held` ([`wraps_held`]).
pub(in crate::ai_editing) fn holds(n: &Node, t: &TextObject, wrap_held: bool) -> bool {
    let xf = t.xf.as_coeffs();
    let shape_ok = match &t.kind {
        TextKind::Point => xf.iter().all(|v| v.is_finite()) && t.xf.determinant() > 1e-12,
        TextKind::Area { .. } => closed_frame(t),
        // An open path whose ends meet would be read back closed.
        TextKind::OnPath { path, .. } => {
            let open = path.subpaths.iter().any(|sp| !sp.anchors.is_empty() && !sp.closed);
            shape(t).as_ref().and_then(segments).is_some_and(|s| !(open && closed(&s)))
        }
    };
    // Area type with wrap shapes: the text document doesn't hold them. The reader works them out
    // again from the art above the type, which wraps it the same only where `wraps_held` says so.
    shape_ok
        && (t.wrap.is_empty() || wrap_held)
        && t.ruby.is_empty()
        && !t.stacks_warichu()
        && t.runs.iter().any(|r| r.text.chars().any(|c| !c.is_whitespace()))
        && runs_held(t)
        && plain(n)
        && within_reader(t, story_scale(t))
}

/// Is the frame of area type `t` one closed part? An open frame whose ends meet would be read back
/// closed.
fn closed_frame(t: &TextObject) -> bool {
    shape(t).filter(|p| p.subpaths.iter().all(|sp| sp.anchors.is_empty() || sp.closed)).as_ref().and_then(segments).is_some_and(|s| closed(&s))
}

/// Document points to a unit of the story of type `t`: its scale is put into its lengths.
fn story_scale(t: &TextObject) -> f64 {
    1.0 / t.xf.determinant().abs().sqrt()
}

/// Are the lengths and scales of type `t`, written at `scale`, within what the reader's
/// `Texts::style`, `Texts::para` and `AreaFrame::read` read (it reads others as their defaults)?
fn within_reader(t: &TextObject, scale: f64) -> bool {
    const MAX_LENGTH: f64 = 1e5;
    let length = |v: f64| (v / scale).is_finite() && (v / scale).abs() < MAX_LENGTH;
    // Scales are written as fractions to eight decimals.
    let factor = |v: f64| v.is_finite() && v / 100.0 >= 1e-7 && v / 100.0 < 100.0;
    let char_ok = |s: &CharStyle| {
        // Sizes are written to eight decimals.
        length(s.size)
            && s.size / scale >= 1e-7
            && s.leading.is_none_or(|l| length(l) && l / scale >= 1e-7)
            // Tracking is written rounded to a whole number.
            && s.tracking.is_finite()
            && s.tracking.round().abs() < MAX_LENGTH
            && length(s.baseline_shift)
            && factor(s.h_scale)
            && factor(s.v_scale)
            && (s.stroke.is_none() || (s.stroke_width.is_finite() && s.stroke_width >= 0.0 && s.stroke_width / scale < 1e4))
    };
    let para_ok = |p: &ParaStyle| [p.left_indent, p.right_indent, p.first_line_indent, p.space_before, p.space_after].into_iter().all(length);
    let max = vectorcraft_doc::AreaOptions::MAX_ROWS_COLUMNS;
    let area_ok = |o: &AreaOptions| {
        [o.gutter, o.row_gutter.unwrap_or(o.gutter), o.first_baseline_min].iter().all(|g| g.is_finite() && *g >= 0.0 && *g / scale <= 10_000.0)
            && o.rows <= max
            && o.columns <= max
    };
    t.runs.iter().all(|r| char_ok(&r.style))
        && t.paragraph_styles().iter().all(para_ok)
        && (!matches!(t.kind, TextKind::Area { .. }) || area_ok(&t.area))
}

/// Are the runs of `t` characters (no inline graphics, and none of the characters a story ends its
/// paragraphs and lines with, `\r` and `\x03`) filled and stroked with solid colors or nothing?
fn runs_held(t: &TextObject) -> bool {
    t.runs
        .iter()
        .all(|r| r.inline.is_none() && !r.text.contains(['\r', '\u{3}']) && solid(&r.style.fill).is_some() && solid(&r.style.stroke).is_some())
}

/// The story of a thread whose frames `texts` (in order) each hold a slice of it: their runs one
/// after another, and their paragraphs' attributes, a paragraph split between two frames once (a
/// frame whose text before it doesn't end with a paragraph break continues that paragraph), and
/// whether a paragraph split between two frames has other attributes in the second (left out:
/// the story keeps the first's).
fn joined(texts: &[&TextObject]) -> Option<(TextObject, bool)> {
    let mut whole = (*texts.first()?).clone();
    whole.runs.clear();
    let mut last: Option<ParaStyle> = None;
    let mut differs = false;
    // Whether the story so far has text, and ends with a paragraph break.
    let (mut started, mut ended) = (false, false);
    for t in texts.iter().filter(|t| t.runs.iter().any(|r| !r.text.is_empty())) {
        let continues = started && !ended;
        (started, ended) = (true, t.plain_text().ends_with('\n'));
        let own = t.paragraph_styles();
        if continues && own.first() != last.as_ref() {
            differs = true;
        }
        last = own.last().cloned();
        whole.runs.extend(t.runs.iter().filter(|r| !r.text.is_empty()).cloned());
    }
    if whole.runs.is_empty() {
        whole.runs = texts.first()?.runs.clone();
    }
    whole.set_paragraph_styles(vectorcraft_text::thread::story_paras(texts, &[]));
    Some((whole, differs))
}

/// The frame of area type or the path of type on a path, in document coordinates: when the matrix
/// of type `t` only moves it and scales it by one factor (as a large canvas does), its frame or
/// path through that matrix ([`story_scale`] puts the factor into its lengths). `None` for point
/// type and for type that is turned, mirrored, skewed or scaled by two factors.
fn shape(t: &TextObject) -> Option<PathData> {
    let [a, b, c, d, e, f] = t.xf.as_coeffs();
    let even = a > 0.0 && (a - d).abs() <= a * 1e-9 && b == 0.0 && c == 0.0 && e.is_finite() && f.is_finite();
    match &t.kind {
        TextKind::Area { frame } if even => Some(frame.transformed(t.xf)),
        TextKind::OnPath { path, .. } if even => Some(path.transformed(t.xf)),
        _ => None,
    }
}

/// Has text object `n` no fills, strokes or effects of its own that paint or show, and no opacity
/// mask (the reader doesn't keep a text object's mask)?
fn plain(n: &Node) -> bool {
    n.appearance.items.iter().all(|i| !i.visible() || (i.paint().is_none() && !i.effects().iter().any(|e| e.visible)))
        && !n.appearance.effects.iter().any(|e| e.visible)
        && n.extras().mask.is_none()
}

/// The area type of `doc` whose wrap shapes the reader of the editing data works out again as they
/// are ([`vectorcraft_doc::text::resolve_wrap`], which the reader runs on what it reads): the
/// shapes are the ones the wrap objects above it give it now, and each of those objects is written
/// as itself and read back with the same outline and options ([`wrap_source_held`]), in layers
/// and groups written as they are ([`plain_containers`]). `styled`: the objects written with an
/// art style. Empty when the shapes of the document's area type would hold more anchors than the
/// reader copies into them ([`crate::import::MAX_WRAP_WORK`]).
pub(in crate::ai_editing) fn wraps_held(doc: &Document, styled: &HashMap<NodeId, Arc<Node>>) -> HashSet<NodeId> {
    let mut held = HashSet::new();
    if vectorcraft_doc::text::wrap_work(doc) > crate::import::MAX_WRAP_WORK {
        return held;
    }
    let mut plain_in = HashSet::new();
    for layer in &doc.layers {
        plain_containers(layer, true, styled, &mut plain_in);
    }
    let mut checked: HashMap<NodeId, bool> = HashMap::new();
    vectorcraft_doc::text::wrap_sources(doc, |n, sources, shapes| {
        let NodeKind::Text(t) = &n.kind else { return };
        if t.wrap.is_empty() || shapes != t.wrap || !plain_in.contains(&n.id) {
            return;
        }
        let ok = sources.iter().all(|s| *checked.entry(s.id).or_insert_with(|| plain_in.contains(&s.id) && wrap_source_held(s, styled)));
        if ok {
            held.insert(n.id);
        }
    });
    held
}

/// Adds to `out` the objects with text wrap options and the text objects in `n` (a top-level layer
/// when `top`) that sit only in layers and groups written as they are: layers and sublayers that
/// don't clip and have no effects, and groups that don't clip, have no fills, strokes, effects,
/// art style, opacity mask or graph and aren't written with an art style (`styled`).
fn plain_containers(n: &Node, top: bool, styled: &HashMap<NodeId, Arc<Node>>, out: &mut HashSet<NodeId>) {
    let plain = match &n.kind {
        NodeKind::Layer { clip, .. } => !clip && n.appearance.effects.is_empty(),
        NodeKind::Group { clip, .. } => !top && !clip && n.appearance.items.is_empty() && own_plain(n, styled),
        _ => false,
    };
    if !plain {
        return;
    }
    for c in n.children().map(Vec::as_slice).unwrap_or_default() {
        if c.extras().wrap.is_some() || matches!(c.kind, NodeKind::Text(_)) {
            out.insert(c.id);
        }
        plain_containers(c, false, styled, out);
    }
}

/// Has object `n` no effects, art style, opacity mask or graph, and isn't it a ruler guide or
/// written with an art style (`styled`)?
fn own_plain(n: &Node, styled: &HashMap<NodeId, Arc<Node>>) -> bool {
    !styled.contains_key(&n.id)
        && n.appearance.effects.is_empty()
        && n.appearance.items.iter().all(|i| i.effects().is_empty())
        && n.extras().graphic_style.is_none()
        && n.extras().mask.is_none()
        && n.extras().graph.is_none()
        && !n.is_ruler_guide()
}

/// Is wrap object `n` written as itself and read back with the same outline and text wrap options:
/// an offset the writer and the reader keep as it is (-1000 to 1000 pt) and a plain object
/// ([`plain_wrap_art`])?
fn wrap_source_held(n: &Node, styled: &HashMap<NodeId, Arc<Node>>) -> bool {
    n.extras().wrap.is_some_and(|w| w.offset.is_finite() && w.offset.abs() <= 1000.0) && plain_wrap_art(n, styled)
}

/// Is `n` written as one object of its own kind that the reader reads back with the same outline
/// ([`vectorcraft_doc::text::resolve_wrap`]): a path of one subpath, a compound path of visible
/// paths with two subpaths or more in all, each painted with no paint, a solid fill, a plain line of a solid color or both, or a
/// group that doesn't clip of such paths, compound paths and groups (hidden ones too) without text
/// wrap options of their own; each without effects, art style, opacity mask or graph
/// ([`own_plain`]). Each subpath has two anchors or more.
fn plain_wrap_art(n: &Node, styled: &HashMap<NodeId, Arc<Node>>) -> bool {
    use vectorcraft_doc::AppearanceItem;
    let solid = |p: &Paint| matches!(p, Paint::Solid { .. });
    let painted = || match super::visible_items(n).as_slice() {
        [] => true,
        [AppearanceItem::Fill(f)] => solid(&f.paint),
        [AppearanceItem::Stroke(s)] => solid(&s.paint) && vectorcraft_effects::stroke::is_plain(s),
        [AppearanceItem::Fill(f), AppearanceItem::Stroke(s)] => solid(&f.paint) && solid(&s.paint) && vectorcraft_effects::stroke::is_plain(s),
        _ => false,
    };
    let whole = |p: &PathData| p.subpaths.iter().all(|sp| sp.anchors.len() >= 2);
    own_plain(n, styled)
        && match &n.kind {
            NodeKind::Path { path, clipping: false, guide: false, .. } => path.subpaths.len() == 1 && whole(path) && painted(),
            // One subpath in all would be written, and read back, as a path.
            NodeKind::Compound { children, .. } => {
                let subpaths: usize = children.iter().filter_map(|c| c.path_data()).map(|p| p.subpaths.len()).sum();
                let members = children.iter().all(|c| {
                    c.visible
                        && matches!(&c.kind, NodeKind::Path { path, clipping: false, guide: false, .. } if !path.subpaths.is_empty() && whole(path))
                });
                painted() && subpaths > 1 && members
            }
            NodeKind::Group { children, clip: false } => {
                n.appearance.items.is_empty() && children.iter().all(|c| c.extras().wrap.is_none() && plain_wrap_art(c, styled))
            }
            _ => false,
        }
}

/// Is the end bracket of type on an open path so near the path's end that the reader reads it as
/// no bracket (`Story::node`: within 0.02 of a segment)?
fn end_read_as_none(t: &TextObject) -> bool {
    let (Some((_, end)), Some(segs)) = (t.kind.path_span(), shape(t).as_ref().and_then(segments)) else { return false };
    !closed(&segs) && end < 1.0 - 1e-9 && place_on(&segs, end) >= segs.len() as f64 - 0.02
}

/// Insert Aki Left or Right `v` as the text document holds it: 0 to [`MAX_AKI`] in 1/1000 em, and 0
/// for a value that is not a number.
fn held_aki(v: f64) -> f64 {
    if v.is_nan() { 0.0 } else { v.clamp(0.0, MAX_AKI) }
}

/// Has type `t` attributes the text document leaves out ([`TYPE_ATTRIBUTES`])? `fonts` holds the
/// fonts already resolved.
fn loses(t: &TextObject, fonts: &mut ResolvedFonts, links: &HashMap<LinkKey, Option<String>>) -> bool {
    // Small capitals are read back at Document Setup's default size.
    let default_small_caps = vectorcraft_doc::DocSetup::default().small_caps_size;
    // Is paint `p` held with its swatch link, by an art style ([`TextDoc::link`])?
    let linked = |p: &Paint| link_key(p).is_some_and(|k| matches!(links.get(&k), Some(Some(_))));
    // A run's style as the text document holds it: every other attribute at its default. A color
    // is held without its swatch link, unless an art style holds the link.
    let held_char = |s: &CharStyle| {
        let unlinked = |p: &Paint| match p {
            Paint::Solid { color, .. } if !linked(p) => Paint::solid(*color),
            other => other.clone(),
        };
        let stroked = !s.stroke.is_none();
        CharStyle {
            font_family: s.font_family.clone(),
            font_style: s.font_style.clone(),
            // The font record's version (`/5`) names the face; whether it comes back is `font_kept`'s.
            font_version: s.font_version.clone(),
            size: s.size,
            leading: s.leading,
            tracking: s.tracking.round(),
            baseline_shift: s.baseline_shift,
            h_scale: s.h_scale,
            v_scale: s.v_scale,
            fill: unlinked(&s.fill),
            stroke: unlinked(&s.stroke),
            stroke_width: if stroked { s.stroke_width } else { CharStyle::default().stroke_width },
            stroke_cap: if stroked { s.stroke_cap } else { CharStyle::default().stroke_cap },
            stroke_join: if stroked { s.stroke_join } else { CharStyle::default().stroke_join },
            stroke_miter_limit: if stroked { miter_limit(s.stroke_miter_limit) } else { CharStyle::default().stroke_miter_limit },
            all_caps: s.all_caps,
            small_caps: s.small_caps.filter(|v| *v == default_small_caps),
            tate_chu_yoko: s.tate_chu_yoko,
            // Key 35 holds one of the two: a Tate-chu-yoko run is read back without the other.
            standard_vertical_roman: s.standard_vertical_roman && !s.tate_chu_yoko,
            aki_left: held_aki(s.aki_left),
            aki_right: held_aki(s.aki_right),
            ..CharStyle::default()
        }
    };
    let mut run_lost = |s: &CharStyle| {
        let stroked = !s.stroke.is_none();
        // Without a stroke, its weight, cap, join and miter limit draw nothing: nothing is lost.
        let unstroked = CharStyle::default();
        let s = if stroked {
            s.clone()
        } else {
            CharStyle {
                stroke_width: unstroked.stroke_width,
                stroke_cap: unstroked.stroke_cap,
                stroke_join: unstroked.stroke_join,
                stroke_miter_limit: unstroked.stroke_miter_limit,
                ..s.clone()
            }
        };
        // A Lab color is written as RGB, unless an art style holds it with its link.
        let lab = |p: &Paint| matches!(p, Paint::Solid { color: Color::Lab { .. }, .. }) && !linked(p);
        s != held_char(&s) || lab(&s.fill) || lab(&s.stroke) || !fonts.kept(&s)
    };
    // A paragraph's style as the text document holds it, and its text: aligned to the start of
    // its direction (Auto) without a direction of its own, it comes back aligned left, which
    // differs when its first strong letter is right-to-left.
    let para_lost = |(p, text): (&ParaStyle, &str)| {
        let rtl_letter = text.chars().find_map(strong_direction) == Some(true);
        if p.justify == Justify::Auto && p.direction.is_none() && rtl_letter {
            return true;
        }
        let hangul =
            text.chars().any(|c| matches!(c, '\u{1100}'..='\u{11FF}' | '\u{3130}'..='\u{318F}' | '\u{A960}'..='\u{A97F}' | '\u{AC00}'..='\u{D7FF}' | '\u{FFA0}'..='\u{FFDC}'));
        let held = ParaStyle {
            justify: p.justify,
            left_indent: p.left_indent,
            right_indent: p.right_indent,
            first_line_indent: p.first_line_indent,
            space_before: p.space_before,
            space_after: p.space_after,
            hyphenate: p.hyphenate,
            direction: p.direction.filter(|d| *d == ParaDirection::RightToLeft),
            // The Mojikumi set new type gets (Line-end Punctuation Half Width) isn't reported: it
            // changes no other script's lines.
            mojikumi: if p.mojikumi == Mojikumi::LineEndHalf { p.mojikumi } else { ParaStyle::default().mojikumi },
            kinsoku: p.kinsoku,
            kinsoku_type: p.kinsoku_type,
            burasagari: p.burasagari,
            bunri_kinshi: p.bunri_kinshi,
            // The text document has no Korean Line Break Anywhere: it reads back off, which
            // changes only the lines of a paragraph with Hangul in it.
            korean_line_break_anywhere: p.korean_line_break_anywhere && !hangul,
            ..ParaStyle::default()
        };
        *p != held
    };
    let area = AreaOptions::default();
    let text = t.plain_text();
    t.runs.iter().any(|r| run_lost(&r.style))
        || t.paragraph_styles().iter().zip(text.split('\n')).any(para_lost)
        || t.optical_margin
        || t.system_layout
        || !t.fractional_widths
        || (matches!(t.kind, TextKind::Area { .. })
            && (t.area.inset != area.inset
                || t.area.vertical_align != VerticalAlign::default()
                || t.area.flow != area.flow
                || t.area.fit != area.fit
                // The text document holds the Ascent first baseline by leaving it out, without a minimum.
                || (t.area.first_baseline == FirstBaseline::Ascent && t.area.first_baseline_min != 0.0)))
        || (matches!(t.kind, TextKind::OnPath { .. })
            && (t.path_effect != PathEffect::default()
                || t.path_align != PathAlign::default()
                || t.path_spacing != 0.0
                || end_read_as_none(t)
                // The writer writes type that starts before its open path starting at the
                // path's start.
                || t.kind.starts_before_path()))
}

/// Key 35 of a run in style `st`, as the reader's `Texts::style` reads it: 3 for Tate-chu-yoko, 1
/// (Standard) for Standard Vertical Roman Alignment, else 2 (rotated).
fn baseline_direction(st: &CharStyle) -> i64 {
    if st.tate_chu_yoko {
        3
    } else if st.standard_vertical_roman {
        1
    } else {
        2
    }
}

/// A font of the font set, as its `CoolTypeFont` record names it.
#[derive(Clone, Debug, PartialEq)]
struct FontRecord {
    /// The PostScript name (`/0`).
    name: String,
    /// Whether its outlines are TrueType (`/2`).
    truetype: bool,
    /// For a variable font, where in its design space the type is set (`/4`): each `fvar` axis in
    /// the font's order, in the axis's own units. Empty for a static font, which has none: without
    /// it, an app that opens the file takes a variable font's default instance (#6388).
    design: Vec<f32>,
    /// Its version string (`/5`, name ID 5): for a variable font, and for a static font whose
    /// PostScript name alone gives the reader another installed copy (two copies of one font, see
    /// the reader's `document_font`); else empty.
    version: String,
}

/// The font the font set names for style `st`: an installed face's PostScript name, its version
/// for a variable font or when the name alone gives the reader another installed copy, and for a
/// variable font the setting of each of its axes (the named instance's, or the style's own Variable
/// Font setting, clamped to the axis as layout clamps it); else its family and style as
/// `family-style` (the family alone for Regular), the form the reader's `family_style` splits.
fn font_name(st: &CharStyle) -> FontRecord {
    let db = vectorcraft_text::FontDb::global();
    // The face of that family and style (the font database gives a fallback face for others).
    let record = |name: String| FontRecord { name, truetype: false, design: vec![], version: String::new() };
    match db
        .face_version(&st.font_family, &st.font_style, st.font_version.as_deref())
        .filter(|f| f.family == st.font_family && f.style == st.font_style)
    {
        Some(face) => {
            let axes = face.axes();
            let design: Vec<f32> = axes
                .iter()
                .map(|a| {
                    let own = face.axis_value(a.tag).unwrap_or(a.default);
                    let set = st.font_axes.get(String::from_utf8_lossy(&a.tag).as_ref()).copied().filter(|v| v.is_finite());
                    set.map_or(own, |v| (v as f32).clamp(a.min.min(a.max), a.max.max(a.min)))
                })
                .collect();
            let name = face.postscript_name();
            // The reader takes the face of a name alone as `postscript_face` finds it.
            let by_name = crate::import::postscript_face(&name, None).is_some_and(|f| f.id() == face.id());
            let version = if design.is_empty() && by_name { String::new() } else { face.version.clone() };
            FontRecord { name, truetype: face.is_truetype(), design, version }
        }
        None if st.font_style.is_empty() || st.font_style == "Regular" => record(st.font_family.clone()),
        None => record(format!("{}-{}", st.font_family, st.font_style.replace(' ', ""))),
    }
}

/// The text document's number for a stroke's cap (key 61), as PostScript numbers them: 0 butt, 1
/// round, 2 projecting.
fn cap_code(cap: LineCap) -> i64 {
    match cap {
        LineCap::Butt => 0,
        LineCap::Round => 1,
        LineCap::Square => 2,
    }
}

/// The text document's number for a stroke's join (key 62), as PostScript numbers them: 0 miter, 1
/// round, 2 bevel.
fn join_code(join: LineJoin) -> i64 {
    match join {
        LineJoin::Miter => 0,
        LineJoin::Round => 1,
        LineJoin::Bevel => 2,
    }
}

/// A stroke's miter limit (key 64) as the reader takes it back: at least 1 and at most 500, 10 when
/// it isn't a number at least 1.
fn miter_limit(v: f64) -> f64 {
    if v.is_finite() && v >= 1.0 { v.min(500.0) } else { 10.0 }
}

/// `v` as the 16.16 fixed-point number a design vector holds (`wght` 700 is 45875200).
fn fixed(v: f32) -> i64 {
    if v.is_finite() { (f64::from(v) * 65536.0).round() as i64 } else { 0 }
}

/// Does the font of style `st` come back as itself: are the family and style the reader makes of
/// the record it is written as (its name, and its version as the reader takes it, as its
/// `char_style` does) the style's own, and the face they and the version read back resolve to the
/// face of the style? A version that isn't installed draws in the face the font resolves to anyway.
fn font_kept(st: &CharStyle) -> bool {
    let record = font_name(st);
    let version = crate::import::record_version(&record.version);
    let (family, style, read) = crate::import::document_font(&record.name, version);
    let db = vectorcraft_text::FontDb::global();
    let face = |family: &str, style: &str, version: Option<&str>| db.face_version(family, style, version).map(|f| f.id());
    family == st.font_family
        && style == st.font_style
        && face(&family, &style, read.as_deref()) == face(&st.font_family, &st.font_style, st.font_version.as_deref())
}

/// The font fields of a style, as [`ResolvedFonts`] keeps what they resolve to: the family, style
/// and version, and each Variable Font setting as the bits of its value, so two settings that
/// differ are two keys even where they are written as one number.
#[derive(PartialEq, Eq, Hash)]
struct FontKey {
    family: String,
    style: String,
    version: Option<String>,
    axes: Vec<(String, u64)>,
}

impl FontKey {
    fn of(st: &CharStyle) -> Self {
        FontKey {
            family: st.font_family.clone(),
            style: st.font_style.clone(),
            version: st.font_version.clone(),
            axes: st.font_axes.iter().map(|(tag, v)| (tag.clone(), v.to_bits())).collect(),
        }
    }
}

/// What one style's font fields resolved to, each with the font database's generation
/// ([`vectorcraft_text::FontDb::generation`]) when it was resolved.
#[derive(Default)]
struct Resolved {
    /// Its font record ([`font_name`]).
    record: Option<(u64, FontRecord)>,
    /// Whether its font comes back as itself ([`font_kept`]).
    kept: Option<(u64, bool)>,
}

/// The fonts of the styles written so far, each resolved once for the whole save. A value is used
/// again only while the font database's generation is the one it was resolved in: fonts added or
/// loaded since then resolve it again.
#[derive(Default)]
struct ResolvedFonts(HashMap<FontKey, Resolved>);

#[cfg(test)]
thread_local! {
    /// While set on this thread, [`ResolvedFonts`] keeps nothing: every run's font is resolved again.
    pub(in crate::ai_editing) static RESOLVE_EACH_RUN: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// How many times [`ResolvedFonts`] has resolved a font record or checked a font on this thread.
    pub(in crate::ai_editing) static FONT_RESOLVES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

impl ResolvedFonts {
    /// The value `slot` holds for the font of style `st` when it was resolved in the font
    /// database's current generation, else `resolve(st)`, kept for later runs.
    fn get<T>(&mut self, st: &CharStyle, slot: fn(&mut Resolved) -> &mut Option<(u64, T)>, resolve: fn(&CharStyle) -> T) -> &T {
        let now = vectorcraft_text::FontDb::global().generation();
        let held = slot(self.0.entry(FontKey::of(st)).or_default());
        #[cfg(test)]
        if RESOLVE_EACH_RUN.get() {
            *held = None;
        }
        if held.as_ref().is_some_and(|(generation, _)| *generation != now) {
            *held = None;
        }
        let (_, value) = held.get_or_insert_with(|| {
            #[cfg(test)]
            FONT_RESOLVES.set(FONT_RESOLVES.get() + 1);
            (now, resolve(st))
        });
        value
    }

    /// The font record of style `st` ([`font_name`]).
    fn record(&mut self, st: &CharStyle) -> &FontRecord {
        self.get(st, |r| &mut r.record, font_name)
    }

    /// Whether the font of style `st` comes back as itself ([`font_kept`]).
    fn kept(&mut self, st: &CharStyle) -> bool {
        *self.get(st, |r| &mut r.kept, font_kept)
    }
}

/// Whether `c` is a strong right-to-left letter (`Some(true)`: Hebrew, Arabic, Syriac, Thaana, NKo
/// and their presentation forms) or a strong left-to-right one (`Some(false)`: any other letter).
fn strong_direction(c: char) -> Option<bool> {
    let rtl = matches!(c, '\u{0590}'..='\u{08FF}' | '\u{FB1D}'..='\u{FDFF}' | '\u{FE70}'..='\u{FEFF}');
    if rtl {
        Some(true)
    } else if c.is_alphabetic() {
        Some(false)
    } else {
        None
    }
}

/// The cubic segments of `path`, eight numbers each (start, two handles, end) in document
/// coordinates; `None` unless it has one subpath with points and no other (a frame of the text
/// document is one path), of no more segments than the reader reads.
fn segments(path: &PathData) -> Option<Vec<[f64; 8]>> {
    let mut drawn = path.subpaths.iter().filter(|sp| !sp.anchors.is_empty());
    let sp = drawn.next().filter(|sp| sp.anchors.len() > 1 && sp.segment_count() <= crate::import::MAX_SEGMENTS)?;
    if drawn.next().is_some() {
        return None;
    }
    let bp = PathData::new(vec![sp.clone()]).to_bezpath();
    let out: Vec<[f64; 8]> = bp
        .segments()
        .map(|s| {
            let c = match s {
                PathSeg::Line(l) => kurbo::CubicBez::new(l.p0, l.p0, l.p1, l.p1),
                PathSeg::Quad(q) => q.raise(),
                PathSeg::Cubic(c) => c,
            };
            [c.p0.x, c.p0.y, c.p1.x, c.p1.y, c.p2.x, c.p2.y, c.p3.x, c.p3.y]
        })
        .collect();
    (!out.is_empty() && out.len() <= crate::import::MAX_SEGMENTS && out.iter().flatten().all(|v| v.is_finite())).then_some(out)
}

/// Do `segments` end where they start?
fn closed(segments: &[[f64; 8]]) -> bool {
    matches!((segments.first(), segments.last()), (Some(f), Some(l)) if (f[0] - l[6]).abs() < 1e-3 && (f[1] - l[7]).abs() < 1e-3)
}

/// Where the fraction `f` of the length of `segments` lies, as the reader's `length_fraction`
/// reads it back: a segment's index plus the place in that segment.
fn place_on(segments: &[[f64; 8]], f: f64) -> f64 {
    const ACCURACY: f64 = 1e-4;
    let cubic = |s: &[f64; 8]| kurbo::CubicBez::new((s[0], s[1]), (s[2], s[3]), (s[4], s[5]), (s[6], s[7]));
    let lengths: Vec<f64> = segments.iter().map(|s| cubic(s).arclen(ACCURACY)).collect();
    let total: f64 = lengths.iter().sum();
    if !(total.is_finite() && total > 0.0) {
        return 0.0;
    }
    let mut left = f.clamp(0.0, 1.0) * total;
    for (i, (s, len)) in segments.iter().zip(&lengths).enumerate() {
        if left <= *len || i + 1 == segments.len() {
            let t = if *len > 0.0 { cubic(s).inv_arclen(left.min(*len), ACCURACY) } else { 0.0 };
            return i as f64 + t.clamp(0.0, 1.0);
        }
        left -= len;
    }
    segments.len() as f64
}

/// Where the text document ended at some point ([`TextDoc::written`]).
#[derive(Clone, Copy)]
pub(in crate::ai_editing) struct TextMark {
    at: (usize, usize, usize, usize, usize),
    values: usize,
    lost: bool,
    planned: usize,
}

/// The text document being written.
#[derive(Default)]
pub(in crate::ai_editing) struct TextDoc {
    /// The fonts the runs name, each once: two settings of one variable font are two fonts.
    fonts: Vec<FontRecord>,
    frames: Vec<V>,
    stories: Vec<V>,
    /// The story and frame each text object of a thread is written as.
    threaded: HashMap<NodeId, (usize, usize)>,
    /// The threads (by their first frame) already planned.
    planned: HashSet<NodeId>,
    /// A thread written since the last [`Self::take_lost`] has attributes the text document leaves
    /// out ([`loses`], in any of its frames).
    lost: bool,
    /// The bytes of the stories written, each counted once for each text object that names it.
    text_bytes: usize,
    /// The bytes of the frames and stories as written in the text document, and the values the
    /// reader counts in them.
    doc_bytes: usize,
    doc_values: usize,
    /// The fonts of the styles written so far, as they resolve.
    resolved: ResolvedFonts,
    /// The swatch links of type's colors looked at so far, by swatch and tint: the art style that
    /// holds each (a style of one paint, the color with its swatch, which key 5 of a run's paint
    /// names), `None` when the editing data can't hold it ([`super::Writer::link_type_colors`]).
    links: HashMap<LinkKey, Option<String>>,
}

impl TextDoc {
    pub(in crate::ai_editing) fn is_empty(&self) -> bool {
        self.stories.is_empty()
    }

    /// Has type `t` attributes the text document leaves out ([`loses`])?
    pub(in crate::ai_editing) fn loses(&mut self, t: &TextObject) -> bool {
        loses(t, &mut self.resolved, &self.links)
    }

    /// The colors of type whose swatch links are still to be looked at: the fills and strokes of
    /// `texts`' runs that are linked to a swatch and not in [`Self::links`] yet, each once.
    pub(in crate::ai_editing) fn unlooked_links<'t>(&self, texts: &[&'t TextObject]) -> Vec<&'t Paint> {
        let mut seen = HashSet::new();
        texts
            .iter()
            .flat_map(|t| t.runs.iter().flat_map(|r| [&r.style.fill, &r.style.stroke]))
            .filter(|p| link_key(p).is_some_and(|k| !self.links.contains_key(&k) && seen.insert(k)))
            .collect()
    }

    /// Record the art style `style` that holds the swatch link of colors linked as `p` is, or
    /// `None` when none does ([`Self::links`]).
    pub(in crate::ai_editing) fn set_link(&mut self, p: &Paint, style: Option<String>) {
        if let Some(k) = link_key(p) {
            self.links.insert(k, style);
        }
    }

    /// The art style that holds the swatch link of `p`, if one does.
    fn link(&self, p: &Paint) -> Option<&str> {
        self.links.get(&link_key(p)?)?.as_deref()
    }

    /// The index of the font of `st` in the font set.
    fn font(&mut self, st: &CharStyle) -> i64 {
        let font = self.resolved.record(st);
        // Two installed versions of one font are two fonts, so each keeps its own `/5`.
        let same = |f: &FontRecord| {
            f.name == font.name && f.version == font.version && f.design.iter().map(|v| fixed(*v)).eq(font.design.iter().map(|v| fixed(*v)))
        };
        let i = match self.fonts.iter().position(same) {
            Some(i) => i,
            None => {
                self.fonts.push(font.clone());
                self.fonts.len() - 1
            }
        };
        i as i64
    }

    /// The character attributes of a run in style `st`, at `scale` (document points to a unit of
    /// the text document), as the reader's `Texts::style` reads them.
    fn char_features(&mut self, st: &CharStyle, scale: f64) -> V {
        let font = self.font(st);
        let num = |v: f64, d: f64| if v.is_finite() { v } else { d };
        // Sizes to eight decimals: the reader reads a size of 0 as none.
        let mut d = vec![(0, V::Int(font)), (1, V::Small(num(st.size, 12.0) / scale))];
        match st.leading {
            Some(l) if l.is_finite() && l > 0.0 => d.extend([(4, V::Bool(false)), (5, V::Small(l / scale))]),
            _ => d.push((4, V::Bool(true))),
        }
        d.extend([
            (6, V::Small(num(st.h_scale, 100.0) / 100.0)),
            (7, V::Small(num(st.v_scale, 100.0) / 100.0)),
            // Tracking is a whole number of thousandths of an em.
            (8, V::Int(num(st.tracking, 0.0).round() as i64)),
            (9, real(num(st.baseline_shift, 0.0) / scale)),
        ]);
        let caps = if st.all_caps {
            2
        } else if st.small_caps.is_some() {
            1
        } else {
            0
        };
        d.push((12, V::Int(caps)));
        d.push((35, V::Int(baseline_direction(st))));
        // Insert Aki Left and Right in 1/1000 em, as the reader reads keys 49 and 50. No key is no
        // space added: neither the stories' character attributes nor `[Normal Character Style]` set one.
        for (key, v) in [(49, st.aki_left), (50, st.aki_right)] {
            let v = held_aki(v);
            if v > 0.0 {
                d.push((key, real(v)));
            }
        }
        let fill = solid(&st.fill).flatten();
        // A stroke of no width keeps its paint.
        let stroke = solid(&st.stroke).flatten();
        // A color linked to a swatch names the art style that holds the link, the fill's for a
        // stroke too (the reader's `Texts::with_paints`).
        if let Some(c) = &fill {
            d.push((53, paint(c, self.link(&st.fill))));
        }
        if let Some(c) = &stroke {
            d.push((54, paint(c, self.link(&st.stroke))));
        }
        d.extend([(56, V::Bool(fill.is_some())), (57, V::Bool(stroke.is_some()))]);
        if stroke.is_some() {
            d.extend([
                (61, V::Int(cap_code(st.stroke_cap))),
                (62, V::Int(join_code(st.stroke_join))),
                (63, V::Small(st.stroke_width / scale)),
                (64, real(miter_limit(st.stroke_miter_limit))),
            ]);
        }
        dict(d)
    }

    /// The paragraph attributes of `p` at `scale`, as the reader's `Texts::para` reads them.
    fn para_features(p: &ParaStyle, scale: f64) -> V {
        let rtl = p.direction == Some(ParaDirection::RightToLeft);
        // A right-to-left paragraph's left and right alignments are each other's.
        let justify = match p.justify {
            // Auto: the start of the paragraph's direction.
            Justify::Auto => 0,
            Justify::Left if rtl => 1,
            Justify::Right if rtl => 0,
            Justify::Left => 0,
            Justify::Right => 1,
            Justify::Center => 2,
            Justify::JustifyLeft => 3,
            Justify::JustifyRight => 4,
            Justify::JustifyCenter => 5,
            Justify::JustifyAll => 6,
        };
        let len = |v: f64| real(if v.is_finite() { v / scale } else { 0.0 });
        let mut d = vec![
            (0, V::Int(justify)),
            (1, len(p.first_line_indent)),
            (2, len(p.left_indent)),
            (3, len(p.right_indent)),
            (4, len(p.space_before)),
            (5, len(p.space_after)),
            (9, V::Bool(p.hyphenate)),
            (23, V::Bool(p.bunri_kinshi)),
            (
                24,
                V::Int(match p.burasagari {
                    Burasagari::None => 0,
                    Burasagari::Standard => 1,
                    Burasagari::Forced => 2,
                }),
            ),
            (
                25,
                V::Int(match p.kinsoku_type {
                    KinsokuShoriType::PushInFirst => 0,
                    KinsokuShoriType::PushOutFirst => 1,
                    KinsokuShoriType::PushOutOnly => 2,
                }),
            ),
            // The Kinsoku set: `/nil` for None, else its index in the document's sets ([`KINSOKU_SETS`]).
            (
                27,
                match p.kinsoku {
                    Kinsoku::None => V::Name("nil"),
                    Kinsoku::Hard => V::Int(0),
                    Kinsoku::Soft => V::Int(1),
                },
            ),
        ];
        if rtl {
            d.push((33, V::Int(1)));
        }
        dict(d)
    }

    /// The story of type `t`: its characters, runs and the frames `frames` (indices into the
    /// frame set). `scale`: document points to a unit of the text document.
    fn story(&mut self, t: &TextObject, frames: &[usize], scale: f64) -> usize {
        let mut text = String::new();
        let mut char_runs = vec![];
        for r in &t.runs {
            let piece: String = r
                .text
                .chars()
                .map(|c| match c {
                    '\n' => '\r',
                    '\u{2028}' => '\u{3}',
                    c => c,
                })
                .collect();
            let n = piece.chars().count();
            if n == 0 {
                continue;
            }
            text.push_str(&piece);
            let features = self.char_features(&r.style, scale);
            char_runs.push((features, n));
        }
        // Every paragraph ends in `\r`, the last one too; the last run takes it.
        text.push('\r');
        match char_runs.last_mut() {
            Some(last) => last.1 += 1,
            None => {
                let features = self.char_features(&CharStyle::default(), scale);
                char_runs.push((features, 1));
            }
        }
        let paras: Vec<(V, usize)> =
            text.split_inclusive('\r').enumerate().map(|(i, p)| (Self::para_features(t.para_at(i), scale), p.chars().count())).collect();
        let run = |features: V, n: usize, para: bool| {
            let sheet = if para {
                dict(vec![(0, V::Str(String::new())), (5, features), (6, V::Int(0))])
            } else {
                dict(vec![(0, V::Str(String::new())), (5, V::Int(0)), (6, features)])
            };
            dict(vec![(0, dict(vec![(0, sheet)])), (1, V::Int(n as i64))])
        };
        let para_runs: Vec<V> = paras.into_iter().map(|(f, n)| run(f, n, true)).collect();
        let char_runs: Vec<V> = char_runs.into_iter().map(|(f, n)| run(f, n, false)).collect();
        let model = dict(vec![
            (0, V::Str(text)),
            (5, dict(vec![(0, V::List(para_runs))])),
            (6, dict(vec![(0, V::List(char_runs))])),
            (10, dict(vec![(0, V::Int(4)), (2, V::Bool(true))])),
        ]);
        let view = dict(vec![(0, V::List(frames.iter().map(|f| dict(vec![(0, V::Int(*f as i64))])).collect()))]);
        self.stories.push(dict(vec![(0, model), (1, view)]));
        self.stories.len() - 1
    }

    /// The frame of type `t`, `canvas` taking the document onto the canvas (y down) and `scale`
    /// being document points to a unit there. Point type's frame is its point and, unless it is
    /// neither turned nor scaled, the matrix of its type round that point (scale taken out).
    fn frame(&mut self, t: &TextObject, canvas: Affine, scale: f64) -> usize {
        let mut data = vec![];
        let mut f = vec![];
        let vertical = t.vertical.then_some((1, V::Int(2)));
        let segs = |path: &PathData| -> Vec<f64> {
            segments(path)
                .unwrap_or_default()
                .iter()
                .flat_map(|s| {
                    let p = |x: f64, y: f64| canvas * Point::new(x, y);
                    let (a, b, c, d) = (p(s[0], s[1]), p(s[2], s[3]), p(s[4], s[5]), p(s[6], s[7]));
                    [a.x, a.y, b.x, b.y, c.x, c.y, d.x, d.y]
                })
                .collect()
        };
        match &t.kind {
            TextKind::Point => {
                let at = canvas * Point::new(t.xf.as_coeffs()[4], t.xf.as_coeffs()[5]);
                f.push((0, reals(&[at.x, at.y])));
                data.extend(vertical);
                let [a, b, c, d, ..] = t.xf.as_coeffs();
                let s = (a * d - b * c).abs().sqrt();
                let (a, b, c, d) = (a / s, b / s, c / s, d / s);
                if [a - 1.0, b, c, d - 1.0].iter().any(|v| v.abs() > 1e-9) {
                    // Round the point: it stays where it is.
                    let (e, g) = (at.x - (a * at.x + c * at.y), at.y - (b * at.x + d * at.y));
                    let mut m = fines(&[a, b, c, d]);
                    if let V::List(l) = &mut m {
                        l.extend([V::Real(e), V::Real(g)]);
                    }
                    data.push((2, m));
                }
            }
            TextKind::Area { .. } => {
                f.push((1, dict(vec![(0, reals(&shape(t).map(|p| segs(&p)).unwrap_or_default()))])));
                data.push((0, V::Int(1)));
                data.extend(vertical);
                let o = &t.area;
                data.extend([(3, V::Int(o.rows.max(1) as i64)), (4, V::Int(o.columns.max(1) as i64))]);
                data.extend([(7, real(o.row_gutter.unwrap_or(o.gutter) / scale)), (8, real(o.gutter / scale))]);
                let kind = match o.first_baseline {
                    FirstBaseline::Ascent => None,
                    FirstBaseline::Fixed => Some(0),
                    FirstBaseline::CapHeight => Some(2),
                    FirstBaseline::Leading => Some(3),
                    FirstBaseline::XHeight => Some(4),
                    FirstBaseline::EmBoxHeight => Some(5),
                };
                if let Some(k) = kind {
                    data.push((10, dict(vec![(0, V::Int(k)), (1, real(o.first_baseline_min / scale))])));
                }
            }
            TextKind::OnPath { .. } => {
                let path = shape(t).unwrap_or_else(|| PathData::new(vec![]));
                let doc_segs = segments(&path).unwrap_or_default();
                f.push((1, dict(vec![(0, reals(&segs(&path)))])));
                data.push((0, V::Int(2)));
                data.extend(vertical);
                let (start, end) = t.kind.path_span().unwrap_or((0.0, 1.0));
                let n = doc_segs.len() as f64;
                let s = place_on(&doc_segs, start);
                // Round a closed path the end is the start plus the run, past the last segment when
                // the run passes the path's start.
                let e = if closed(&doc_segs) {
                    let run = (end - start).clamp(0.0, 1.0);
                    if run >= 1.0 - 1e-9 { s + n } else { place_on(&doc_segs, end.rem_euclid(1.0)) }
                } else {
                    place_on(&doc_segs, end)
                };
                data.push((6, reals(&[s, e])));
            }
        }
        f.push((2, dict(data)));
        self.frames.push(dict(f));
        self.frames.len() - 1
    }

    /// Room for a story of type `t` named by `frames` text objects, within what the reader reads
    /// of a story and of all the text objects' stories together: taken when there is.
    fn room(&mut self, t: &TextObject, frames: usize) -> bool {
        // The story as written: `\x03` (one byte) for a forced line break, and its last `\r`.
        let len: usize = t.runs.iter().flat_map(|r| r.text.chars()).map(|c| if c == '\u{2028}' { 1 } else { c.len_utf8() }).sum::<usize>() + 1;
        let total = self.text_bytes.saturating_add(len.saturating_mul(frames));
        let fits = len <= crate::import::MAX_STORY && frames <= crate::import::MAX_FRAMES && total <= crate::import::MAX_TEXT;
        if fits {
            self.text_bytes = total;
        }
        fits
    }

    /// Type `t` as a story of one frame, `canvas` taking the document onto the canvas; its story
    /// index, `None` past what the reader reads ([`Self::room`]). Lengths are written in document
    /// points: a large canvas document is written at its scale already, and the reader scales the
    /// text document's lengths with the art. Type that is scaled has its scale taken out of its
    /// frame's matrix, or put into its frame or path, and put into its lengths.
    pub(in crate::ai_editing) fn add(&mut self, t: &TextObject, canvas: Affine) -> Option<usize> {
        let mark = self.mark();
        if !self.room(t, 1) {
            return None;
        }
        let scale = story_scale(t);
        let frame = self.frame(t, canvas, scale);
        let story = self.story(t, &[frame], scale);
        self.fits(mark).then_some(story)
    }

    /// Where the frames, stories and fonts written so far end, and the bytes of the document they
    /// make.
    fn mark(&self) -> (usize, usize, usize, usize, usize) {
        (self.frames.len(), self.stories.len(), self.doc_bytes, self.text_bytes, self.fonts.len())
    }

    /// Whether the frames and stories written since `mark` keep the text document within what the
    /// reader reads of it (its ASCII85, a quarter of the bytes it decodes, with room for the
    /// resources and settings); when they don't, they are taken out again.
    fn fits(&mut self, (frames, stories, bytes, text, fonts_before): (usize, usize, usize, usize, usize)) -> bool {
        // Room for the fonts, sheets and settings: bytes, and values.
        const RESERVE: usize = 1 << 20;
        const RESERVE_VALUES: usize = 1 << 16;
        let (mut added, mut values) = (0, 0);
        for v in self.frames.iter().skip(frames).chain(self.stories.iter().skip(stories)) {
            let mut out = vec![];
            v.write(&mut out);
            added += out.len() + 1;
            values += v.count();
        }
        let total = bytes.saturating_add(added);
        let total_values = self.doc_values.saturating_add(values);
        // The font set: each name in UTF-16 with its dictionary round it.
        // A design vector's numbers and a version string add to it.
        let fonts: usize =
            self.fonts.iter().map(|f| f.name.encode_utf16().count() * 2 + 64 + f.design.len() * 12 + f.version.encode_utf16().count() * 2).sum();
        // ASCII85 writes five characters for every four bytes.
        if total.saturating_add(RESERVE).saturating_add(fonts).saturating_mul(5) / 4 <= crate::import::MAX_TEXT_DOCUMENT / 4
            && total_values.saturating_add(RESERVE_VALUES) <= crate::import::MAX_TEXT_VALUES
        {
            (self.doc_bytes, self.doc_values) = (total, total_values);
            return true;
        }
        self.frames.truncate(frames);
        self.stories.truncate(stories);
        // The fonts only the stories taken out named.
        self.fonts.truncate(fonts_before);
        self.text_bytes = text;
        false
    }

    /// The story and frame text object `id` of a thread is written as, planning the thread
    /// `thread` (its frames in order, all area type the document holds, the first holding the
    /// story) the first time one of its frames is written. `None` when the thread can't be written
    /// as one story: `writable` says which frames are written where type can be text (every frame
    /// of the thread must be), and `wraps_held` which frames with wrap shapes the reader wraps the
    /// same ([`wraps_held`]; every frame with wrap shapes must be one).
    pub(in crate::ai_editing) fn thread(
        &mut self,
        doc: &Document,
        thread: &[NodeId],
        id: NodeId,
        canvas: Affine,
        writable: &dyn Fn(NodeId) -> bool,
        wraps_held: &HashSet<NodeId>,
    ) -> Option<(usize, usize)> {
        let first = *thread.first()?;
        if self.planned.insert(first) {
            let members: Option<Vec<&Node>> = thread.iter().map(|i| doc.node(*i)).collect();
            // The first frame holds the story; the others hold no text of their own.
            // The frames of a story share one orientation (the reader reads no other).
            let vertical = members.as_ref().and_then(|m| m.first()).is_some_and(|n| matches!(&n.kind, NodeKind::Text(t) if t.vertical));
            // The story has one scale for its lengths, so its frames share it.
            let scale = match members.as_ref().and_then(|m| m.first()).map(|n| &n.kind) {
                Some(NodeKind::Text(t)) => story_scale(t),
                _ => 1.0,
            };
            let area = |n: &&Node| match &n.kind {
                NodeKind::Text(t) => {
                    matches!(t.kind, TextKind::Area { .. })
                        && t.vertical == vertical
                        && writable(n.id)
                        // The text document doesn't hold wrap shapes (see `holds`).
                        && (t.wrap.is_empty() || wraps_held.contains(&n.id))
                        && plain(n)
                        && t.ruby.is_empty()
                        && !t.stacks_warichu()
                        && runs_held(t)
                        && (story_scale(t) / scale - 1.0).abs() <= 1e-9
                        && within_reader(t, scale)
                        && closed_frame(t)
                }
                _ => false,
            };
            let ok = members.as_ref().is_some_and(|m| m.len() > 1 && m.iter().all(area));
            if let (true, Some(members)) = (ok, members) {
                let texts: Vec<&TextObject> =
                    members.iter().filter_map(|n| if let NodeKind::Text(t) = &n.kind { Some(t.as_ref()) } else { None }).collect();
                let (whole, split_differs) = joined(&texts)?;
                let mark = self.mark();
                if !whole.runs.iter().any(|r| r.text.chars().any(|c| !c.is_whitespace())) || !self.room(&whole, texts.len()) {
                    return None;
                }
                let lost = split_differs || texts.iter().any(|t| loses(t, &mut self.resolved, &self.links));
                let frames: Vec<usize> = texts.iter().map(|t| self.frame(t, canvas, scale)).collect();
                let story = self.story(&whole, &frames, scale);
                if !self.fits(mark) {
                    return None;
                }
                self.lost |= lost;
                for (k, i) in thread.iter().enumerate() {
                    self.threaded.insert(*i, (story, k));
                }
            }
        }
        self.threaded.get(&id).copied()
    }

    /// Where the text document written so far ends, for [`Self::take_back`].
    pub(in crate::ai_editing) fn written(&self) -> TextMark {
        TextMark { at: self.mark(), values: self.doc_values, lost: self.lost, planned: self.planned.len() }
    }

    /// Take the frames, stories and fonts added since `m` out again, with what they counted. Where
    /// a thread was planned since, its frames name its story, so nothing is taken out.
    pub(in crate::ai_editing) fn take_back(&mut self, m: TextMark) {
        if self.planned.len() != m.planned {
            return;
        }
        let (frames, stories, bytes, text, fonts) = m.at;
        self.frames.truncate(frames);
        self.stories.truncate(stories);
        self.fonts.truncate(fonts);
        (self.doc_bytes, self.text_bytes, self.doc_values, self.lost) = (bytes, text, m.values, m.lost);
    }

    /// Whether a thread written since the last call has attributes the text document leaves out.
    pub(in crate::ai_editing) fn take_lost(&mut self) -> bool {
        std::mem::take(&mut self.lost)
    }

    /// The text document as its section of the program: `%AI11_BeginTextDocument`, the ASCII85 of
    /// the document on comment lines, `%AI11_EndTextDocument`. `doc` gives the text settings.
    pub(in crate::ai_editing) fn section(&self, doc: &Document) -> String {
        let mut bytes = vec![];
        let root: Vec<(u32, V)> = vec![(98, dict(vec![(0, V::Int(i64::from(STREAM_VERSION)))])), (0, self.resources()), (1, self.objects(doc))];
        for (i, (k, v)) in root.iter().enumerate() {
            if i > 0 {
                bytes.push(b' ');
            }
            let _ = write!(Bytes(&mut bytes), "/{k} ");
            v.write(&mut bytes);
        }
        let packed: String = ps::ascii85(&bytes).chars().filter(|c| !c.is_whitespace()).collect();
        let mut s = String::from("%AI11_BeginTextDocument\n/AI11TextDocument : /ASCII85Decode ,\n");
        // Each line starts with `%`, no line's data starts with `_` (a `%_` line is content), and the
        // end marker `~>` stays on one line (the reader looks for it before it joins the lines).
        let mut col = 0;
        s.push('%');
        for ch in packed.chars() {
            if col >= LINE && ch != '_' && !(ch == '>' && s.ends_with('~')) {
                s.push_str("\n%");
                col = 0;
            }
            s.push(ch);
            col += 1;
        }
        s.push_str("\n;\n%AI11_EndTextDocument\n");
        s
    }

    fn resources(&self) -> V {
        let mut fonts: Vec<V> = self
            .fonts
            .iter()
            .map(|f| {
                let mut font = vec![(0, V::Str(f.name.clone())), (2, V::Int(i64::from(f.truetype)))];
                // A variable font's place in its design space (#6388), and the version of a
                // variable font or of a static one its name alone doesn't give back.
                if !f.design.is_empty() {
                    font.push((4, V::List(f.design.iter().map(|v| V::Int(fixed(*v))).collect())));
                }
                if !f.version.is_empty() {
                    font.push((5, V::Str(f.version.clone())));
                }
                dict(vec![(99, V::Name("CoolTypeFont")), (0, dict(font))])
            })
            .collect();
        fonts.push(dict(vec![(99, V::Name("CoolTypeFont")), (0, dict(vec![(0, V::Str(HIDDEN_GLYPH_FONT.into())), (2, V::Int(0))]))]));
        let normal_char = dict(vec![(0, V::Str("[Normal Character Style]".into())), (6, Self::normal_char())]);
        let normal_para = dict(vec![(0, V::Str("[Normal Paragraph Style]".into())), (5, Self::normal_para())]);
        let empty = || set(vec![], false);
        dict(vec![
            (1, set(fonts, true)),
            (2, empty()),
            (3, empty()),
            (4, set(KINSOKU_SETS.iter().map(|name| dict(vec![(0, V::Str((*name).into()))])).collect(), false)),
            (5, set(vec![normal_char], true)),
            (6, set(vec![normal_para], true)),
            (8, set(self.frames.iter().map(clone_v).collect(), false)),
            (9, empty()),
        ])
    }

    /// The character attributes stories start from.
    fn normal_char() -> V {
        dict(vec![
            (0, V::Int(0)),
            (1, real(12.0)),
            (4, V::Bool(true)),
            (6, real(1.0)),
            (7, real(1.0)),
            (8, real(0.0)),
            (9, real(0.0)),
            (12, V::Int(0)),
            // Every run writes its own key 35, so this only sets where new type starts: Standard (1).
            (35, V::Int(1)),
            (53, paint(&Color::gray(1.0), None)),
            (56, V::Bool(true)),
            (57, V::Bool(false)),
            (63, real(1.0)),
        ])
    }

    /// The paragraph attributes stories start from.
    fn normal_para() -> V {
        Self::para_features(&ParaStyle::default(), 1.0)
    }

    fn objects(&self, doc: &Document) -> V {
        let s = &doc.setup;
        let hidden = (self.fonts.len()) as i64;
        let mapping = V::List(HIDDEN_GLYPHS.iter().map(|(c, g)| dict(vec![(0, V::Str((*c).into())), (1, V::Str((*g).into()))])).collect());
        let q = &s.quotes;
        let quotes = dict(vec![
            (0, V::Int(0)),
            (1, V::Str(q.double[0].to_string())),
            (2, V::Str(q.double[1].to_string())),
            (3, V::Str(q.single[0].to_string())),
            (4, V::Str(q.single[1].to_string())),
        ]);
        let pct = |v: f64| real(if v.is_finite() { v / 100.0 } else { 0.0 });
        let settings = dict(vec![
            (0, dict(vec![(0, V::Int(hidden)), (1, mapping)])),
            (1, V::Int(0)),
            (2, V::Int(0)),
            (3, pct(s.superscript.size)),
            (4, pct(s.superscript.position)),
            (5, pct(s.subscript.size)),
            (6, pct(s.subscript.position)),
            (7, pct(s.small_caps_size)),
            (8, V::Bool(s.typographers_quotes)),
            (9, V::List(vec![quotes])),
            (11, V::Int(6)),
            (16, V::Bool(true)),
            (17, V::Int(0)),
        ]);
        dict(vec![(0, settings), (1, V::List(self.stories.iter().map(clone_v).collect())), (2, Self::normal_char()), (3, Self::normal_para())])
    }
}

/// A copy of `v`.
fn clone_v(v: &V) -> V {
    match v {
        V::Int(n) => V::Int(*n),
        V::Real(r) => V::Real(*r),
        V::Fine(r) => V::Fine(*r),
        V::Small(r) => V::Small(*r),
        V::Bool(b) => V::Bool(*b),
        V::Str(s) => V::Str(s.clone()),
        V::Name(n) => V::Name(n),
        V::List(l) => V::List(l.iter().map(clone_v).collect()),
        V::Dict(d) => V::Dict(d.iter().map(|(k, v)| (*k, clone_v(v))).collect()),
    }
}

/// The objects the reader reads of [`stub`] for `t`: the text object, and a path for each subpath
/// of its frame's path.
pub(in crate::ai_editing) fn stub_objects(t: &TextObject) -> usize {
    1 + shape(t).map_or(0, |p| p.subpaths.iter().filter(|sp| !sp.anchors.is_empty()).count())
}

/// The text object stub of story `story`, frame `frame`: `/AI11Text :`, `/FreeUndo`, `/FrameIndex`,
/// `/StoryIndex`, for area type and type on a path the frame's path (`/Art :`, `X=`, the path in
/// art space through `to_art`, `X+`, `; /ConfiningPath ,`), `/TextAntialiasing`, `;`.
pub(in crate::ai_editing) fn stub(s: &mut String, story: usize, frame: usize, t: &TextObject, to_art: Affine) {
    let _ = writeln!(s, "/AI11Text :\n0 /FreeUndo ,\n{frame} /FrameIndex ,\n{story} /StoryIndex ,");
    let area = matches!(t.kind, TextKind::Area { .. });
    if let Some(path) = shape(t) {
        s.push_str("/Art :\nX=\n");
        for sp in path.subpaths.iter().filter(|sp| !sp.anchors.is_empty()) {
            crate::ai_ops::subpath(s, to_art, sp);
            s.push_str(if sp.closed || area { "n\n" } else { "N\n" });
        }
        s.push_str("X+\n; /ConfiningPath ,\n");
    }
    s.push_str("2 /TextAntialiasing ,\n;\n");
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A style's font resolved through [`ResolvedFonts`], the first time and again, is the font
    /// record and the check [`font_name`] and [`font_kept`] give for it: static and variable fonts,
    /// a version of their own, Variable Font settings within and outside the axis's range, not a
    /// number, +0 and -0, an axis the font doesn't have, and a family that isn't installed. A font
    /// resolved before the font database last changed is resolved again.
    #[test]
    fn resolved_fonts_are_the_fonts_resolved_for_each_run() {
        use vectorcraft_text::test_fonts::{VARIABLE_FAMILY, variable_font};
        vectorcraft_text::FontDb::global().add_font(variable_font().unwrap());
        let font = |family: &str, style: &str, version: Option<&str>, axes: &[(&str, f64)]| CharStyle {
            font_family: family.into(),
            font_style: style.into(),
            font_version: version.map(str::to_owned),
            font_axes: axes.iter().map(|(t, v)| (t.to_string(), *v)).collect(),
            ..CharStyle::default()
        };
        let styles = [
            CharStyle::default(),
            font("Source Sans 3", "Bold", None, &[]),
            font("Inter", "Medium", None, &[]),
            font(VARIABLE_FAMILY, "Regular", None, &[]),
            font(VARIABLE_FAMILY, "Bold", Some("Version 1.000"), &[]),
            font(VARIABLE_FAMILY, "Bold", None, &[("wght", 650.0)]),
            font(VARIABLE_FAMILY, "Bold", None, &[("wght", 900.0)]),
            font(VARIABLE_FAMILY, "Bold", None, &[("wght", f64::NAN)]),
            font(VARIABLE_FAMILY, "Bold", None, &[("wght", 0.0)]),
            font(VARIABLE_FAMILY, "Bold", None, &[("wght", -0.0)]),
            font(VARIABLE_FAMILY, "Bold", None, &[("wdth", 80.0), ("wght", 500.0)]),
            font("No Such Family", "Bold", None, &[]),
            font("No Such Family", "", Some("2.0"), &[]),
        ];
        let mut fonts = ResolvedFonts::default();
        for round in ["first", "again"] {
            for st in &styles {
                assert_eq!(fonts.record(st), &font_name(st), "{round}: {st:?}");
                assert_eq!(fonts.kept(st), font_kept(st), "{round}: {st:?}");
            }
        }
        assert_eq!(fonts.0.len(), styles.len(), "each style's font fields are a key of their own");
        // What was resolved before the font database last changed is resolved again.
        let stale = vectorcraft_text::FontDb::global().generation().wrapping_sub(1);
        let wrong = FontRecord { name: "Wrong".into(), truetype: false, design: vec![], version: String::new() };
        for st in &styles {
            let held = fonts.0.entry(FontKey::of(st)).or_default();
            (held.record, held.kept) = (Some((stale, wrong.clone())), Some((stale, !font_kept(st))));
            assert_eq!(fonts.record(st), &font_name(st), "{st:?}");
            assert_eq!(fonts.kept(st), font_kept(st), "{st:?}");
        }
    }
}
