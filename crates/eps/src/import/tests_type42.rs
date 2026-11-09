//! Plain Type 42 fonts (TrueType fonts in PostScript, Adobe Technical Note #5012, "The Type 42
//! Font Format Specification") that carry their TrueType font: each code's character from its
//! glyph name, or from the glyph its `CharStrings` name gives when the name says nothing
//! (`/g12`); its width from the font's own metrics; glyphs that stand for no character drawn
//! from the font's outlines. Spacing from the show operators (PLRM 3rd ed., §5.1.4 and the
//! `ashow`, `widthshow` and `xshow` entries of chapter 8) is kept on live type as tracking. The
//! fonts are written from scratch around the bundled Source Sans 3.

use skrifa::{FontRef, MetadataProvider};
use vectorcraft_doc::{NodeKind, TextObject};

use super::tests::{all, read};
use super::tests_composite::{FONT, glyph, sfnts};

/// A program defining Type 42 font `/Plain42` whose `Encoding` gives codes 1 to `names.len()`
/// glyph names `/g1`, `/g2`… mapped through `CharStrings` to the glyph of each of `glyphs`.
fn font(glyphs: &[u32]) -> String {
    let mut enc = String::new();
    let mut strings = String::new();
    for (i, g) in glyphs.iter().enumerate() {
        enc.push_str(&format!("Encoding {} /g{} put ", i + 1, i + 1));
        strings.push_str(&format!("/g{} {g} def ", i + 1));
    }
    format!(
        "10 dict begin /FontName /Plain42 def /FontType 42 def /FontMatrix [1 0 0 1 0 0] def /FontBBox [0 0 1 1] def /PaintType 0 def\n\
         /Encoding 256 array def 0 1 255 {{ Encoding exch /.notdef put }} for {enc}\n\
         /CharStrings {} dict def CharStrings begin /.notdef 0 def {strings} end\n\
         /sfnts [\n{}] def FontName currentdict end definefont pop\n",
        glyphs.len() + 1,
        sfnts()
    )
}

fn texts(r: &crate::Imported) -> Vec<TextObject> {
    all(&r.document)
        .iter()
        .filter_map(|n| match &n.kind {
            NodeKind::Text(t) => Some((**t).clone()),
            _ => None,
        })
        .collect()
}

/// Where each glyph of `t` starts along its baseline (text space).
fn origins(t: &TextObject) -> Vec<f64> {
    vectorcraft_text::layout(vectorcraft_text::FontDb::global(), t).glyphs.iter().map(|g| g.origin.x).collect()
}

#[test]
fn glyph_names_that_say_nothing_read_the_fonts_character_map() {
    let (h, _) = glyph('H');
    let (i, _) = glyph('I');
    let r = read(&format!("{}/Plain42 findfont 40 scalefont setfont 10 20 moveto (\\001\\002) show", font(&[h, i])));
    let t = texts(&r);
    assert_eq!(t.len(), 1, "{:?}", all(&r.document));
    assert_eq!(t[0].plain_text(), "HI");
    assert_eq!(t[0].runs[0].style.font_family, "Plain42");
}

#[test]
fn a_glyph_name_says_its_character_whatever_its_code() {
    // The Euro by its name at code 128, where Latin-1 has a control code.
    // A standard encoding name says its character too (`hyphen`), not one the font's character
    // map also maps to its glyph (the soft hyphen).
    let (e, _) = glyph('€');
    let (h, _) = glyph('-');
    let program = font(&[e, h])
        .replace("Encoding 1 /g1 put", "Encoding 128 /Euro put")
        .replace("/g1 ", "/Euro ")
        .replace("Encoding 2 /g2 put", "Encoding 45 /hyphen put")
        .replace("/g2 ", "/hyphen ");
    let r = read(&format!("{program}/Plain42 findfont 40 scalefont setfont 10 20 moveto (\\200-) show"));
    assert_eq!(texts(&r).iter().map(|t| t.plain_text()).collect::<String>(), "€-");
}

/// A glyph of the bundled font with an outline that the character map doesn't reach (an
/// alternate form) and whose name says no character.
fn alternate_glyph() -> u32 {
    let f = FontRef::new(FONT).unwrap();
    let mapped: std::collections::HashSet<u32> = f.charmap().mappings().map(|(_, g)| g.to_u32()).collect();
    let names = f.glyph_names();
    (1..f.glyph_names().num_glyphs())
        .find(|g| !mapped.contains(g) && super::text::glyph_char(names.get(skrifa::GlyphId::new(*g)).unwrap().as_str()) == '\u{2022}')
        .and_then(|g| {
            // One with an outline.
            let (o, _) = super::type42::glyph(FONT, g)?;
            (!o.elements().is_empty()).then_some(g)
        })
        .unwrap()
}

#[test]
fn glyphs_that_stand_for_no_character_are_drawn_from_the_font() {
    let alt = alternate_glyph();
    let (h, _) = glyph('H');
    let r = read(&format!("{}/Plain42 findfont 40 scalefont setfont 10 20 moveto (\\001\\002) show", font(&[h, alt])));
    assert!(texts(&r).is_empty());
    let paths = all(&r.document).iter().filter(|n| matches!(n.kind, NodeKind::Path { .. })).count();
    assert_eq!(paths, 2);
}

#[test]
fn the_fonts_own_widths_space_its_type_and_move_the_point() {
    let (h, wh) = glyph('H');
    let (i, wi) = glyph('I');
    let f = font(&[h, i]);
    let check = |cond: String| super::tests::check(&format!("{f} /Plain42 findfont 50 scalefont setfont {cond}"));
    check(format!("(\\001\\002) stringwidth pop {} sub abs 0.01 lt", 50.0 * (wh + wi)));
    check(format!("gsave nulldevice 10 20 moveto (\\001\\002) show currentpoint grestore pop {} sub abs 0.01 lt", 10.0 + 50.0 * (wh + wi)));
    check("gsave nulldevice 10 20 moveto (\\001\\002) [30 7] xshow currentpoint grestore pop 47 sub abs 0.01 lt".into());
    check("newpath 10 20 moveto (\\001) true charpath pathbbox exch pop sub abs 25 gt exch 10 sub abs 8 lt and".into());
}

/// `xshow` and `ashow` put each character where they say: the type's tracking makes up the
/// difference from its font's advances.
#[test]
fn show_operators_space_live_type_with_tracking() {
    let (h, wh) = glyph('H');
    let (i, _) = glyph('I');
    let f = font(&[h, i, h]);
    let r = read(&format!("{f}/Plain42 findfont 50 scalefont setfont 10 20 moveto (\\001\\002\\003) [40 25 10] xshow"));
    let t = texts(&r);
    assert_eq!(t.len(), 1);
    assert_eq!(t[0].plain_text(), "HIH");
    let o = origins(&t[0]);
    assert!((o[1] - o[0] - 40.0).abs() < 0.05 && (o[2] - o[1] - 25.0).abs() < 0.05, "{o:?}");
    let r = read(&format!("{f}/Plain42 findfont 50 scalefont setfont 10 20 moveto 6 0 (\\001\\002) ashow"));
    let o = origins(&texts(&r)[0]);
    assert!((o[1] - o[0] - (50.0 * wh + 6.0)).abs() < 0.05, "{o:?}");
}

/// A font found by name (its program isn't in the file) keeps the installed font's spacing,
/// plus what the show operator adds.
#[test]
fn named_fonts_take_the_show_operators_spacing() {
    let r = read("/Helvetica findfont 20 scalefont setfont 10 20 moveto (AB) [30 0] xshow");
    let t = texts(&r);
    assert_eq!(t[0].plain_text(), "AB");
    let o = origins(&t[0]);
    assert!((o[1] - o[0] - 30.0).abs() < 0.05, "{o:?}");
    // Plain `show` keeps a single run with no tracking.
    let r = read("/Helvetica findfont 20 scalefont setfont 10 20 moveto (AB) show");
    let t = texts(&r);
    assert!(t[0].runs.len() == 1 && t[0].runs[0].style.tracking == 0.0, "{:?}", t[0].runs);
}

/// `kshow` runs its procedure between each two characters with their codes (PLRM 3rd ed.,
/// `kshow`): where it moves the current point, the next character follows (kept on live type as
/// tracking), and what it draws is drawn over the type.
#[test]
fn kshow_procedures_kern_live_type() {
    let (h, wh) = glyph('H');
    let (i, wi) = glyph('I');
    let f = font(&[h, i, h]);
    // Only the pair of codes 1 then 2 is kerned, 20 units.
    let kern = "{ exch 1 eq exch 2 eq and { 20 0 rmoveto } if }";
    let r = read(&format!("{f}/Plain42 findfont 50 scalefont setfont 10 20 moveto {kern} (\\001\\002\\003) kshow"));
    let t = texts(&r);
    assert_eq!(t.len(), 1, "{:?}", all(&r.document));
    assert_eq!(t[0].plain_text(), "HIH");
    let o = origins(&t[0]);
    assert!((o[1] - o[0] - (50.0 * wh + 20.0)).abs() < 0.05, "{o:?}");
    assert!((o[2] - o[1] - 50.0 * wi).abs() < 0.05, "{o:?}");
    let check = |cond: String| super::tests::check(&format!("{f} /Plain42 findfont 50 scalefont setfont {cond}"));
    check(format!(
        "gsave nulldevice 10 20 moveto {kern} (\\001\\002\\003) kshow currentpoint grestore pop {} sub abs 0.01 lt",
        10.0 + 50.0 * (2.0 * wh + wi) + 20.0
    ));
    // A font found by name kerns the same way.
    let r = read("/Helvetica findfont 20 scalefont setfont 10 20 moveto { pop pop -3 0 rmoveto } (HIH) kshow");
    let t = texts(&r);
    let plain = read("/Helvetica findfont 20 scalefont setfont 10 20 moveto (HIH) show");
    let (o, p) = (origins(&t[0]), origins(&texts(&plain)[0]));
    assert!((o[1] - o[0] - (p[1] - p[0] - 3.0)).abs() < 0.05 && (o[2] - o[1] - (p[2] - p[1] - 3.0)).abs() < 0.05, "{o:?} {p:?}");
    // A procedure that draws: a square after the type, over it.
    let r = read("/Helvetica findfont 20 scalefont setfont 10 20 moveto { pop pop currentpoint 2 2 rectfill } (AB) kshow");
    let kinds: Vec<bool> = all(&r.document)
        .iter()
        .filter(|n| matches!(n.kind, NodeKind::Text(_) | NodeKind::Path { .. }))
        .map(|n| matches!(n.kind, NodeKind::Text(_)))
        .collect();
    assert_eq!(kinds, [true, false]);
}

/// Characters the installed font would join in a ligature (`fi`) keep the places the file gives
/// each of them: the type is set without ligatures and each character's tracking is its own.
#[test]
fn characters_a_ligature_would_join_keep_their_places() {
    // Each character at its own width in the bundled Source Sans 3, as a PDF converter writes them.
    let (_, wf) = glyph('f');
    let (_, wi) = glyph('i');
    let (f, i) = (50.0 * wf, 50.0 * wi);
    let r = read(&format!("/SourceSans3-Regular findfont 50 scalefont setfont 10 20 moveto (fit) [{f} {i} 0] xshow"));
    let t = texts(&r);
    assert_eq!(t[0].plain_text(), "fit");
    let o = origins(&t[0]);
    assert_eq!(o.len(), 3, "{o:?}");
    assert!((o[1] - o[0] - f).abs() < 0.05 && (o[2] - o[1] - i).abs() < 0.05, "{o:?} {f} {i}");
}

/// A move that is finite in user space but not along the type's baseline (a font matrix
/// squashed a thousandfold) is an `undefinedresult`, not a tracking that isn't a number.
#[test]
fn moves_too_large_for_the_baseline_are_refused() {
    let r = read("0 0 10 10 rectfill /Helvetica findfont [0.001 0 0 50 0 0] makefont setfont 10 20 moveto (A\\001) [1e307 -1e307] xshow");
    assert!(texts(&r).iter().all(|t| t.runs.iter().all(|r| r.style.tracking.is_finite())), "{:?}", texts(&r));
    assert!(r.warnings.iter().any(|w| w.contains("undefinedresult")), "{:?}", r.warnings);
}

/// Moves that overflow (two `1e308` numbers in `xshow`) in type drawn from the font's outlines
/// are an `undefinedresult` before anything is painted, as for live type: no glyph lands at
/// infinity.
#[test]
fn outline_moves_that_overflow_are_refused() {
    let alt = alternate_glyph();
    let (h, _) = glyph('H');
    let r = read(&format!(
        "0 0 10 10 rectfill {}/Plain42 findfont 40 scalefont setfont 10 20 moveto (\\001\\002\\001) [1e308 1e308 0] xshow",
        font(&[h, alt])
    ));
    assert!(r.warnings.iter().any(|w| w.contains("undefinedresult")), "{:?}", r.warnings);
    assert_eq!(all(&r.document).iter().filter(|n| matches!(n.kind, NodeKind::Path { .. })).count(), 1, "{:?}", all(&r.document));
}

/// Reading a font's character map counts the mappings it visits, once, for the operation
/// budget.
#[test]
fn reading_the_character_map_counts_its_mappings() {
    let f = super::type42::Font::new(FONT.to_vec());
    let (h, _) = glyph('H');
    assert_eq!(f.char_of(h), Some('H'));
    let n = f.take_char_map_work();
    assert!(n > 100, "{n}");
    f.char_of(h);
    assert_eq!(f.take_char_map_work(), 0);
}

/// A glyph named for a symbol font's private-use code (`uniF041`) stands for no character, as
/// when the character map maps only that code to it, so it's drawn from the font's outline;
/// other names still say their character.
#[test]
fn private_use_glyph_names_stand_for_no_character() {
    use super::type42::post_name_char;
    assert_eq!(post_name_char("uniF041"), None);
    assert_eq!(post_name_char("uF0FF"), None);
    assert_eq!(post_name_char("uni00AD"), None);
    assert_eq!(post_name_char("uniF100"), Some('\u{F100}'));
    assert_eq!(post_name_char("uni20AC"), Some('€'));
    assert_eq!(post_name_char("A"), Some('A'));
    assert_eq!(post_name_char("bullet"), Some('\u{2022}'));
    assert_eq!(post_name_char("g12"), None);
}

/// A glyph the character map doesn't reach is looked up by its glyph name once: the first lookup
/// counts for the operation budget, a second one is remembered (a miss too).
#[test]
fn glyph_name_lookups_are_counted_once() {
    let f = super::type42::Font::new(FONT.to_vec());
    let alt = alternate_glyph();
    let first = f.char_of(alt);
    f.take_char_map_work();
    // The character map was read by the first call; the name lookup is remembered.
    let g = FontRef::new(FONT).unwrap().glyph_names().num_glyphs() + 5;
    assert_eq!(f.char_of(g), None);
    assert!(f.take_char_map_work() > 0);
    assert_eq!(f.char_of(g), None);
    assert_eq!(f.char_of(alt), first);
    assert_eq!(f.take_char_map_work(), 0);
}

/// Control codes (a new string's zeros) set no type but are still read: showing or measuring a
/// long string of them in a loop runs out of the operation budget.
#[test]
fn control_codes_in_a_loop_run_out_of_the_budget() {
    for body in ["{ s stringwidth pop pop } loop", "{ 0 0 moveto s show } loop"] {
        super::tests::runs_out_of_budget(&format!("/Helvetica findfont 10 scalefont setfont /s 1000000 string def {body}"));
    }
}
