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
