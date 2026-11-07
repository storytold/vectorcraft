//! Composite (Type 0) fonts with the 8/8 mapping, as PDF-to-PostScript converters write a PDF's
//! two-byte fonts: each character a byte picking the descendant font and a byte for the code in
//! it (PLRM 3rd ed., §5.10). Type 42 descendants draw their glyphs from the TrueType font they
//! carry (Adobe Technical Note #5012); the fonts here are written from scratch around the bundled
//! Source Sans 3.

use skrifa::instance::{LocationRef, Size};
use skrifa::{FontRef, MetadataProvider};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::NodeKind;

use super::tests::{all, bounds, fill, read};

pub(super) const FONT: &[u8] = include_bytes!("../../../../assets/fonts/SourceSans3-Regular.ttf");

/// The glyph index of `c` in [`FONT`] and its advance width in ems.
pub(super) fn glyph(c: char) -> (u32, f64) {
    let font = FontRef::new(FONT).unwrap();
    let gid = font.charmap().map(c).unwrap();
    let upem = f64::from(font.metrics(Size::unscaled(), LocationRef::default()).units_per_em);
    let adv = font.glyph_metrics(Size::unscaled(), LocationRef::default()).advance_width(gid).unwrap();
    (gid.to_u32(), f64::from(adv) / upem)
}

/// `sfnts` strings of [`FONT`]: the first of odd length, ending in a padding byte that isn't part
/// of the font, the rest split at even lengths.
pub(super) fn sfnts() -> String {
    let hex = |b: &[u8]| b.iter().map(|x| format!("{x:02x}")).collect::<String>();
    let mut out = format!("<{}00>\n", hex(&FONT[..20000]));
    for chunk in FONT[20000..].chunks(40000) {
        out.push_str(&format!("<{}>\n", hex(chunk)));
    }
    out
}

/// A program defining Type 42 font `/T42` (codes 1 and 2 are H and I, by glyph names that
/// don't say which characters they are) and composite font `/Comp` over it.
fn fonts() -> String {
    let (h, _) = glyph('H');
    let (i, _) = glyph('I');
    format!(
        "10 dict begin /FontName /T42 def /FontType 42 def /FontMatrix [1 0 0 1 0 0] def /FontBBox [0 0 1 1] def /PaintType 0 def\n\
         /Encoding 256 array def 0 1 255 {{ Encoding exch /.notdef put }} for Encoding 1 /c01 put Encoding 2 /c02 put\n\
         /CharStrings 3 dict def CharStrings begin /.notdef 0 def /c01 {h} def /c02 {i} def end\n\
         /sfnts [\n{}] def FontName currentdict end definefont pop\n\
         16 dict begin /FontName /Comp def /FontType 0 def /FontMatrix [1 0 0 1 0 0] def /FMapType 2 def\n\
         /Encoding [0] def /FDepVector [/T42 findfont] def FontName currentdict end definefont pop\n",
        sfnts()
    )
}

#[test]
fn type42_descendants_draw_their_own_glyphs() {
    let (_, wh) = glyph('H');
    let (_, wi) = glyph('I');
    let r = read(&format!("{}/Comp findfont 50 scalefont setfont 0 0 1 setrgbcolor 10 20 moveto (\\000\\001\\000\\002) show", fonts()));
    assert!(r.warnings.is_empty(), "{:?}", r.warnings);
    let nodes = all(&r.document);
    assert!(!nodes.iter().any(|n| matches!(n.kind, NodeKind::Text(_))));
    let paths: Vec<_> = nodes.iter().filter(|n| matches!(n.kind, NodeKind::Path { .. })).collect();
    assert_eq!(paths.len(), 2);
    for p in &paths {
        assert_eq!(fill(p), Some(&Paint::solid(Color::rgb(0.0, 0.0, 1.0))));
    }
    // The H, then the I one H's width along; both stand on the baseline, a cap height tall.
    let (bh, bi) = (bounds(paths[0]), bounds(paths[1]));
    assert!((bh.x0 - 10.0).abs() < 8.0 && bh.width() > 20.0 && bh.width() < 50.0 * wh, "{bh:?}");
    assert!((bh.height() - bi.height()).abs() < 0.5 && bh.height() > 25.0 && bh.height() < 40.0, "{bh:?} {bi:?}");
    assert!(bi.x0 > 10.0 + 50.0 * wh - 1.0 && bi.x1 < 10.0 + 50.0 * (wh + wi), "{bi:?}");
}

#[test]
fn type42_descendants_measure_space_and_outline_their_glyphs() {
    let (_, wh) = glyph('H');
    let (_, wi) = glyph('I');
    let check = |cond: String| super::tests::check(&format!("{} /Comp findfont 50 scalefont setfont {cond}", fonts()));
    // The width its font gives each glyph.
    check(format!("(\\000\\001\\000\\002) stringwidth pop {} sub abs 0.01 lt", 50.0 * (wh + wi)));
    // xyshow places each character where its numbers say.
    check("gsave nulldevice 10 20 moveto (\\000\\001\\000\\002) [30 0 7 0] xyshow currentpoint grestore pop 47 sub abs 0.01 lt".into());
    // charpath outlines them.
    check("newpath 10 20 moveto (\\000\\001) true charpath pathbbox exch pop sub abs 25 gt exch 10 sub abs 8 lt and".into());
}

/// A descendant that is a font by name only (its program isn't run) sets its characters as type,
/// at the size the composite font's matrix gives them, not at 1000 times it.
#[test]
fn named_descendants_set_type_at_the_composite_fonts_size() {
    let r = read(
        "16 dict begin /FontName /Comp def /FontType 0 def /FontMatrix [1 0 0 1 0 0] def /FMapType 2 def\n\
         /Encoding [0] def /FDepVector [/Helvetica findfont] def FontName currentdict end definefont pop\n\
         /Comp findfont 30 scalefont setfont 10 20 moveto (\\000A\\000B) show",
    );
    let sizes: Vec<f64> = all(&r.document)
        .iter()
        .filter_map(|n| match &n.kind {
            NodeKind::Text(t) => Some(t.runs.iter().map(|r| r.style.size * t.xf.determinant().abs().sqrt()).sum::<f64>() / t.runs.len() as f64),
            _ => None,
        })
        .collect();
    assert!(!sizes.is_empty() && sizes.iter().all(|s| (s - 30.0).abs() < 0.5), "{sizes:?}");
    let bounds_all = all(&r.document).iter().map(|n| bounds(n)).reduce(|a, b| a.union(b)).unwrap();
    assert!(bounds_all.width() < 60.0, "{bounds_all:?}");
}
