//! Strokes on type keep every stroke option in PDF: the characters' cap, join, miter limit and
//! dash array, and the alignment of the object's own strokes.

use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{AppearanceItem, CharStyle, Dash, Document, LineCap, LineJoin, Node, NodeKind, StrokeAlign, StrokeLayer, TextObject};
use vectorcraft_geom::Point;

use crate::*;

#[test]
fn text_strokes_keep_cap_join_miter_dashes_and_alignment() {
    let style = CharStyle {
        size: 48.0,
        stroke: Paint::solid(Color::BLACK),
        stroke_width: 1.5,
        stroke_cap: LineCap::Round,
        stroke_join: LineJoin::Miter,
        stroke_miter_limit: 7.0,
        stroke_dash: Some(Dash { pattern: vec![4.0, 2.0], ..Default::default() }),
        ..Default::default()
    };
    let mut d = Document::new(240.0, 100.0);
    let mut n = Node::new(d.alloc_id(), NodeKind::Text(Box::new(TextObject::point(Point::new(10.0, 70.0), "Wave", style))));
    let mut own = StrokeLayer::new(Paint::solid(Color::rgb8(0, 0, 255)), 3.0);
    own.align = StrokeAlign::Inside;
    n.appearance.items.push(AppearanceItem::Stroke(own));
    let l = d.layers[0].id;
    d.insert(Some(l), 0, n).unwrap();
    let pdf = export(&d, &PdfOptions::uncompressed()).unwrap();
    assert!(String::from_utf8_lossy(&pdf).contains("7 M 1 J[4 2]0 d"), "the characters' miter limit, cap and dash array");
    let back = import(&pdf).unwrap();
    let mut strokes = vec![];
    back.walk(|n| strokes.extend(n.appearance.items.iter().filter_map(|i| if let AppearanceItem::Stroke(s) = i { Some(s.clone()) } else { None })));
    let chars = strokes.iter().find(|s| s.dash.is_some()).expect("the characters' stroke");
    assert_eq!((chars.cap, chars.join), (LineCap::Round, LineJoin::Miter));
    assert!((chars.miter_limit - 7.0).abs() < 1e-3, "miter limit {}", chars.miter_limit);
    // An inside stroke is drawn twice as wide, clipped to the glyphs.
    assert!(strokes.iter().any(|s| s.dash.is_none() && (s.width - 6.0).abs() < 1e-3), "{strokes:?}");
}

/// Align Stroke on live type: the characters' stroke drawn twice as wide, clipped inside or
/// outside the glyphs, as on the canvas; the type stays text.
#[test]
fn an_aligned_character_stroke_is_clipped_to_its_side_of_the_glyphs() {
    for (align, clip) in [(StrokeAlign::Inside, false), (StrokeAlign::Outside, true)] {
        let style = CharStyle { size: 48.0, stroke: Paint::solid(Color::BLACK), stroke_width: 1.5, stroke_align: align, ..Default::default() };
        let mut d = Document::new(240.0, 100.0);
        let n = Node::new(d.alloc_id(), NodeKind::Text(Box::new(TextObject::point(Point::new(10.0, 70.0), "Wave", style))));
        let l = d.layers[0].id;
        d.insert(Some(l), 0, n).unwrap();
        let pdf = String::from_utf8_lossy(&export(&d, &PdfOptions::uncompressed()).unwrap()).into_owned();
        let ops: Vec<&str> = pdf.split_ascii_whitespace().collect();
        assert!(ops.windows(2).any(|w| w == ["3", "w"]), "{align:?}: twice the weight");
        assert!(ops.contains(&"BT"), "{align:?}: the type stays text");
        assert!(ops.contains(&if clip { "W*" } else { "W" }), "{align:?}: clipped to its side");
    }
}

/// The cap, join and miter limit a PDF strokes its type with, whatever the rendering mode that
/// strokes it (stroke, fill and stroke, and either with a clip), and with replacement text
/// (`ActualText`) too: not butt caps and miter joins (#7741).
#[test]
fn imported_stroked_type_keeps_the_files_cap_join_and_miter_limit() {
    use vectorcraft_testkit::pdf::{PdfPage, pdf};
    let page = |content: &str| {
        let resources = "/Font << /F1 << /Type /Font /Subtype /Type1 /BaseFont /Helvetica >> >>";
        pdf(&[PdfPage { resources: resources.into(), ..PdfPage::new(300.0, 200.0, content) }], None)
    };
    let styles = |content: &str| {
        let d = import(&page(content)).unwrap();
        let mut runs = vec![];
        d.walk(|n| {
            if let NodeKind::Text(t) = &n.kind {
                runs.extend(t.runs.iter().map(|r| r.style.clone()));
            }
        });
        runs
    };
    let cases = [
        ("1 J 1 j", (LineCap::Round, LineJoin::Round, 10.0)),
        ("2 J 2 j 3 M", (LineCap::Square, LineJoin::Bevel, 3.0)),
        ("0 J 0 j 3 M", (LineCap::Butt, LineJoin::Miter, 3.0)),
    ];
    for (settings, want) in cases {
        for mode in ["1", "2", "5", "6"] {
            // A word shown at once, a glyph at a time (a stroke over the glyph just filled), and
            // in a span with replacement text.
            for text in ["(WAVE) Tj", "(W) Tj (A) Tj", "/Span << /ActualText (WAVE) >> BDC (WAVE) Tj EMC"] {
                let content = format!("0 g 0 0 0 RG 4 w {settings} BT {mode} Tr /F1 60 Tf 40 110 Td {text} 0 Tr ET");
                let runs = styles(&content);
                let stroked: Vec<_> = runs.iter().filter(|s| !s.stroke.is_none()).collect();
                assert!(!stroked.is_empty(), "{content}: stroked type comes in: {runs:?}");
                if matches!(mode, "2" | "6") && !text.starts_with("(WAVE)") {
                    assert!(stroked.iter().any(|s| !s.fill.is_none()), "{content}: the stroke is over the filled glyphs: {runs:?}");
                }
                for s in stroked {
                    assert!((s.stroke_width - 4.0).abs() < 1e-6, "{content}: width {}", s.stroke_width);
                    let got = (s.stroke_cap, s.stroke_join, s.stroke_miter_limit);
                    assert_eq!(got, want, "{content}");
                }
            }
        }
    }
    // Glyphs stroked with another join are a run of their own.
    let runs = styles("0 0 0 RG 4 w BT 1 Tr /F1 60 Tf 40 110 Td 1 j (WA) Tj 0 j (VE) Tj 0 Tr ET");
    let joins: Vec<_> = runs.iter().map(|s| s.stroke_join).collect();
    assert_eq!(joins, [LineJoin::Round, LineJoin::Miter]);
    // Filled and stroked a glyph at a time: the next glyph's fill joins the run before it, and its
    // stroke, with another join, restyles that glyph alone.
    for mode in ["2", "6"] {
        let content = format!("0 g 0 0 0 RG 4 w BT {mode} Tr /F1 60 Tf 40 110 Td 1 j (W) Tj 0 j (A) Tj 0 Tr ET");
        let runs = styles(&content);
        let d = import(&page(&content)).unwrap();
        let mut text = String::new();
        d.walk(|n| {
            if let NodeKind::Text(t) = &n.kind {
                text.push_str(&t.plain_text());
            }
        });
        assert_eq!(text, "WA", "{content}");
        let joins: Vec<_> = runs.iter().map(|s| (!s.stroke.is_none()).then_some(s.stroke_join)).collect();
        assert_eq!(joins, [Some(LineJoin::Round), Some(LineJoin::Miter)], "{content}: {runs:?}");
    }
}

/// Fill and stroke rendering (mode 2, or 6 with a clip) draws a text-showing operator's glyphs
/// filled, then the same glyphs stroked: the type comes in once, filled and stroked, whether the
/// operator shows one glyph or a word, or a TJ array of strings with kerning between them, in an
/// outline font, with or without replacement text (`ActualText`). Stroke rendering (modes 1 and 5) stays stroke only. A Type 3 glyph is filled in
/// every one of those modes, once (#8015).
#[test]
fn a_word_filled_then_stroked_imports_as_one_filled_and_stroked_type() {
    use vectorcraft_testkit::pdf::{PdfPage, first_extra, pdf_with};
    // `/T3`: a Type 3 font whose glyphs take the text's paint (`d1`), `W A V E` mapped by ToUnicode.
    let t3 = first_extra(1);
    let glyph = crate::tests_import_fidelity::stream("", "600 0 0 0 600 700 d1 0 0 m 600 0 l 300 700 l h f");
    let to_unicode = crate::tests_import_fidelity::stream(
        "",
        "/CIDInit /ProcSet findresource begin 12 dict begin begincmap /CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def /CMapName /T3-UCS def /CMapType 2 def 1 begincodespacerange <00> <FF> endcodespacerange 4 beginbfchar\n<41> <0041>\n<45> <0045>\n<56> <0056>\n<57> <0057>\nendbfchar endcmap CMapName currentdict /CMap defineresource pop end end",
    );
    let font = format!(
        "<< /Type /Font /Subtype /Type3 /FontBBox [0 0 600 700] /FontMatrix [0.001 0 0 0.001 0 0] /CharProcs << /g {} 0 R >> /Encoding << /Type /Encoding /Differences [65 /g 69 /g 86 /g 87 /g] >> /FirstChar 65 /LastChar 87 /Widths [600 0 0 0 600 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 600 600] /ToUnicode {} 0 R /Resources << >> >>",
        t3 + 1,
        t3 + 2
    );
    let page = |content: &str| {
        let resources = format!("/Font << /F1 << /Type /Font /Subtype /Type1 /BaseFont /Helvetica >> /T3 {t3} 0 R >>");
        pdf_with(&[PdfPage { resources, ..PdfPage::new(300.0, 200.0, content) }], &[&font, &glyph, &to_unicode], None)
    };
    let objects = |content: &str| {
        let d = import(&page(content)).unwrap();
        let mut types = vec![];
        d.walk(|n| {
            if let NodeKind::Text(t) = &n.kind {
                types.push((**t).clone());
            }
        });
        types
    };
    let cream = Paint::solid(Color::rgb8(0xff, 0xff, 0xcc));
    let black = Paint::solid(Color::BLACK);
    let shows =
        ["(WAVE) Tj", "[(WA) -50 (VE)] TJ", "(WA) Tj (VE) Tj", "(W) Tj (A) Tj (V) Tj (E) Tj", "/Span << /ActualText (WAVE) >> BDC (WAVE) Tj EMC"];
    for font in ["F1", "T3"] {
        for show in shows {
            for mode in ["0", "1", "2", "4", "5", "6"] {
                let content = format!("0 0 0 RG 1 1 0.8 rg 3 w 1 J 1 j BT {mode} Tr /{font} 60 Tf 40 110 Td {show} 0 Tr ET");
                let types = objects(&content);
                assert_eq!(types.len(), 1, "{content}: one type object: {types:#?}");
                let t = &types[0];
                assert_eq!(t.plain_text(), "WAVE", "{content}");
                // A Type 3 glyph paints itself: the reader fills it whatever the mode.
                let (fill, stroke) = match mode {
                    _ if font == "T3" => (true, false),
                    "0" | "4" => (true, false),
                    "1" | "5" => (false, true),
                    _ => (true, true),
                };
                for r in &t.runs {
                    let s = &r.style;
                    assert_eq!(s.fill, if fill { cream.clone() } else { Paint::None }, "{content}: fill: {:?}", t.runs);
                    if stroke {
                        assert_eq!(s.stroke, black, "{content}: stroke: {:?}", t.runs);
                        assert!((s.stroke_width - 3.0).abs() < 1e-6, "{content}: width {}", s.stroke_width);
                        assert_eq!((s.stroke_cap, s.stroke_join), (LineCap::Round, LineJoin::Round), "{content}");
                    } else {
                        assert!(s.stroke.is_none(), "{content}: no stroke: {:?}", t.runs);
                    }
                }
            }
        }
    }
    // A word filled, then stroked with another join part way through (one operator each):
    // each part keeps its own join, all of it filled.
    let content = "0 0 0 RG 1 1 0.8 rg 3 w BT 2 Tr /F1 60 Tf 40 110 Td 1 j (WA) Tj 0 j (VE) Tj 0 Tr ET";
    let types = objects(content);
    assert_eq!(types.len(), 1, "{content}: {types:#?}");
    let joins: Vec<_> = types[0].runs.iter().map(|r| (r.text.as_str(), r.style.fill == cream, r.style.stroke_join)).collect();
    assert_eq!(joins, [("WA", true, LineJoin::Round), ("VE", true, LineJoin::Miter)], "{content}");
    // Filled type, then stroked type of its own elsewhere: a stroke over no filled glyph.
    let content = "0 0 0 RG 1 1 0.8 rg 3 w BT 0 Tr /F1 60 Tf 40 110 Td (WAVE) Tj 1 Tr 0 -70 Td (WAVE) Tj 0 Tr ET";
    let types = objects(content);
    assert_eq!(types.len(), 2, "{content}: {types:#?}");
    let looks: Vec<_> = types.iter().flat_map(|t| &t.runs).map(|r| (r.style.fill == cream, !r.style.stroke.is_none())).collect();
    assert_eq!(looks, [(true, false), (false, true)], "{content}");
}

/// Helvetica PDF page drawing `content`, imported: each run of type as its text, size, and
/// whether it is filled and stroked, in text order.
fn word_runs(content: &str) -> Vec<(String, f64, bool, bool)> {
    use vectorcraft_testkit::pdf::{PdfPage, pdf};
    let resources = "/Font << /F1 << /Type /Font /Subtype /Type1 /BaseFont /Helvetica >> >>";
    let bytes = pdf(&[PdfPage { resources: resources.into(), ..PdfPage::new(300.0, 200.0, content) }], None);
    let d = import(&bytes).unwrap();
    let mut runs = vec![];
    d.walk(|n| {
        if let NodeKind::Text(t) = &n.kind {
            runs.extend(t.runs.iter().map(|r| (r.text.clone(), r.style.size, !r.style.fill.is_none(), !r.style.stroke.is_none())));
        }
    });
    runs.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)));
    runs
}

/// A stroke goes only over a glyph filled with the very same placement: a word filled, then its
/// first glyph stroked at another size from the same origin, is two pieces of type, the word
/// unstroked and the glyph at its own size (#8015).
#[test]
fn a_stroke_at_another_size_does_not_go_over_the_filled_glyph() {
    for mode in ["1", "2"] {
        let content = format!("0 0 0 RG 1 1 0.8 rg 3 w BT /F1 60 Tf 40 110 Td (WAVE) Tj ET BT {mode} Tr /F1 30 Tf 40 110 Td (W) Tj 0 Tr ET");
        let filled = mode == "2";
        assert_eq!(word_runs(&content), [("W".to_string(), 30.0, filled, true), ("WAVE".to_string(), 60.0, true, false)], "{content}");
    }
}

/// A stroke over one glyph of a filled word strokes that glyph alone, not the rest of the word
/// (#8015).
#[test]
fn a_stroke_over_one_glyph_of_a_filled_word_strokes_only_that_glyph() {
    // W alone stroked at the same size from the same origin: W is stroked, AVE not.
    let content = "0 0 0 RG 1 1 0.8 rg 3 w BT /F1 60 Tf 40 110 Td (WAVE) Tj ET BT 1 Tr /F1 60 Tf 40 110 Td (W) Tj 0 Tr ET";
    assert_eq!(word_runs(content), [("AVE".to_string(), 60.0, true, false), ("W".to_string(), 60.0, true, true)], "{content}");
    // Filled and stroked there instead: that W is drawn again, over the word, as type of its own.
    let content = "0 0 0 RG 1 1 0.8 rg 3 w BT /F1 60 Tf 40 110 Td (WAVE) Tj ET BT 2 Tr /F1 60 Tf 40 110 Td (W) Tj 0 Tr ET";
    assert_eq!(word_runs(content), [("W".to_string(), 60.0, true, true), ("WAVE".to_string(), 60.0, true, false)], "{content}");
}

/// A glyph filled at one opacity and stroked at another (`ca` and `CA` differ) stays two pieces
/// of type, each at its own opacity, also when it is shown on its own, so the stroke is drawn
/// right after its fill (#8015). Two such operators are four pieces in the order they are drawn,
/// the second glyph's fill over the first one's stroke.
#[test]
fn a_glyph_stroked_at_another_opacity_than_its_fill_stays_its_own_type() {
    let (fill, stroke) = ((64.0 / 255.0, true, false), (191.0 / 255.0, false, true));
    for (show, want) in [("(A) Tj", vec![fill, stroke]), ("(A) Tj (B) Tj", vec![fill, stroke, fill, stroke])] {
        let content = format!("/G gs 1 0 0 rg 0 0 1 RG 2 w BT /F1 20 Tf 2 Tr 1 0 0 1 10 20 Tm {show} 0 Tr ET");
        let d = import(&crate::tests::reader_compat::text_pdf(&content)).unwrap();
        let mut texts = vec![];
        d.walk(|n| {
            if let NodeKind::Text(t) = &n.kind {
                let st = t.first_style();
                texts.push((n.opacity, !st.fill.is_none(), !st.stroke.is_none()));
            }
        });
        assert_eq!(texts, want, "{content}");
    }
}

/// Type filled and stroked at the same opacity below 1 stays a filled object and a stroked one:
/// the file composites the fill and the stroke apiece (darker where they overlap), while an
/// object's opacity applies to its fill and stroke together. Opaque, it is one object (#8015).
#[test]
fn translucent_fill_and_stroke_text_stays_two_objects() {
    let half = |fill, stroke| (128.0 / 255.0, fill, stroke);
    for show in ["(A) Tj", "(AB) Tj"] {
        let content = format!("/G gs 1 0 0 rg 0 0 1 RG 2 w BT /F1 20 Tf 2 Tr 1 0 0 1 10 20 Tm {show} 0 Tr ET");
        for (state, want) in [("/ca 0.5 /CA 0.5", vec![half(true, false), half(false, true)]), ("/ca 1 /CA 1", vec![(1.0, true, true)])] {
            let d = import(&crate::tests::reader_compat::text_pdf_with(state, &content)).unwrap();
            let mut texts = vec![];
            d.walk(|n| {
                if let NodeKind::Text(t) = &n.kind {
                    let st = t.first_style();
                    texts.push((n.opacity, !st.fill.is_none(), !st.stroke.is_none()));
                }
            });
            assert_eq!(texts, want, "{state}: {content}");
        }
    }
}

/// A stroke goes only over a glyph filled with the same text: "AB" filled, its "A" stroked, then
/// an "X" stroked where the "B" stands is type of its own, and the "B" stays unstroked (#8015).
#[test]
fn a_stroke_of_another_glyph_where_one_was_filled_stays_its_own_type() {
    let content = "0 0 0 RG 1 1 0.8 rg 3 w BT /F1 60 Tf 1 0 0 1 40 110 Tm (AB) Tj 1 Tr 1 0 0 1 40 110 Tm (A) Tj (X) Tj 0 Tr ET";
    let want = [("A".to_string(), 60.0, true, true), ("B".to_string(), 60.0, true, false), ("X".to_string(), 60.0, false, true)];
    assert_eq!(word_runs(content), want, "{content}");
}

/// Helvetica PDF page drawing `content` (fill cream, stroke black, 3 points wide), imported: each
/// type object's runs in drawing order, as their text and whether they are filled and stroked.
fn type_runs(content: &str) -> Vec<Vec<(String, bool, bool)>> {
    use vectorcraft_testkit::pdf::{PdfPage, pdf};
    let resources = "/Font << /F1 << /Type /Font /Subtype /Type1 /BaseFont /Helvetica >> >>";
    let content = format!("0 0 0 RG 1 1 0.8 rg 3 w {content}");
    let bytes = pdf(&[PdfPage { resources: resources.into(), ..PdfPage::new(300.0, 200.0, &content) }], None);
    let d = import(&bytes).unwrap();
    let mut types = vec![];
    d.walk(|n| {
        if let NodeKind::Text(t) = &n.kind {
            types.push(t.runs.iter().map(|r| (r.text.clone(), !r.style.fill.is_none(), !r.style.stroke.is_none())).collect());
        }
    });
    types
}

/// A glyph filled only (mode 0) right after one filled and stroked (mode 2) is filled only: it
/// joins the same type object, but not the stroked run, whether the glyphs before were filled
/// only, filled and stroked, or both, and a glyph filled and stroked after it is stroked again
/// (#8300).
#[test]
fn a_glyph_filled_only_after_one_filled_and_stroked_is_not_stroked() {
    let run = |text: &str, stroked: bool| (text.to_string(), true, stroked);
    let cases = [
        ("2 Tr (B) Tj 0 Tr (C) Tj", vec![run("B", true), run("C", false)]),
        ("0 Tr (A) Tj 2 Tr (B) Tj 0 Tr (C) Tj", vec![run("A", false), run("B", true), run("C", false)]),
        ("2 Tr (B) Tj 0 Tr (C) Tj 2 Tr (D) Tj", vec![run("B", true), run("C", false), run("D", true)]),
        ("2 Tr (AB) Tj 0 Tr (CD) Tj", vec![run("AB", true), run("CD", false)]),
        ("0 Tr (A) Tj 2 Tr (BC) Tj", vec![run("A", false), run("BC", true)]),
    ];
    for (show, want) in cases {
        let content = format!("BT /F1 48 Tf 20 80 Td {show} 0 Tr ET");
        assert_eq!(type_runs(&content), [want], "{content}");
    }
}

/// Fill and stroke rendering glyph by glyph, and a word filled whole then stroked whole at the
/// same place, still come in as one run filled and stroked (#8300).
#[test]
fn glyphs_filled_and_stroked_one_by_one_stay_one_run() {
    for content in [
        "BT /F1 48 Tf 20 80 Td 2 Tr (A) Tj (B) Tj (C) Tj 0 Tr ET",
        "BT /F1 48 Tf 1 0 0 1 20 80 Tm 0 Tr (ABC) Tj 1 Tr 1 0 0 1 20 80 Tm (ABC) Tj 0 Tr ET",
    ] {
        assert_eq!(type_runs(content), [vec![("ABC".to_string(), true, true)]], "{content}");
    }
}
