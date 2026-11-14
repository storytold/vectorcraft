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

/// `widthshow` adds its spacing to the composite character whose font number times 256 plus code
/// is its `char` (PLRM 3rd ed., `widthshow` in chapter 8), not to every character with that code.
#[test]
fn widthshow_matches_a_composite_character_by_font_number_and_code() {
    let (_, wh) = glyph('H');
    let fonts = fonts().replace("/Encoding [0] def", "/Encoding [0 0] def");
    let check = |cond: String| super::tests::check(&format!("{fonts} /Comp findfont 50 scalefont setfont {cond}"));
    let end = |ch: u32| format!("gsave nulldevice 10 20 moveto 20 0 {ch} (\\000\\001\\001\\001) widthshow currentpoint grestore pop");
    // 257 is font 1's code 1: the second H only.
    check(format!("{} {} sub abs 0.01 lt", end(257), 10.0 + 100.0 * wh + 20.0));
    // 1 is font 0's code 1: the first H only.
    check(format!("{} {} sub abs 0.01 lt", end(1), 10.0 + 100.0 * wh + 20.0));
    // 2 is neither.
    check(format!("{} {} sub abs 0.01 lt", end(2), 10.0 + 100.0 * wh));
}

/// `kshow` runs its procedure between each two characters of a composite font, with the values of
/// their codes (font number times 256 plus code), and the next character follows where it moves
/// the current point (PLRM 3rd ed., `kshow`).
#[test]
fn kshow_runs_between_each_two_composite_characters() {
    let (_, wh) = glyph('H');
    let (_, wi) = glyph('I');
    let fonts = fonts().replace("/Encoding [0] def", "/Encoding [0 0] def");
    let check = |cond: String| super::tests::check(&format!("{fonts} /Comp findfont 50 scalefont setfont {cond}"));
    check("gsave nulldevice 10 20 moveto [ {} (\\000\\001\\001\\002\\000\\001) kshow ] grestore aload length 4 eq exch 1 eq and exch 258 eq and exch 258 eq and exch 1 eq and".into());
    check(format!(
        "gsave nulldevice 10 20 moveto {{pop pop 100 0 rmoveto}} (\\000\\001\\000\\002) kshow currentpoint grestore pop {} sub abs 0.01 lt",
        10.0 + 50.0 * (wh + wi) + 100.0
    ));
    // Drawn, the I starts where the procedure moved the point.
    let r = read(&format!("{fonts}/Comp findfont 50 scalefont setfont 10 20 moveto {{pop pop 100 0 rmoveto}} (\\000\\001\\000\\002) kshow"));
    let paths: Vec<_> = all(&r.document).iter().filter(|n| matches!(n.kind, NodeKind::Path { .. })).map(|n| bounds(n)).collect();
    assert_eq!(paths.len(), 2, "{paths:?}");
    assert!(paths[1].x0 > 10.0 + 50.0 * wh + 100.0 - 1.0, "{paths:?}");
    // A procedure that draws: its square goes after the glyphs' group, outside it.
    let r = read(&format!(
        "{fonts}/Comp findfont 50 scalefont setfont 10 20 moveto {{pop pop currentpoint 2 2 rectfill}} (\\000\\001\\000\\002\\000\\001) kshow"
    ));
    let top = super::tests::objects(&r.document);
    assert_eq!(top.len(), 3, "{top:?}");
    assert_eq!(top[0].children().map(Vec::len), Some(3));
    assert!(top[1..].iter().all(|n| matches!(n.kind, NodeKind::Path { .. }) && (bounds(n).width() - 2.0).abs() < 0.01), "{top:?}");
}

/// Measuring a composite font's Type 42 glyphs (`stringwidth` on a long string in a `loop`)
/// counts against the operation budget, as drawing them does.
#[test]
fn measuring_type42_descendants_in_a_loop_runs_out_of_the_budget() {
    super::tests::runs_out_of_budget(&format!(
        "{} /Comp findfont 50 scalefont setfont /s 100000 string def {{ s stringwidth pop pop }} loop",
        fonts()
    ));
}

/// A descendant whose TrueType data can't be read (an `sfnts` element that isn't a string, after
/// 32 MB of strings) is read once, not again for every character measured: copying its strings
/// costs about 130,000 operations, so 1,000 measures finish only if it is copied once (reading
/// it each time would need 130 million, over the budget of 50 million).
#[test]
fn unreadable_type42_data_is_read_once() {
    super::tests::check(
        "10 dict begin /FontName /Bad def /FontType 42 def /FontMatrix [1 0 0 1 0 0] def /FontBBox [0 0 1 1] def /PaintType 0 def\n\
         /Encoding 256 array def 0 1 255 { Encoding exch /.notdef put } for\n\
         /CharStrings 1 dict def CharStrings /.notdef 0 put /sfnts [ 0 1 7 { pop 4000000 string } for 0 ] def FontName currentdict end definefont pop\n\
         16 dict begin /FontName /Comp def /FontType 0 def /FontMatrix [1 0 0 1 0 0] def /FMapType 2 def\n\
         /Encoding [0] def /FDepVector [/Bad findfont] def FontName currentdict end definefont pop\n\
         /Comp findfont 50 scalefont setfont 1000 { (\\000\\001) stringwidth pop pop } repeat true",
    );
}

/// A Type 3 descendant draws and measures its glyphs with its own procedure, under the composite
/// font's matrix: here 50 pt and twice as wide, so a glyph 500 × 700 units wide (600 units
/// apart) is 50 × 35 pt, 60 pt apart (PLRM 3rd ed., §5.10: a descendant's glyphs are under the
/// composite font's matrix as well as its own).
#[test]
fn type3_descendants_draw_and_measure_under_the_composite_matrix() {
    let fonts = "8 dict begin /FontName /T3 def /FontType 3 def /FontMatrix [0.001 0 0 0.001 0 0] def /FontBBox [0 0 1000 1000] def\n\
                 /Encoding 256 array def 0 1 255 { Encoding exch /.notdef put } for Encoding 1 /box put\n\
                 /BuildChar { exch begin 1 eq { 600 0 0 0 500 700 setcachedevice 0 0 500 700 rectfill } { 0 0 setcharwidth } ifelse end } def\n\
                 FontName currentdict end definefont pop\n\
                 16 dict begin /FontName /Comp def /FontType 0 def /FontMatrix [2 0 0 1 0 0] def /FMapType 2 def\n\
                 /Encoding [0] def /FDepVector [/T3 findfont] def FontName currentdict end definefont pop\n\
                 /Comp findfont 50 scalefont setfont";
    let r = read(&format!("{fonts} 0 0 1 setrgbcolor 10 20 moveto (\\000\\001\\000\\001) show"));
    let paths: Vec<_> = all(&r.document).into_iter().filter(|n| matches!(n.kind, NodeKind::Path { .. })).collect();
    assert_eq!(paths.len(), 2, "{:?}", r.warnings);
    let (a, b) = (bounds(&paths[0]), bounds(&paths[1]));
    for g in [a, b] {
        assert!((g.width() - 50.0).abs() < 0.01 && (g.height() - 35.0).abs() < 0.01, "{g:?}");
    }
    assert!((a.x0 - 10.0).abs() < 0.01 && (b.x0 - 70.0).abs() < 0.01, "{a:?} {b:?}");
    let check = |cond: &str| super::tests::check(&format!("{fonts} {cond}"));
    check("(\\000\\001\\000\\001) stringwidth pop 120 sub abs 0.01 lt");
    check("newpath 10 20 moveto (\\000\\001) true charpath currentpoint pop 70 sub abs 0.01 lt");
}

/// A descendant found by name is scaled for each character it sets: copying a huge font
/// dictionary for every character counts against the operation budget, so a `loop` of it stops.
#[test]
fn a_huge_named_descendant_in_a_loop_runs_out_of_the_budget() {
    super::tests::runs_out_of_budget(
        "200000 dict begin 0 1 199999 { dup def } for /FontName /Big def /FontType 1 def /FontMatrix [0.001 0 0 0.001 0 0] def\n\
         FontName currentdict end definefont pop\n\
         16 dict begin /FontName /Comp def /FontType 0 def /FontMatrix [1 0 0 1 0 0] def /FMapType 2 def\n\
         /Encoding [0] def /FDepVector [/Big findfont] def FontName currentdict end definefont pop\n\
         /Comp findfont 50 scalefont setfont { (\\000A) stringwidth pop pop } loop",
    );
}

/// A composite font's TrueType data stays read while it is in use: the font a string uses for
/// every other character isn't dropped (and read again, counting its size against the memory
/// budget each time) while 16 other fonts are read between its uses.
#[test]
fn type42_data_in_use_stays_read() {
    let mut others = String::new();
    for k in 1..=16 {
        others.push_str(&format!(
            "10 dict begin /FontName /B{k} def /FontType 42 def /FontMatrix [1 0 0 1 0 0] def /FontBBox [0 0 1 1] def /PaintType 0 def\n\
             /Encoding 256 array def 0 1 255 {{ Encoding exch /.notdef put }} for\n\
             /CharStrings 1 dict def CharStrings /.notdef 0 put /sfnts [<00>] def FontName currentdict end definefont pop\n"
        ));
    }
    let deps: String = (1..=16).map(|k| format!(" /B{k} findfont")).collect();
    let fonts = fonts().replace(
        "/Encoding [0] def /FDepVector [/T42 findfont] def",
        &format!("/Encoding [0 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16] def /FDepVector [/T42 findfont{deps}] def"),
    );
    let s: String = (1..=16).map(|k| format!("\\000\\001\\{k:03o}\\001")).collect();
    super::tests::check(&format!("{others}{fonts} /Comp findfont 50 scalefont setfont 3000 {{ ({s}) stringwidth pop pop }} repeat true"));
}

/// A page that cycles through more Type 42 fonts than are kept read reads each again when it
/// comes back, but counts its data against the memory budget once.
#[test]
fn type42_data_read_again_counts_once() {
    let mut fonts = format!("/S [\n{}] def\n", sfnts());
    for k in 0..17 {
        fonts.push_str(&format!(
            "10 dict begin /FontName /F{k} def /FontType 42 def /FontMatrix [1 0 0 1 0 0] def /FontBBox [0 0 1 1] def /PaintType 0 def\n\
             /Encoding 256 array def 0 1 255 {{ Encoding exch /.notdef put }} for\n\
             /CharStrings 1 dict def CharStrings /.notdef 0 put /sfnts [ S aload pop ] def FontName currentdict end definefont pop\n"
        ));
    }
    let deps: String = (0..17).map(|k| format!(" /F{k} findfont")).collect();
    let picks: String = (0..17).map(|k| format!(" {k}")).collect();
    fonts.push_str(&format!(
        "16 dict begin /FontName /Comp def /FontType 0 def /FontMatrix [1 0 0 1 0 0] def /FMapType 2 def\n\
         /Encoding [{picks}] def /FDepVector [{deps}] def FontName currentdict end definefont pop\n"
    ));
    let s: String = (0..17).map(|k| format!("\\{k:03o}\\001")).collect();
    // 17 fonts of 431 kB, each read 200 times: 1.4 GB if each read counted.
    super::tests::check(&format!("{fonts} /Comp findfont 50 scalefont setfont 200 {{ ({s}) stringwidth pop pop }} repeat true"));
}

/// Reading `sfnts` data that turns out unreadable is charged too: a composite font cycling
/// through more such descendants than are kept read copies their strings on every glyph, and a
/// `loop` of it runs out of the budget.
#[test]
fn unreadable_type42_data_read_again_runs_out_of_the_budget() {
    let mut fonts = String::from("/S [ 0 1 7 { pop 4000000 string } for ] def\n");
    for k in 0..17 {
        fonts.push_str(&format!(
            "10 dict begin /FontName /Bad{k} def /FontType 42 def /FontMatrix [1 0 0 1 0 0] def /FontBBox [0 0 1 1] def /PaintType 0 def\n\
             /Encoding 256 array def 0 1 255 {{ Encoding exch /.notdef put }} for\n\
             /CharStrings 1 dict def CharStrings /.notdef 0 put /sfnts [ S aload pop 0 ] def FontName currentdict end definefont pop\n"
        ));
    }
    let deps: String = (0..17).map(|k| format!(" /Bad{k} findfont")).collect();
    let picks: String = (0..17).map(|k| format!(" {k}")).collect();
    let s: String = (0..17).map(|k| format!("\\{k:03o}\\001")).collect();
    super::tests::runs_out_of_budget(&format!(
        "{fonts}16 dict begin /FontName /Comp def /FontType 0 def /FontMatrix [1 0 0 1 0 0] def /FMapType 2 def\n\
         /Encoding [{picks}] def /FDepVector [{deps}] def FontName currentdict end definefont pop\n\
         /Comp findfont 50 scalefont setfont {{ ({s}) stringwidth pop pop }} loop"
    ));
}

/// Characters that map to no glyph (an empty `FDepVector`) cost nothing to draw, but finding that
/// out reads the whole string: a `loop` showing, measuring or outlining a long string of them runs
/// out of the operation budget instead of stalling the import.
#[test]
fn unmapped_composite_characters_in_a_loop_run_out_of_the_budget() {
    let font = "16 dict begin /FontName /Comp def /FontType 0 def /FontMatrix [1 0 0 1 0 0] def /FMapType 2 def\n\
                /Encoding [0] def /FDepVector [] def FontName currentdict end definefont pop\n\
                /Comp findfont 50 scalefont setfont /s 100000 string def";
    for body in ["{ 0 0 moveto s show } loop", "{ s stringwidth pop pop } loop", "{ newpath 0 0 moveto s false charpath } loop"] {
        super::tests::runs_out_of_budget(&format!("{font} {body}"));
    }
}
