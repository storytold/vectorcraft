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

use kurbo::{ParamCurveArclen, PathSeg};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{
    AreaOptions, Burasagari, CharStyle, Document, FirstBaseline, Justify, Mojikumi, Node, NodeId, NodeKind, ParaDirection, ParaStyle, PathAlign,
    PathEffect, TextKind, TextObject, VerticalAlign,
};
use vectorcraft_geom::{Affine, PathData, Point};

use crate::ps;

/// What the text document holds of type, and what it leaves out.
pub(in crate::ai_editing) const TYPE_ATTRIBUTES: &str = "attributes of type other than its font, size, leading, tracking, baseline shift, horizontal and vertical scale, solid fill, solid stroke and stroke weight, capitals, small capitals at the document's size, Tate-chu-yoko, Standard Vertical Roman Alignment, paragraph alignment, indents, space before and after, hyphenation on or off and right-to-left direction (for example underline, kerning, OpenType features, swatch links of its colors, character and paragraph styles, tabs, lists, optical margin alignment, area type's inset spacing, vertical alignment and flow, a minimum first baseline with the Ascent first baseline, and type on a path's effect, alignment and spacing and an end bracket at the very end of an open path) are left out of the editing data for other apps";

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

/// A resource set: its resources, each `<< /0 resource >>`, and its display list.
fn set(resources: Vec<V>, display: bool) -> V {
    let mut d = vec![(0, V::List(resources.into_iter().map(|r| dict(vec![(0, r)])).collect()))];
    if display {
        d.push((1, V::List(vec![dict(vec![(0, V::Int(0))])])));
    }
    dict(d)
}

/// A paint: kind 0 gray (`[alpha, lightness]`), 1 RGB, 2 CMYK, as the reader's `paint` reads them.
fn paint(c: &Color) -> V {
    let (kind, values) = match *c {
        Color::Gray { k } => (0, vec![1.0 - f64::from(k.clamp(0.0, 1.0))]),
        Color::Cmyk { c, m, y, k } => (2, [c, m, y, k].iter().map(|v| f64::from(v.clamp(0.0, 1.0))).collect()),
        Color::Rgb { .. } | Color::Lab { .. } => (1, c.to_rgb().iter().map(|v| f64::from(v.clamp(0.0, 1.0))).collect()),
    };
    let values: Vec<f64> = std::iter::once(1.0).chain(values).collect();
    dict(vec![(99, V::Name("SimplePaint")), (0, dict(vec![(0, V::Int(kind)), (1, reals(&values))]))])
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
/// own; some text.
pub(in crate::ai_editing) fn holds(n: &Node, t: &TextObject) -> bool {
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
    // Area type that flows around wrap objects: the data doesn't hold their wrap settings.
    shape_ok
        && t.wrap.is_empty()
        && t.ruby.is_empty()
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

/// Is the end bracket of type on an open path so near the path's end that the reader reads it as
/// no bracket (`Story::node`: within 0.02 of a segment)?
fn end_read_as_none(t: &TextObject) -> bool {
    let (Some((_, end)), Some(segs)) = (t.kind.path_span(), shape(t).as_ref().and_then(segments)) else { return false };
    !closed(&segs) && end < 1.0 - 1e-9 && place_on(&segs, end) >= segs.len() as f64 - 0.02
}

/// Has type `t` attributes the text document leaves out ([`TYPE_ATTRIBUTES`])?
pub(in crate::ai_editing) fn loses(t: &TextObject) -> bool {
    // Small capitals are read back at Document Setup's default size.
    let default_small_caps = vectorcraft_doc::DocSetup::default().small_caps_size;
    // A run's style as the text document holds it: every other attribute at its default. A color
    // is held without its swatch link.
    let held_char = |s: &CharStyle| {
        let unlinked = |p: &Paint| match p {
            Paint::Solid { color, .. } => Paint::solid(*color),
            other => other.clone(),
        };
        let stroked = !s.stroke.is_none();
        CharStyle {
            font_family: s.font_family.clone(),
            font_style: s.font_style.clone(),
            size: s.size,
            leading: s.leading,
            tracking: s.tracking.round(),
            baseline_shift: s.baseline_shift,
            h_scale: s.h_scale,
            v_scale: s.v_scale,
            fill: unlinked(&s.fill),
            stroke: unlinked(&s.stroke),
            stroke_width: if stroked { s.stroke_width } else { CharStyle::default().stroke_width },
            all_caps: s.all_caps,
            small_caps: s.small_caps.filter(|v| *v == default_small_caps),
            tate_chu_yoko: s.tate_chu_yoko,
            // Key 35 holds one of the two: a Tate-chu-yoko run is read back without the other.
            standard_vertical_roman: s.standard_vertical_roman && !s.tate_chu_yoko,
            ..CharStyle::default()
        }
    };
    let run_lost = |s: &CharStyle| {
        let stroked = !s.stroke.is_none();
        let s = CharStyle { stroke_width: if stroked { s.stroke_width } else { CharStyle::default().stroke_width }, ..s.clone() };
        // A Lab color is written as RGB.
        let lab = |p: &Paint| matches!(p, Paint::Solid { color: Color::Lab { .. }, .. });
        s != held_char(&s) || lab(&s.fill) || lab(&s.stroke) || !font_kept(&s)
    };
    // A paragraph's style as the text document holds it, and its text: aligned to the start of
    // its direction (Auto) without a direction of its own, it comes back aligned left, which
    // differs when its first strong letter is right-to-left.
    let para_lost = |(p, text): (&ParaStyle, &str)| {
        let rtl_letter = text.chars().find_map(strong_direction) == Some(true);
        if p.justify == Justify::Auto && p.direction.is_none() && rtl_letter {
            return true;
        }
        let held = ParaStyle {
            justify: p.justify,
            left_indent: p.left_indent,
            right_indent: p.right_indent,
            first_line_indent: p.first_line_indent,
            space_before: p.space_before,
            space_after: p.space_after,
            hyphenate: p.hyphenate,
            direction: p.direction.filter(|d| *d == ParaDirection::RightToLeft),
            // The composition of Japanese punctuation new type gets (Line-end Punctuation Half
            // Width, Standard burasagari) isn't reported: it changes no other script's lines.
            mojikumi: if p.mojikumi == Mojikumi::LineEndHalf { p.mojikumi } else { ParaStyle::default().mojikumi },
            burasagari: if p.burasagari == Burasagari::Standard { p.burasagari } else { ParaStyle::default().burasagari },
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
            && (t.path_effect != PathEffect::default() || t.path_align != PathAlign::default() || t.path_spacing != 0.0 || end_read_as_none(t)))
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

/// The name the font of style `st` is written under, and whether it's TrueType: an installed
/// face's PostScript name; else its family and style as `family-style` (the family alone for
/// Regular), the form the reader's `family_style` splits.
fn font_name(st: &CharStyle) -> (String, bool) {
    let db = vectorcraft_text::FontDb::global();
    // The face of that family and style (the font database gives a fallback face for others).
    match db
        .face_version(&st.font_family, &st.font_style, st.font_version.as_deref())
        .filter(|f| f.family == st.font_family && f.style == st.font_style)
    {
        Some(face) => (face.postscript_name(), face.is_truetype()),
        None if st.font_style.is_empty() || st.font_style == "Regular" => (st.font_family.clone(), false),
        None => (format!("{}-{}", st.font_family, st.font_style.replace(' ', "")), false),
    }
}

/// Does the font of style `st` come back as itself: the family and style the reader makes of the
/// name it is written under (as its `char_style` does)?
fn font_kept(st: &CharStyle) -> bool {
    let (family, style) = crate::family_style(&font_name(st).0);
    let family = vectorcraft_text::FontDb::global().find_family(&family).unwrap_or(family);
    family == st.font_family && style == st.font_style
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

/// The text document being written.
#[derive(Default)]
pub(in crate::ai_editing) struct TextDoc {
    /// Each font's PostScript name and whether it's TrueType.
    fonts: Vec<(String, bool)>,
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
}

impl TextDoc {
    pub(in crate::ai_editing) fn is_empty(&self) -> bool {
        self.stories.is_empty()
    }

    /// The index of the font of `st` in the font set.
    fn font(&mut self, st: &CharStyle) -> i64 {
        let (name, truetype) = font_name(st);
        let i = match self.fonts.iter().position(|(n, _)| *n == name) {
            Some(i) => i,
            None => {
                self.fonts.push((name, truetype));
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
        let fill = solid(&st.fill).flatten();
        // A stroke of no width keeps its paint.
        let stroke = solid(&st.stroke).flatten();
        if let Some(c) = &fill {
            d.push((53, paint(c)));
        }
        if let Some(c) = &stroke {
            d.push((54, paint(c)));
        }
        d.extend([(56, V::Bool(fill.is_some())), (57, V::Bool(stroke.is_some()))]);
        if stroke.is_some() {
            d.push((63, V::Small(st.stroke_width / scale)));
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
        let fonts: usize = self.fonts.iter().map(|(n, _)| n.encode_utf16().count() * 2 + 64).sum();
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
    /// of the thread must be).
    pub(in crate::ai_editing) fn thread(
        &mut self,
        doc: &Document,
        thread: &[NodeId],
        id: NodeId,
        canvas: Affine,
        writable: &dyn Fn(NodeId) -> bool,
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
                        && t.wrap.is_empty()
                        && plain(n)
                        && t.ruby.is_empty()
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
                let lost = split_differs || texts.iter().any(|t| loses(t));
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
            .map(|(name, truetype)| {
                dict(vec![(99, V::Name("CoolTypeFont")), (0, dict(vec![(0, V::Str(name.clone())), (2, V::Int(i64::from(*truetype)))]))])
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
            (4, empty()),
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
            (53, paint(&Color::gray(1.0))),
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
