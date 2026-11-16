//! Type layers of PSD export: point type as a type tool object (the `TySh` block of the format's
//! public specification), so a reader can edit the text. The block holds the text's transform, the
//! text descriptor (the text, its orientation and bounds, and its `EngineData`: the runs of
//! character and paragraph attributes with the fonts they use) and an unwarped warp descriptor.
//! The layer keeps its pixels for readers that show them.

use vectorcraft_color::Paint;
use vectorcraft_doc::text::TypeAntiAlias;
use vectorcraft_doc::{CharStyle, Composer, Justify, Node, NodeKind, ParaStyle, TextKind, TextObject};
use vectorcraft_geom::{Affine, Point, Rect};

/// Point type that a type layer draws as `n` draws: horizontal or vertical point type with text,
/// whose characters use only the attributes a type layer carries ([`carried`]) in solid colours
/// with whole-number tracking, each drawn with its own font (no fallback font), without strokes or
/// inline graphics, whose paragraphs set only an alignment ([`flat`]) and run left to right, and
/// without System Layout, Optical Margin Alignment, ruby, an appearance of its own, effects or an
/// opacity mask. Other type stays pixels.
pub(super) fn editable(n: &Node) -> Option<&TextObject> {
    let NodeKind::Text(t) = &n.kind else { return None };
    let text = t.plain_text();
    let has_cjk = text.chars().any(cjk);
    let flat = |p: &ParaStyle| flat(p, has_cjk);
    let plain = matches!(t.kind, TextKind::Point)
        && !text.is_empty()
        && !t.system_layout
        && !t.optical_margin
        && t.ruby.is_empty()
        && !t.stacks_warichu()
        && flat(&t.para)
        && t.paras.iter().all(flat)
        && text.split('\n').enumerate().all(|(i, p)| !vectorcraft_text::paragraph_is_rtl(p, t.para_at(i).direction))
        && n.appearance.items.is_empty()
        && n.appearance.effects.is_empty()
        && n.extras().mask.is_none()
        && !crate::fx::has_object_fx(n)
        && t.runs
            .iter()
            .all(|r| r.inline.is_none() && matches!(r.style.fill, Paint::Solid { .. }) && matches!(r.style.stroke, Paint::None) && carried(&r.style))
        // A type layer's tracking is a whole number of thousandths of an em.
        && t.runs.iter().all(|r| r.style.tracking.fract() == 0.0)
        // The type layer names one font per run: a character its font lacks would be drawn with
        // another. Breaks, joiners and variation selectors draw no glyph of their own (as the
        // shaper's `drawn_chars` counts them).
        && t.runs.iter().all(|r| face(&r.style).is_some_and(|f| r.text.chars().all(|c| !drawn(c) || f.covers(c))));
    plain.then_some(&**t)
}

/// Does `c` need a glyph of its own: not a control character (tab, line or paragraph break), a
/// line separator, a joiner or a variation selector?
fn drawn(c: char) -> bool {
    !(c.is_control() || matches!(c, '\u{200C}' | '\u{200D}' | '\u{2028}' | '\u{FE00}'..='\u{FE0F}' | '\u{E0100}'..='\u{E01EF}'))
}

/// The face the text of style `s` is drawn with: its font version when that is installed.
fn face(s: &CharStyle) -> Option<std::sync::Arc<vectorcraft_text::FontFace>> {
    vectorcraft_text::FontDb::global().face_version(&s.font_family, &s.font_style, s.font_version.as_deref())
}

/// Does paragraph `p` set only its alignment, the others being at their defaults (no indents,
/// spacing, tabs, bullets or numbers, direction, Auto Leading, word or letter spacing, glyph scaling
/// or hanging punctuation)? Settings that only choose where area type breaks its lines or what
/// hangs where it breaks them (hyphenation, the single-line or every-line composer, kinsoku,
/// Burasagari, Bunri-Kinshi, breaking Korean anywhere) and the paragraph style's name may be set,
/// and so may Mojikumi when `cjk` is false (it spaces only Chinese, Japanese and Korean characters).
/// A paragraph set to a Standard composer is not flat.
fn flat(p: &ParaStyle, cjk: bool) -> bool {
    let only = ParaStyle {
        justify: p.justify,
        style_name: p.style_name.clone(),
        hyphenate: p.hyphenate,
        hyphenation: p.hyphenation.clone(),
        composer: if matches!(p.composer, Composer::SingleLine | Composer::EveryLine) { p.composer } else { ParaStyle::default().composer },
        kinsoku: p.kinsoku,
        kinsoku_type: p.kinsoku_type,
        burasagari: p.burasagari,
        bunri_kinshi: p.bunri_kinshi,
        korean_line_break_anywhere: p.korean_line_break_anywhere,
        mojikumi: if cjk { ParaStyle::default().mojikumi } else { p.mojikumi },
        ..ParaStyle::default()
    };
    only == *p
}

/// Is `c` in a block of Chinese, Japanese or Korean characters or their punctuation?
fn cjk(c: char) -> bool {
    matches!(c, '\u{1100}'..='\u{11FF}' | '\u{2E80}'..='\u{9FFF}' | '\u{A960}'..='\u{A97F}' | '\u{AC00}'..='\u{D7FF}' | '\u{F900}'..='\u{FAFF}' | '\u{FE30}'..='\u{FE4F}' | '\u{FF00}'..='\u{FFEF}' | '\u{20000}'..='\u{3FFFF}')
}

/// Does `s` set only attributes a type layer carries (font and its version, size, leading,
/// tracking, scaling, baseline shift, all caps, underline, strikethrough, fill), the others being
/// at their defaults? Attributes that change nothing drawn without a stroke (the stroke's own
/// settings, overprint, the character style's name) may be set, and so may the language when it
/// is the default, English: USA. The type layer does not record the language, so type in another
/// language stays pixels.
fn carried(s: &CharStyle) -> bool {
    if !s.language.as_deref().is_none_or(|l| l.eq_ignore_ascii_case("en-US")) {
        return false;
    }
    let only = CharStyle {
        language: s.language.clone(),
        font_family: s.font_family.clone(),
        font_style: s.font_style.clone(),
        font_version: s.font_version.clone(),
        size: s.size,
        leading: s.leading,
        tracking: s.tracking,
        baseline_shift: s.baseline_shift,
        h_scale: s.h_scale,
        v_scale: s.v_scale,
        fill: s.fill.clone(),
        stroke: s.stroke.clone(),
        stroke_width: s.stroke_width,
        stroke_cap: s.stroke_cap,
        stroke_join: s.stroke_join,
        stroke_miter_limit: s.stroke_miter_limit,
        stroke_dash: s.stroke_dash.clone(),
        stroke_align: s.stroke_align,
        overprint_fill: s.overprint_fill,
        overprint_stroke: s.overprint_stroke,
        style_name: s.style_name.clone(),
        underline: s.underline,
        strikethrough: s.strikethrough,
        all_caps: s.all_caps,
        ..CharStyle::default()
    };
    only == *s
}

/// A value of the `EngineData` text engine document.
enum E {
    Dict(Vec<(&'static str, E)>),
    Array(Vec<E>),
    Str(String),
    Int(i64),
    Real(f64),
    Bool(bool),
}

impl E {
    fn is_container(&self) -> bool {
        match self {
            E::Dict(_) => true,
            E::Array(a) => a.iter().any(|v| matches!(v, E::Dict(_) | E::Array(_))),
            _ => false,
        }
    }

    /// Writes the value with tab indents and a key per line.
    fn write(&self, depth: usize, out: &mut Vec<u8>) {
        let tabs = |out: &mut Vec<u8>, n: usize| out.extend(std::iter::repeat_n(b'\t', n));
        match self {
            E::Dict(items) => {
                out.extend(b"<<\n");
                for (k, v) in items {
                    tabs(out, depth + 1);
                    out.push(b'/');
                    out.extend(k.as_bytes());
                    if v.is_container() {
                        out.push(b'\n');
                        tabs(out, depth + 1);
                    } else {
                        out.push(b' ');
                    }
                    v.write(depth + 1, out);
                    out.push(b'\n');
                }
                tabs(out, depth);
                out.extend(b">>");
            }
            E::Array(items) if self.is_container() => {
                out.extend(b"[\n");
                for v in items {
                    tabs(out, depth);
                    v.write(depth, out);
                    out.push(b'\n');
                }
                tabs(out, depth);
                out.push(b']');
            }
            E::Array(items) => {
                out.push(b'[');
                for v in items {
                    out.push(b' ');
                    v.write(depth, out);
                }
                out.extend(b" ]");
            }
            // UTF-16BE after a byte order mark, with `(`, `)` and `\` escaped.
            E::Str(s) => {
                out.push(b'(');
                for b in [0xFE, 0xFF].into_iter().chain(s.encode_utf16().flat_map(u16::to_be_bytes)) {
                    if matches!(b, b'(' | b')' | b'\\') {
                        out.push(b'\\');
                    }
                    out.push(b);
                }
                out.push(b')');
            }
            E::Int(i) => out.extend(i.to_string().as_bytes()),
            E::Real(r) => out.extend(real(*r).as_bytes()),
            E::Bool(b) => out.extend(if *b { &b"true"[..] } else { b"false" }),
        }
    }
}

/// A number as the text engine writes it: `12.0`, `.5`, `-.25`; at most five decimals.
fn real(r: f64) -> String {
    if !r.is_finite() {
        return "0.0".into();
    }
    let mut s = format!("{r:.5}");
    while s.ends_with('0') {
        s.pop();
    }
    if s.ends_with('.') {
        s.push('0');
    }
    if s == "-0.0" {
        s = "0.0".into();
    }
    match (s.strip_prefix("0."), s.strip_prefix("-0.")) {
        (Some(rest), _) if rest != "0" => format!(".{rest}"),
        (_, Some(rest)) => format!("-.{rest}"),
        _ => s,
    }
}

/// A descriptor value (the format's ActionDescriptor items).
enum V {
    Text(String),
    Enum(&'static str, &'static str),
    Long(i32),
    Doub(f64),
    /// A unit (`#Pnt`: points) and its value.
    UntF(&'static [u8; 4], f64),
    Objc(&'static str, Vec<(&'static str, V)>),
    Raw(Vec<u8>),
}

/// A class, key or enumeration id: a four-character code after a zero length, else its length and
/// bytes. `warp` is written as a string, as readers look it up.
fn id(out: &mut Vec<u8>, s: &str) {
    if s.len() == 4 && s != "warp" {
        out.extend(0u32.to_be_bytes());
    } else {
        out.extend((s.len() as u32).to_be_bytes());
    }
    out.extend(s.as_bytes());
}

/// A Unicode string: its UTF-16 unit count, then the units (`nul`: a terminating zero counted).
fn unicode(out: &mut Vec<u8>, s: &str, nul: bool) {
    let units: Vec<u16> = s.encode_utf16().chain(nul.then_some(0)).collect();
    out.extend((units.len() as u32).to_be_bytes());
    out.extend(units.iter().flat_map(|u| u.to_be_bytes()));
}

/// A descriptor: an empty class name, the class id, then its items.
fn descriptor(out: &mut Vec<u8>, class: &str, items: &[(&str, V)]) {
    unicode(out, "", false);
    id(out, class);
    out.extend((items.len() as u32).to_be_bytes());
    for (key, v) in items {
        id(out, key);
        match v {
            V::Text(s) => {
                out.extend(b"TEXT");
                unicode(out, s, true);
            }
            V::Enum(ty, value) => {
                out.extend(b"enum");
                id(out, ty);
                id(out, value);
            }
            V::Long(i) => {
                out.extend(b"long");
                out.extend(i.to_be_bytes());
            }
            V::Doub(d) => {
                out.extend(b"doub");
                out.extend(d.to_be_bytes());
            }
            V::UntF(unit, d) => {
                out.extend(b"UntF");
                out.extend(*unit);
                out.extend(d.to_be_bytes());
            }
            V::Objc(class, items) => {
                out.extend(b"Objc");
                descriptor(out, class, items);
            }
            V::Raw(data) => {
                out.extend(b"tdta");
                out.extend((data.len() as u32).to_be_bytes());
                out.extend(data);
            }
        }
    }
}

/// The PostScript name of the face the text is drawn with (the style's font version when that is
/// installed), as type layers name fonts; without the face installed, its family and style without
/// spaces.
fn postscript(s: &CharStyle) -> String {
    if let Some(face) = face(s) {
        return face.postscript_name();
    }
    let squeeze = |s: &str| -> String { s.chars().filter(|c| c.is_ascii_graphic() && !"[](){}<>/%".contains(*c)).collect() };
    match s.font_style.as_str() {
        "" | "Regular" => squeeze(&s.font_family),
        style => format!("{}-{}", squeeze(&s.font_family), squeeze(style)),
    }
}

/// The character attributes of `s` at `px` pixels per point, its font being `font` in the font set.
fn style_data(s: &CharStyle, font: usize, px: f64) -> E {
    let rgb = match &s.fill {
        Paint::Solid { color, .. } => color.to_rgb(),
        _ => [0.0; 3],
    };
    E::Dict(vec![
        ("Font", E::Int(font as i64)),
        ("FontSize", E::Real(s.size * px)),
        ("FauxBold", E::Bool(false)),
        ("FauxItalic", E::Bool(false)),
        ("AutoLeading", E::Bool(s.leading.is_none())),
        ("Leading", E::Real(s.leading.unwrap_or(0.0) * px)),
        ("HorizontalScale", E::Real(s.h_scale / 100.0)),
        ("VerticalScale", E::Real(s.v_scale / 100.0)),
        ("Tracking", E::Int(s.tracking.round() as i64)),
        ("AutoKerning", E::Bool(true)),
        ("Kerning", E::Int(0)),
        ("BaselineShift", E::Real(s.baseline_shift * px)),
        ("FontCaps", E::Int(if s.all_caps { 2 } else { 0 })),
        ("Underline", E::Bool(s.underline)),
        ("Strikethrough", E::Bool(s.strikethrough)),
        ("FillFlag", E::Bool(true)),
        ("FillColor", E::Dict(vec![("Type", E::Int(1)), ("Values", E::Array([1.0, rgb[0], rgb[1], rgb[2]].map(|v| E::Real(f64::from(v))).into()))])),
    ])
}

/// The paragraph attributes of alignment `j`.
fn paragraph_data(j: Justify) -> E {
    let n = match j {
        Justify::Auto | Justify::Left => 0,
        Justify::Right => 1,
        Justify::Center => 2,
        Justify::JustifyLeft => 3,
        Justify::JustifyRight => 4,
        Justify::JustifyCenter => 5,
        Justify::JustifyAll => 6,
    };
    E::Dict(vec![("Justification", E::Int(n)), ("AutoLeading", E::Real(1.2)), ("LeadingType", E::Int(0))])
}

/// The anti-aliasing method `aa` as the text descriptor's `AntA` value and the `EngineData`
/// `AntiAlias` number.
fn anti_alias(aa: TypeAntiAlias) -> (&'static str, i64) {
    match aa {
        TypeAntiAlias::None => ("Anno", 0),
        TypeAntiAlias::Crisp => ("AnCr", 1),
        TypeAntiAlias::Strong => ("AnSt", 2),
        TypeAntiAlias::Sharp => ("AnSh", 4),
    }
}

/// The `EngineData` of `t` at `px` pixels per point, drawn with anti-aliasing `aa`: the text with a
/// `\r` after each paragraph, a style run per text run and a paragraph run per paragraph (lengths in
/// UTF-16 units), point type's shape, and the fonts.
fn engine_data(t: &TextObject, px: f64, aa: TypeAntiAlias) -> Vec<u8> {
    let text = t.plain_text().replace('\n', "\r");
    let mut fonts: Vec<String> = vec![];
    let (mut styles, mut lengths) = (vec![], vec![]);
    let runs: Vec<_> = t.runs.iter().filter(|r| !r.text.is_empty()).collect();
    for (i, r) in runs.iter().enumerate() {
        let name = postscript(&r.style);
        let font = fonts.iter().position(|f| *f == name).unwrap_or_else(|| {
            fonts.push(name);
            fonts.len() - 1
        });
        styles.push(E::Dict(vec![("StyleSheet", E::Dict(vec![("StyleSheetData", style_data(&r.style, font, px))]))]));
        // The closing `\r` belongs to the last run.
        lengths.push(E::Int(r.text.encode_utf16().count() as i64 + i64::from(i + 1 == runs.len())));
    }
    let (mut paras, mut para_lengths) = (vec![], vec![]);
    for (i, p) in text.split('\r').enumerate() {
        paras.push(E::Dict(vec![(
            "ParagraphSheet",
            E::Dict(vec![("DefaultStyleSheet", E::Int(0)), ("Properties", paragraph_data(t.para_at(i).justify))]),
        )]));
        para_lengths.push(E::Int(p.encode_utf16().count() as i64 + 1));
    }
    let (writing, procession) = if t.vertical { (2, 1) } else { (0, 0) };
    let point = |x: f64, y: f64| E::Array(vec![E::Real(x), E::Real(y)]);
    let shape = E::Dict(vec![
        ("ShapeType", E::Int(0)),
        ("Procession", E::Int(procession)),
        ("Lines", E::Dict(vec![("WritingDirection", E::Int(writing)), ("Children", E::Array(vec![]))])),
        (
            "Cookie",
            E::Dict(vec![(
                "Photoshop", // brand-ok: the key's name in the files
                E::Dict(vec![
                    ("ShapeType", E::Int(0)),
                    ("PointBase", point(0.0, 0.0)),
                    (
                        "Base",
                        E::Dict(vec![
                            ("ShapeType", E::Int(0)),
                            ("TransformPoint0", point(1.0, 0.0)),
                            ("TransformPoint1", point(0.0, 1.0)),
                            ("TransformPoint2", point(0.0, 0.0)),
                        ]),
                    ),
                ]),
            )]),
        ),
    ]);
    let first = runs.first().map_or_else(|| style_data(&CharStyle::default(), 0, px), |r| style_data(&r.style, 0, px));
    let resources = || {
        E::Dict(vec![
            (
                "FontSet",
                E::Array(
                    fonts
                        .iter()
                        .map(|f| E::Dict(vec![("Name", E::Str(f.clone())), ("Script", E::Int(0)), ("FontType", E::Int(1)), ("Synthetic", E::Int(0))]))
                        .collect(),
                ),
            ),
            ("StyleSheetSet", E::Array(vec![E::Dict(vec![("Name", E::Str("Normal RGB".into())), ("StyleSheetData", E::Dict(vec![]))])])),
            (
                "ParagraphSheetSet",
                E::Array(vec![E::Dict(vec![
                    ("Name", E::Str("Normal RGB".into())),
                    ("DefaultStyleSheet", E::Int(0)),
                    ("Properties", paragraph_data(Justify::Left)),
                ])]),
            ),
            ("TheNormalStyleSheet", E::Int(0)),
            ("TheNormalParagraphSheet", E::Int(0)),
        ])
    };
    let mut normal = resources();
    // The normal style sheet holds the first run's attributes.
    if let E::Dict(items) = &mut normal
        && let Some((_, E::Array(sheets))) = items.iter_mut().find(|(k, _)| *k == "StyleSheetSet")
        && let Some(E::Dict(sheet)) = sheets.first_mut()
        && let Some((_, data)) = sheet.iter_mut().find(|(k, _)| *k == "StyleSheetData")
    {
        *data = first;
    }
    let doc = E::Dict(vec![
        (
            "EngineDict",
            E::Dict(vec![
                ("Editor", E::Dict(vec![("Text", E::Str(format!("{text}\r")))])),
                ("ParagraphRun", E::Dict(vec![("RunArray", E::Array(paras)), ("RunLengthArray", E::Array(para_lengths)), ("IsJoinable", E::Int(1))])),
                ("StyleRun", E::Dict(vec![("RunArray", E::Array(styles)), ("RunLengthArray", E::Array(lengths)), ("IsJoinable", E::Int(2))])),
                ("AntiAlias", E::Int(anti_alias(aa).1)),
                ("UseFractionalGlyphWidths", E::Bool(t.fractional_widths)),
                (
                    "Rendered",
                    E::Dict(vec![
                        ("Version", E::Int(1)),
                        ("Shapes", E::Dict(vec![("WritingDirection", E::Int(writing)), ("Children", E::Array(vec![shape]))])),
                    ]),
                ),
            ]),
        ),
        ("ResourceDict", resources()),
        ("DocumentResources", normal),
    ]);
    let mut out = b"\n\n".to_vec();
    doc.write(0, &mut out);
    out.push(b'\n');
    out
}

/// The `TySh` block of point type `t`, drawn into a canvas whose top left is `origin` (document
/// points) at `px` pixels per point with anti-aliasing `aa`; `ink` is what the layer paints, in
/// canvas pixels. The transform maps the text's own space, in pixels with its origin at the first
/// baseline's start, to the canvas.
pub(super) fn type_block(t: &TextObject, origin: Point, px: f64, ink: Rect, aa: TypeAntiAlias) -> Vec<u8> {
    let [a, b, c, d, e, f] = t.xf.as_coeffs();
    let xf = Affine::new([a, b, c, d, (e - origin.x) * px, (f - origin.y) * px]);
    // The ink in text space: the canvas rectangle taken back through the transform.
    let bounds = if xf.determinant().abs() > 1e-12 && [ink.x0, ink.y0, ink.x1, ink.y1].iter().all(|v| v.is_finite()) {
        let inv = xf.inverse();
        let pts = [(ink.x0, ink.y0), (ink.x1, ink.y0), (ink.x0, ink.y1), (ink.x1, ink.y1)].map(|(x, y)| inv * Point::new(x, y));
        let (xs, ys) = (pts.map(|p| p.x), pts.map(|p| p.y));
        let (min, max) = (|v: [f64; 4]| v.into_iter().fold(f64::INFINITY, f64::min), |v: [f64; 4]| v.into_iter().fold(f64::NEG_INFINITY, f64::max));
        [min(xs), min(ys), max(xs), max(ys)]
    } else {
        [0.0; 4]
    };
    let rect = |class: &'static str| {
        V::Objc(
            class,
            vec![
                ("Left", V::UntF(b"#Pnt", bounds[0])),
                ("Top ", V::UntF(b"#Pnt", bounds[1])),
                ("Rght", V::UntF(b"#Pnt", bounds[2])),
                ("Btom", V::UntF(b"#Pnt", bounds[3])),
            ],
        )
    };
    let orientation = if t.vertical { "Vrtc" } else { "Hrzn" };
    let mut out = 1u16.to_be_bytes().to_vec();
    for v in xf.as_coeffs() {
        out.extend(v.to_be_bytes());
    }
    // Text version 50, descriptor version 16.
    out.extend(50u16.to_be_bytes());
    out.extend(16u32.to_be_bytes());
    descriptor(
        &mut out,
        "TxLr",
        &[
            ("Txt ", V::Text(t.plain_text().replace('\n', "\r"))),
            ("textGridding", V::Enum("textGridding", "None")),
            ("Ornt", V::Enum("Ornt", orientation)),
            ("AntA", V::Enum("Annt", anti_alias(aa).0)),
            ("bounds", rect("bounds")),
            ("boundingBox", rect("boundingBox")),
            ("TextIndex", V::Long(0)),
            ("EngineData", V::Raw(engine_data(t, px, aa))),
        ],
    );
    // Warp version 1, descriptor version 16: no warp.
    out.extend(1u16.to_be_bytes());
    out.extend(16u32.to_be_bytes());
    descriptor(
        &mut out,
        "warp",
        &[
            ("warpStyle", V::Enum("warpStyle", "warpNone")),
            ("warpValue", V::Doub(0.0)),
            ("warpPerspective", V::Doub(0.0)),
            ("warpPerspectiveOther", V::Doub(0.0)),
            ("warpRotate", V::Enum("Ornt", "Hrzn")),
        ],
    );
    // The warped bounds: zeros without a warp.
    out.extend([0u8; 16]);
    out
}
