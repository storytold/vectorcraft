use super::writer::Writer;
use super::{
    Affine, AreaOptions, BezPath, CharPosition, CharStyle, CompositeFont, Document, Gradient, GradientKind, Paint, ParaStyle, PathAlign, PathEffect,
    StrokeLayer, TextKind, TextObject, Write, latin, ps, stroke,
};

/// A gradient definition: `%AI5_BeginGradient: (name)`, `(name) type stops Bd`, `[`, the stops
/// (`colorSpec colorStyle midPoint rampPoint %_Bs`, each with its midpoint to the next stop), `BD`,
/// `%AI5_EndGradient`. A linear gradient's stops are listed from the last to the first (§1.4.3). A
/// radial gradient's stops are listed from the first to the last, as in §3.5.2's radial
/// definition (10, then 100).
pub(super) fn gradient_def(s: &mut String, w: &Writer, name: &str, g: &Gradient) {
    let name = latin_string(name);
    let radial = g.kind == GradientKind::Radial;
    let kind = if radial { 1 } else { 0 };
    let _ = writeln!(s, "%AI5_BeginGradient: {name}\n{name} {kind} {} Bd\n[", g.stops.len());
    let mut stops: Vec<_> = g.stops.iter().collect();
    if !radial {
        stops.reverse();
    }
    for stop in stops {
        let paint = Paint::Solid { color: stop.color, swatch: stop.swatch.clone(), tint: stop.tint };
        match w.ink(&paint) {
            Ink::Gray(v) => {
                ps::push_num(s, f64::from(v));
                s.push_str(" 0 ");
            }
            Ink::Cmyk(c) => {
                ps::push_nums(s, &c.map(f64::from));
                s.push_str("1 ");
            }
            Ink::Rgb(c, rgb) => {
                ps::push_nums(s, &c.map(f64::from));
                ps::push_nums(s, &rgb.map(f64::from));
                s.push_str("2 ");
            }
            Ink::Spot { cmyk, name, tint } => {
                ps::push_nums(s, &cmyk.map(f64::from));
                let _ = write!(s, "{} ", latin_string(w.spot_name(&name)));
                ps::push_num(s, f64::from(1.0 - tint));
                s.push_str(" 3 ");
            }
        }
        // The ramp point to five decimals of its percent: a gradient's stops repeated many times
        // keep their places in each run.
        ps::push_num(s, f64::from(stop.midpoint) * 100.0);
        s.push(' ');
        ps::push_num_places(s, f64::from(stop.offset) * 100.0, 5);
        s.push_str(" %_Bs\n");
    }
    s.push_str("BD\n%AI5_EndGradient\n");
}

/// A colour as the format writes it.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Ink {
    /// Lightness (1 is white).
    Gray(f32),
    Cmyk([f32; 4]),
    /// RGB, with its CMYK equivalent.
    Rgb([f32; 4], [f32; 3]),
    /// A spot colour's full CMYK equivalent, name and tint (1: full).
    Spot {
        cmyk: [f32; 4],
        name: String,
        tint: f32,
    },
}

pub(super) use crate::ai_ops::{average, unit};

/// Can `a` be drawn through (a finite, non-flat map)?
pub(super) fn invertible(a: Affine) -> bool {
    let det = a.determinant();
    det.is_finite() && det.abs() > 1e-12
}

/// Name `name` as a PostScript string of its characters' Windows-1252 codes (a character outside
/// Windows-1252 is left out: [`Writer::latin_name`] replaces them first).
pub(super) fn latin_string(name: &str) -> String {
    let bytes: Vec<u8> = name.chars().filter_map(latin::encode).collect();
    ps::bytes(&bytes)
}

/// Can type `t` of document `doc` be written as text: 8-bit text can hold it ([`holds_as_text`]),
/// no character is rotated ([`rotated`]) and no run is set in one of the document's composite
/// fonts, which a reader can't find by its name?
pub(super) fn writable(doc: &Document, t: &TextObject) -> bool {
    holds_as_text(t) && !rotated(t) && !t.runs.iter().any(|r| CompositeFont::named(&doc.composite_fonts, &r.style.font_family).is_some())
}

/// Has type `t` visible characters with Character Rotation? A legacy `.ai` text object's character
/// attributes have none that turns a character (§15), so such type is written as its outlines.
pub(super) fn rotated(t: &TextObject) -> bool {
    t.runs.iter().any(|r| {
        let turn = r.style.rotation;
        turn.is_finite() && turn.rem_euclid(360.0) != 0.0 && r.text.chars().any(|c| !c.is_whitespace())
    })
}

/// Can 8-bit text hold type `t`: horizontal, without ruby or warichu (which a text object can't
/// hold), its runs plain characters (no inline graphics) that 8-bit text can hold, filled and
/// stroked with colours?
pub(super) fn holds_as_text(t: &TextObject) -> bool {
    let solid = |p: &Paint| matches!(p, Paint::Solid { .. } | Paint::None);
    !t.vertical
        && t.ruby.is_empty()
        && !t.stacks_warichu()
        && t.runs.iter().any(|r| !r.text.is_empty())
        && t.runs.iter().all(|r| {
            r.inline.is_none()
                && solid(&r.style.fill)
                && solid(&r.style.stroke)
                && (if r.style.all_caps { r.text.to_uppercase() } else { r.text.clone() }).chars().all(|c| latin::encode(c).is_some())
        })
}

/// Has type `t` attributes a legacy `.ai` text object can't hold (lost when it's written as text)?
pub(super) fn lost_attributes(t: &TextObject) -> bool {
    let run_lost = |s: &CharStyle| {
        s.underline
            || s.strikethrough
            || s.optical_kerning
            || s.small_caps.is_some()
            || s.position != CharPosition::Normal
            || !s.features.is_empty()
            || !s.font_axes.is_empty()
    };
    let para_lost = |p: &ParaStyle| p.space_after != 0.0 || !p.tabs.is_empty() || p.list.is_some();
    t.runs.iter().any(|r| run_lost(&r.style))
        || t.paragraph_styles().iter().any(para_lost)
        || t.optical_margin
        || t.system_layout
        || !t.fractional_widths
        || (matches!(t.kind, TextKind::Area { .. }) && t.area != AreaOptions::default())
        || (matches!(t.kind, TextKind::OnPath { .. })
            && (t.path_effect != PathEffect::default() || t.path_align != PathAlign::default() || t.path_spacing != 0.0 || end_bracket(t)))
        || several_paths(&t.kind)
}

/// Does type on a path `t` end before its path does (an open path's end, or once around a
/// closed path)? A legacy `.ai` text path holds where the type starts and no end (§15).
fn end_bracket(t: &TextObject) -> bool {
    let (TextKind::OnPath { path, .. }, Some((start, end))) = (&t.kind, t.kind.path_span()) else { return false };
    let reach = if path.is_closed() { end - start } else { end };
    reach < 1.0 - 1e-9
}

/// Has the frame of area type, or the path of type on a path, `kind` more than one subpath with
/// anchors? A legacy `.ai` text object is written with the first one only.
fn several_paths(kind: &TextKind) -> bool {
    let (TextKind::Area { frame: path } | TextKind::OnPath { path, .. }) = kind else { return false };
    path.subpaths.iter().filter(|sp| !sp.anchors.is_empty()).count() > 1
}

/// The font type in style `st` is set in: its PostScript name, whether it's TrueType, and its
/// ascent and descent (ems). A font that isn't installed is named after its family and style.
pub(super) fn font_of(st: &CharStyle) -> (String, bool, f64, f64) {
    let db = vectorcraft_text::FontDb::global();
    match db.face_version(&st.font_family, &st.font_style, st.font_version.as_deref()) {
        Some(face) => {
            let (ascent, descent) = face.vertical_metrics();
            let em = face.units_per_em().max(1.0);
            let name = face.postscript_name();
            (name, face.is_truetype(), ascent / em, descent / em)
        }
        None => {
            let squeeze = |s: &str| -> String { s.chars().filter(|c| c.is_ascii_alphanumeric()).collect() };
            let style = squeeze(&st.font_style);
            let name =
                if style.is_empty() || style == "Regular" { squeeze(&st.font_family) } else { format!("{}-{style}", squeeze(&st.font_family)) };
            (if name.is_empty() { "Helvetica".into() } else { name }, false, 0.75, 0.25)
        }
    }
}

/// `v`, or `default` when it isn't finite.
pub(super) fn finite(v: f64, default: f64) -> f64 {
    if v.is_finite() { v } else { default }
}

/// Where a fraction of `path`'s length is, as the start of type on a path gives it (§15.5.2): the
/// index of the segment it's on, plus how far along that segment (its curve parameter).
pub(super) fn segment_point(path: &BezPath, fraction: f64) -> f64 {
    use kurbo::ParamCurveArclen;
    const ACCURACY: f64 = 1e-3;
    let segments: Vec<kurbo::PathSeg> = path.segments().collect();
    let lengths: Vec<f64> = segments.iter().map(|s| s.arclen(ACCURACY)).collect();
    let total: f64 = lengths.iter().sum();
    if !(total.is_finite() && total > 0.0) {
        return 0.0;
    }
    let mut left = fraction.clamp(0.0, 1.0) * total;
    for (i, (seg, len)) in segments.iter().zip(&lengths).enumerate() {
        if left <= *len || i + 1 == segments.len() {
            let t = if *len > 0.0 { seg.inv_arclen(left.min(*len), ACCURACY) } else { 0.0 };
            return i as f64 + t.clamp(0.0, 0.9999);
        }
        left -= len;
    }
    0.0
}

/// Can `p` paint a path object's fill directly (a colour or a linear or radial gradient)?
pub(super) fn direct_fill(p: &Paint) -> bool {
    match p {
        Paint::Solid { .. } => true,
        Paint::Gradient(g) => {
            !matches!(g.gradient.kind, GradientKind::Freeform | GradientKind::Conic | GradientKind::Diamond) && !g.gradient.stops.is_empty()
        }
        _ => false,
    }
}

/// Can stroke `s` be written as the stroke of a path object (a plain line of a colour)?
pub(super) fn direct_stroke(s: &StrokeLayer) -> bool {
    matches!(s.paint, Paint::Solid { .. }) && stroke::is_plain(s)
}
