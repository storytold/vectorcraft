//! Preferences › Type › Highlight Alternate Glyphs (#394): OpenType substitutions get a pink
//! cell behind them on screen when the preference is on. Preferences › Type › Highlight
//! Substituted Fonts does the same for text drawn in a substitute for a missing font.

use crate::*;

/// Highlight Substituted Fonts follows the preference passed in [`RenderOptions`] for every
/// document, whatever a document's own setup stored: on, type in a missing font has pink behind
/// it; off, it does not.
#[test]
fn highlight_substituted_fonts_follows_the_preference_in_every_document() {
    use vectorcraft_doc::text::{CharStyle, TextObject};
    use vectorcraft_doc::{Node, NodeKind};
    use vectorcraft_geom::Point;

    let on = RenderOptions { background: Some([255, 255, 255, 255]), highlight_substitutions: true, ..Default::default() };
    assert!(on.highlight_substituted_fonts, "on by default");
    let off = RenderOptions { highlight_substituted_fonts: false, ..on.clone() };
    let pinkish = |px: &[u8]| px.chunks(4).filter(|p| p[0] > 200 && p[1] < 180 && p[2] > 150 && p[2] < 230).count();
    // A document saved with the old Document Setup value on, and one with it off.
    for stored in [true, false] {
        let mut d = Document::new(200.0, 100.0);
        d.setup.highlight_substituted_fonts = stored;
        let style = CharStyle { size: 30.0, font_family: "No Such Family".into(), ..Default::default() };
        let n = Node::new(d.alloc_id(), NodeKind::Text(Box::new(TextObject::point(Point::new(10.0, 60.0), "Hamburg", style))));
        let l = d.layers[0].id;
        d.insert(Some(l), 0, n).unwrap();
        let with = Renderer::new().render(&d, 200, 100, Affine::IDENTITY, &on);
        let without = Renderer::new().render(&d, 200, 100, Affine::IDENTITY, &off);
        assert!(pinkish(&with.pixels) > 50, "stored {stored}: the preference on highlights the text");
        assert_eq!(pinkish(&without.pixels), 0, "stored {stored}: the preference off leaves no highlight");
    }
}

#[test]
fn is_opentype_alternate_marks_ligatures_and_forms() {
    assert!(is_opentype_alternate(100, 100, 2), "ligature cluster");
    assert!(is_opentype_alternate(50, 99, 1), "contextual / stylistic form");
    assert!(!is_opentype_alternate(50, 50, 1), "nominal cmap glyph");
    assert!(!is_opentype_alternate(50, 0, 2), "missing glyph");
}

#[test]
fn highlight_alternate_glyphs_paints_pink_behind_a_ligature() {
    use vectorcraft_doc::text::{CharStyle, TextObject};
    use vectorcraft_doc::{Node, NodeKind};
    use vectorcraft_geom::Point;

    // Source Serif 4 ships with standard ligatures for "fi".
    let mut d = Document::new(200.0, 100.0);
    let style = CharStyle { size: 48.0, font_family: "Source Serif 4".into(), ..Default::default() };
    let mut n = Node::new(NodeId(0), NodeKind::Text(Box::new(TextObject::point(Point::new(20.0, 70.0), "fi", style))));
    n.id = d.alloc_id();
    let l = d.layers[0].id;
    d.insert(Some(l), 0, n).unwrap();

    let off = RenderOptions {
        background: Some([255, 255, 255, 255]),
        highlight_substitutions: true,
        highlight_alternate_glyphs: false,
        ..Default::default()
    };
    let on = RenderOptions { highlight_alternate_glyphs: true, ..off.clone() };
    let mut r = Renderer::new();
    let without = r.render(&d, 200, 100, Affine::IDENTITY, &off);
    let with = r.render(&d, 200, 100, Affine::IDENTITY, &on);
    assert_ne!(without.pixels, with.pixels, "the preference changes the frame");
    // Pink SUBSTITUTED = (255, 120, 190) at alpha 110, blended over white / ink.
    let pinkish = with.pixels.chunks(4).filter(|p| p[0] > 200 && p[1] < 180 && p[2] > 150 && p[2] < 230).count();
    assert!(pinkish > 10, "pink highlight cells behind the ligature ({pinkish} pixels)");
}

/// The number of glyph boxes Highlight Alternate Glyphs paints for `t`.
fn alternates(t: &vectorcraft_doc::text::TextObject) -> usize {
    text_geom(t).alternate_glyphs(t).elements().iter().filter(|e| matches!(e, kurbo::PathEl::MoveTo(_))).count()
}

/// An installed font that shapes Arabic with OpenType (as `vectorcraft-text`'s bidi tests pick
/// one): Arial or Geeza Pro on macOS and Windows, Noto Sans Arabic on Linux. No font is kept in
/// the repository, so the tests that need one say so and stop without it.
fn arabic_style(test: &str) -> Option<vectorcraft_doc::text::CharStyle> {
    let db = vectorcraft_text::FontDb::global();
    let face = ["Arial", "Noto Sans Arabic", "Geeza Pro"]
        .into_iter()
        .filter_map(|name| db.face(name, "Regular"))
        .find(|f| ['خ', 'ل', 'ا', 'م', 'ي'].iter().all(|c| f.covers(*c)));
    match face {
        Some(f) => {
            eprintln!("ASSERTIONS RUN: {test}; Arabic font {} {}", f.family, f.style);
            Some(vectorcraft_doc::text::CharStyle { size: 36.0, font_family: f.family.clone(), font_style: f.style.clone(), ..Default::default() })
        }
        None => {
            eprintln!("SKIPPED: {test}; no installed font covers Arabic");
            None
        }
    }
}

fn text(s: &str, style: vectorcraft_doc::text::CharStyle) -> vectorcraft_doc::text::TextObject {
    vectorcraft_doc::text::TextObject::point(vectorcraft_geom::Point::ZERO, s, style)
}

/// The glyphs of `t` by cluster, as the layout shapes them.
fn glyphs(t: &vectorcraft_doc::text::TextObject) -> Vec<(usize, usize, u32)> {
    vectorcraft_text::layout(vectorcraft_text::FontDb::global(), t).glyphs.iter().map(|g| (g.byte, g.len, g.gid)).collect()
}

/// #394: Arabic letters take their joining forms (`init`, `medi`, `fina`, `isol`) in every word:
/// required shaping, not alternates, so ordinary Arabic text shows no pink.
#[test]
fn arabic_joining_forms_are_not_alternates() {
    // Looking up an installed font loads it, which changes the fonts.
    let _adding = crate::tests_fontchange::adding_fonts();
    let Some(style) = arabic_style("arabic_joining_forms_are_not_alternates") else { return };
    let t = text("خلطة مبتكرة من اللومي الحساوي", style);
    // The words are shaped: most glyphs are not their letter's cmap glyph.
    assert!(!text_geom(&t).alternate_candidates.is_empty(), "the shaper joins the letters");
    assert_eq!(alternates(&t), 0);
}

/// #394: the lam-alef ligature is required (`rlig`), so it isn't highlighted either. With an
/// older font that files it under `liga` (as the legacy Arial of Microsoft's core fonts does) it
/// is still required: [`lam_alef_in_liga_is_not_an_alternate`] checks that without an OS font.
#[test]
fn lam_alef_is_not_an_alternate() {
    // Looking up an installed font loads it, which changes the fonts.
    let _adding = crate::tests_fontchange::adding_fonts();
    let Some(style) = arabic_style("lam_alef_is_not_an_alternate") else { return };
    for s in ["لا", "سلام", "الاسم"] {
        assert_eq!(alternates(&text(s, style.clone())), 0, "{s}");
    }
}

/// #394: a font whose lam-alef ligature is in `liga` (older Arabic fonts) or `rlig` gets no
/// highlight on it: the ligature is required by the characters, not chosen by an optional
/// feature. Built in code, so it runs whatever fonts the machine has.
#[test]
fn lam_alef_in_liga_is_not_an_alternate() {
    use vectorcraft_text::test_fonts::{lam_alef_family, lam_alef_font};
    let _adding = crate::tests_fontchange::adding_fonts();
    let db = vectorcraft_text::FontDb::global();
    for feature in [b"liga", b"rlig"] {
        db.add_font(lam_alef_font(feature).unwrap());
        let style = vectorcraft_doc::text::CharStyle { size: 36.0, font_family: lam_alef_family(feature).into(), ..Default::default() };
        for s in ["\u{0644}\u{0627}", "\u{0644}\u{0623}", "\u{0628}\u{0644}\u{0627}", "\u{0644}\u{064E}\u{0627}"] {
            let t = text(s, style.clone());
            // The font's ligature glyph stands for the pair.
            assert!(glyphs(&t).iter().any(|g| g.1 > 2), "{}: {s} is shaped to one ligature glyph", lam_alef_family(feature));
            assert_eq!(alternates(&t), 0, "{}: {s}", lam_alef_family(feature));
        }
    }
}

/// Only a lam followed by an alef (marks between or after allowed) is the required lam-alef.
#[test]
fn only_lam_alef_clusters_are_the_required_ligature() {
    for s in ["\u{0644}\u{0627}", "\u{0644}\u{0622}", "\u{0644}\u{0623}", "\u{0644}\u{0625}", "\u{0644}\u{064E}\u{0627}", "\u{0644}\u{0627}\u{064B}"]
    {
        assert!(crate::text_geom::is_lam_alef(s), "{s:?}");
    }
    for s in ["", "\u{0644}", "\u{0627}\u{0644}", "\u{0644}\u{0628}", "\u{0628}\u{0644}\u{0627}", "\u{0644}\u{0627}\u{0627}", "fi", "\u{064E}"] {
        assert!(!crate::text_geom::is_lam_alef(s), "{s:?}");
    }
}

/// #394: an optional feature the characters turn on (a stylistic set, swash, stylistic or
/// discretionary alternate…) is highlighted on the glyphs it changes, and only on those.
#[test]
fn optional_features_on_arabic_flag_only_the_glyphs_they_change() {
    // Looking up an installed font loads it, which changes the fonts.
    let _adding = crate::tests_fontchange::adding_fonts();
    let db = vectorcraft_text::FontDb::global();
    let s = "خلطة مبتكرة من اللومي الحساوي";
    // Arabic fonts that may have optional features; the first installed one with a feature that
    // changes a glyph of the text.
    let fonts = ["Amiri", "El Messiri", "Noto Naskh Arabic", "Scheherazade New", "Arial", "Noto Sans Arabic", "Geeza Pro"];
    let tags = ["swsh", "salt", "dlig", "ss01", "ss02", "ss03", "ss04", "ss05", "ss06", "ss07", "ss08", "ss09", "ss10"];
    let found = fonts.into_iter().filter_map(|name| db.face(name, "Regular")).filter(|f| f.covers('خ') && f.covers('ل')).find_map(|f| {
        let style = vectorcraft_doc::text::CharStyle { size: 36.0, font_family: f.family.clone(), font_style: f.style.clone(), ..Default::default() };
        let plain = glyphs(&text(s, style.clone()));
        tags.into_iter().find_map(|tag| {
            let t = text(s, vectorcraft_doc::text::CharStyle { features: vec![tag.into()], ..style.clone() });
            let changed = glyphs(&t).iter().filter(|g| !plain.contains(g)).count();
            (changed > 0).then(|| (f.family.clone(), tag, t, changed))
        })
    });
    let Some((family, tag, t, changed)) = found else {
        eprintln!(
            "SKIPPED: optional_features_on_arabic_flag_only_the_glyphs_they_change; no installed Arabic font has an optional feature for this text"
        );
        return;
    };
    eprintln!("ASSERTIONS RUN: optional_features_on_arabic_flag_only_the_glyphs_they_change; {family} with {tag}, {changed} glyphs changed");
    assert_eq!(alternates(&t), changed, "{family} {tag}: the glyphs it changed");
}

/// Latin keeps its highlight: the `ffi` of "office" is a standard ligature (`liga`).
#[test]
fn a_latin_standard_ligature_is_still_an_alternate() {
    let style = vectorcraft_doc::text::CharStyle { size: 48.0, font_family: "Source Serif 4".into(), ..Default::default() };
    let t = text("office", style.clone());
    let shaped = glyphs(&t);
    assert!(shaped.iter().any(|(_, len, _)| *len > 1), "Source Serif 4 ligates ffi: {shaped:?}");
    assert!(alternates(&t) >= 1);
    // Without ligatures nothing is substituted.
    assert_eq!(alternates(&text("office", vectorcraft_doc::text::CharStyle { features: vec!["-liga".into()], ..style })), 0);
}

/// #394: in a font the app ships (Source Serif 4), an optional feature turned on for the
/// characters is highlighted on exactly the glyphs it changes, the others staying clear.
#[test]
fn an_optional_feature_flags_only_the_glyphs_it_changes() {
    let style = vectorcraft_doc::text::CharStyle { size: 36.0, font_family: "Source Serif 4".into(), ..Default::default() };
    // No ligature in it, so the default features substitute nothing.
    let s = "Quay 1234 agy";
    assert_eq!(alternates(&text(s, style.clone())), 0);
    let plain = glyphs(&text(s, style.clone()));
    let tags = ["onum", "smcp", "salt", "swsh", "ss01", "ss02", "ss03", "ss04", "ordn", "sups"];
    let found = tags.into_iter().find_map(|tag| {
        let t = text(s, vectorcraft_doc::text::CharStyle { features: vec![tag.into()], ..style.clone() });
        let changed = glyphs(&t).iter().filter(|g| !plain.contains(g)).count();
        (changed > 0 && changed < plain.len()).then_some((tag, t, changed))
    });
    let (tag, t, changed) = found.expect("Source Serif 4 has an optional feature that changes some of these glyphs");
    assert_eq!(alternates(&t), changed, "{tag}: the glyphs it changed");
}
