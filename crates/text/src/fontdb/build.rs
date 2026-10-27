use super::face::{FontBytes, FontFace, Variation};
use super::names::{name, norm};
use super::traits::FaceStyle;
use super::*;

/// `face` at the axis settings `variations` (user units), as face `id`: its names, kind, italic
/// and file, and its weight the `wght` setting when there is one.
pub(super) fn instanced(face: &FontFace, variations: Vec<Variation>, id: u32) -> Option<FontFace> {
    let weight = variations.iter().find(|(t, _)| t == b"wght").map_or(face.weight, |(_, v)| *v);
    let spec = FaceStyle {
        family: face.family.clone(),
        native_family: face.native_family.clone(),
        style: face.style.clone(),
        keys: face.keys.clone(),
        variations,
        weight: Some(weight),
        italic: face.italic,
        traits: face.traits,
    };
    let mut f = make_face(face.bytes.clone(), face.index, spec, face.path.clone())?;
    f.id = id;
    Some(f)
}

pub(super) fn make_face(bytes: FontBytes, index: u32, spec: FaceStyle, path: Option<std::path::PathBuf>) -> Option<FontFace> {
    let data: &[u8] = match &bytes {
        FontBytes::Static(b) => b,
        FontBytes::Owned(v) => v.as_slice(),
    };
    let f = skrifa::FontRef::from_index(data, index).ok()?;
    let FaceStyle { family, native_family, style, keys, variations, weight, italic, traits } = spec;
    let version = name(&f, &[StringId::VERSION_STRING]).unwrap_or_default();
    // A named instance: outlines, metrics and advances at its axis settings.
    let location = if variations.is_empty() { Location::default() } else { f.axes().location(variations.iter().map(|(t, v)| (Tag::new(t), *v))) };
    let m = f.metrics(Size::unscaled(), &location);
    let a = f.attributes();
    let hb = harfrust::FontRef::from_index(data, index).ok()?;
    let shaper = harfrust::ShaperData::new(&hb);
    let instance =
        (!variations.is_empty()).then(|| harfrust::ShaperInstance::from_variations(&hb, variations.iter().map(|(t, v)| (harfrust::Tag::new(t), *v))));
    let upem = m.units_per_em.max(1) as f64;
    let x_height = m.x_height.map(|v| v as f64).filter(|v| *v > 0.0).unwrap_or(m.ascent as f64 * 0.5);
    let decoration = |d: Option<skrifa::metrics::Decoration>, fallback: (f64, f64)| {
        d.map(|d| (f64::from(d.offset), f64::from(d.thickness))).filter(|(o, t)| o.is_finite() && *t > 0.0 && t.is_finite()).unwrap_or(fallback)
    };
    let underline = decoration(m.underline, (-0.075 * upem, 0.05 * upem));
    let strikeout = decoration(m.strikeout, (x_height * 0.5 + 0.025 * upem, 0.05 * upem));
    let colour_bitmaps = crate::bitmap::has_colour_bitmaps(&f);
    let colour_layers = crate::colr::has_colour_layers(&f);
    let svg_glyphs = crate::svg_glyph::has_svg_glyphs(&f);
    Some(FontFace {
        keys,
        traits,
        id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
        family,
        native_family,
        style,
        version,
        weight: weight.unwrap_or(a.weight.value()),
        italic: italic || !matches!(a.style, skrifa::attribute::Style::Normal),
        variations,
        location,
        instance,
        upem,
        ascent: m.ascent as f64,
        descent: -(m.descent as f64),
        cap_height: m.cap_height.map(|v| v as f64).filter(|v| *v > 0.0).unwrap_or(m.ascent as f64 * 0.72),
        x_height,
        underline,
        strikeout,
        shaper,
        bytes,
        index,
        path,
        ideographic_centre: std::sync::OnceLock::new(),
        icf_margins: std::sync::OnceLock::new(),
        colour_bitmaps,
        colour_layers,
        svg_glyphs,
    })
}

/// An axis of a BASE table: its default baseline values for Han ideographs (`hani`), else kana
/// (`kana`), else its first script, by tag, in font units.
pub(super) fn base_values(axis: skrifa::raw::tables::base::Axis<'_>) -> Option<HashMap<[u8; 4], f64>> {
    let tags = axis.base_tag_list()?.ok()?;
    let list = axis.base_script_list().ok()?;
    let records = list.base_script_records();
    let record =
        [b"hani", b"kana"].into_iter().find_map(|t| records.iter().find(|r| r.base_script_tag() == Tag::new(t))).or_else(|| records.first())?;
    let values = record.base_script(list.offset_data()).ok()?.base_values()?.ok()?;
    let mut out = HashMap::new();
    for (tag, coord) in tags.baseline_tags().iter().zip(values.base_coords().iter()) {
        if let Ok(c) = coord {
            out.insert(tag.get().to_be_bytes(), f64::from(c.coordinate()));
        }
    }
    Some(out)
}

/// Weight implied by a style name (400 = regular). Japanese faces number their weights W0–W9
/// (Hiragino, Morisawa…), W6 weighing 600 as its weight class does (#2873).
pub fn style_weight(style: &str) -> f32 {
    let numbered = style.split(|c: char| !c.is_ascii_alphanumeric()).find_map(|part| match part.as_bytes() {
        [b'W' | b'w', d @ b'0'..=b'9'] => Some(f32::from(d - b'0') * 100.0),
        _ => None,
    });
    if let Some(w) = numbered {
        return w.max(100.0);
    }
    let s = norm(style);
    const TABLE: &[(&str, f32)] = &[
        ("extralight", 200.0),
        ("ultralight", 200.0),
        ("semibold", 600.0),
        ("demibold", 600.0),
        ("extrabold", 800.0),
        ("ultrabold", 800.0),
        ("hairline", 100.0),
        ("thin", 100.0),
        ("light", 300.0),
        ("medium", 500.0),
        ("bold", 700.0),
        ("black", 900.0),
        ("heavy", 900.0),
    ];
    TABLE.iter().find(|(k, _)| s.contains(k)).map(|(_, w)| *w).unwrap_or(400.0)
}

pub(super) fn style_italic(style: &str) -> bool {
    let s = norm(style);
    s.contains("italic") || s.contains("oblique") || s == "it"
}

/// How far `face` is from the style asked for: its weight difference, and a penalty for the wrong
/// slant. The penalty is less than the weight gap between Regular and Bold (and Bold and Black),
/// so Bold Italic asked of a family with Bold and Italic faces only gets the Bold (#2491).
pub(super) fn style_distance(face: &FontFace, weight: f32, italic: bool) -> f32 {
    (face.weight - weight).abs() + if face.italic != italic { 250.0 } else { 0.0 }
}
