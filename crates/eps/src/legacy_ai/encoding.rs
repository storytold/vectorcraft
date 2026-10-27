use super::writer::Writer;
use super::*;

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
        ps::push_nums(s, &[f64::from(stop.midpoint) * 100.0, f64::from(stop.offset) * 100.0]);
        s.push_str("%_Bs\n");
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

pub(super) fn unit(v: f32) -> f32 {
    if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 }
}

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

/// Can type `t` be written as text: horizontal, without ruby (which a text object can't hold), its
/// runs plain characters (no inline graphics) that 8-bit text can hold, filled and stroked with
/// colours?
pub(super) fn writable(t: &TextObject) -> bool {
    let solid = |p: &Paint| matches!(p, Paint::Solid { .. } | Paint::None);
    !t.vertical
        && t.ruby.is_empty()
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
            || s.small_caps.is_some()
            || s.position != CharPosition::Normal
            || s.rotation != 0.0
            || !s.features.is_empty()
            || !s.font_axes.is_empty()
    };
    let para_lost = |p: &ParaStyle| p.space_after != 0.0 || !p.tabs.is_empty() || p.list.is_some();
    t.runs.iter().any(|r| run_lost(&r.style))
        || t.paragraph_styles().iter().any(para_lost)
        || (matches!(t.kind, TextKind::Area { .. }) && t.area != AreaOptions::default())
        || (matches!(t.kind, TextKind::OnPath { .. }) && (t.path_effect != PathEffect::default() || t.path_spacing != 0.0))
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
        Paint::Gradient(g) => g.gradient.kind != GradientKind::Freeform && !g.gradient.stops.is_empty(),
        _ => false,
    }
}

/// Can stroke `s` be written as the stroke of a path object (a plain line of a colour)?
pub(super) fn direct_stroke(s: &StrokeLayer) -> bool {
    matches!(s.paint, Paint::Solid { .. }) && stroke::is_plain(s)
}

/// The average colour of a gradient's stops (in RGB).
pub(super) fn average(g: &Gradient) -> Color {
    let n = g.stops.len().max(1) as f32;
    let sum = g.stops.iter().map(|s| s.color.to_rgb()).fold([0.0f32; 3], |a, c| [a[0] + c[0], a[1] + c[1], a[2] + c[2]]);
    Color::rgb(unit(sum[0] / n), unit(sum[1] / n), unit(sum[2] / n))
}
