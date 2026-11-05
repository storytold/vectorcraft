//! The advances a reader gives live type ([`reader_advances`]): the same as when the type is laid
//! out again each time, which happens only when a reader sets the type differently from its layout.

use super::*;
use vectorcraft_doc::text::{Bullet, ParaList};
use vectorcraft_doc::{Document, JustificationOptions, Mojikumi, TextRun};
use vectorcraft_geom::{Point, shapes};

/// The advances with `t` laid out again as a reader sets it, whatever it holds.
fn laid_out_again(t: &TextObject, lay: &vectorcraft_text::TextLayout, kerning: bool) -> HashMap<usize, (f64, f64)> {
    let mut set = TextObject { kind: TextKind::Point, ..t.clone() };
    for p in std::iter::once(&mut set.para).chain(set.paras.iter_mut()) {
        p.mojikumi = Mojikumi::None;
        p.justification = JustificationOptions::default();
    }
    if !kerning {
        for r in &mut set.runs {
            r.style.kerning.get_or_insert(0.0);
        }
    }
    let mut out: HashMap<usize, (f64, f64)> = HashMap::new();
    for g in vectorcraft_text::layout(FontDb::global(), &set).glyphs.iter().filter(|g| g.len > 0) {
        let (advance, _) = out.entry(g.byte).or_insert((0.0, (g.xf.translation().x - g.origin.x) * lay.fit_scale));
        *advance += g.advance * lay.fit_scale;
    }
    out
}

fn style() -> CharStyle {
    CharStyle { size: 12.0, fill: Paint::solid(vectorcraft_color::Color::BLACK), ..CharStyle::default() }
}

/// Point type with Line-end Punctuation Half Width, the setting new type gets.
fn point(text: &str) -> TextObject {
    let mut t = TextObject::point(Point::new(20.0, 20.0), text, style());
    t.para.mojikumi = Mojikumi::LineEndHalf;
    t
}

/// How many times [`reader_advances`] laid out `t` again (`kerning` as it is given), after checking
/// that it gives what laying out again gives.
fn layouts_again(t: &TextObject, kerning: bool) -> usize {
    let lay = vectorcraft_text::layout(FontDb::global(), t);
    READER_LAYOUTS.with(|n| n.set(0));
    let got = reader_advances(t, &lay, kerning);
    let again = READER_LAYOUTS.with(std::cell::Cell::get);
    assert_eq!(got, laid_out_again(t, &lay, kerning), "{:?}", t.plain_text());
    again
}

/// SVG export of `texts`, each a type object, and how many times it laid type out again for its
/// reader advances.
fn export_layouts_again(texts: Vec<TextObject>) -> (String, usize) {
    let mut d = Document::new(300.0, 200.0);
    let l = d.layers[0].id;
    for t in texts {
        let n = Node::new(d.alloc_id(), NodeKind::Text(Box::new(t)));
        d.insert(Some(l), usize::MAX, n).unwrap();
    }
    READER_LAYOUTS.with(|n| n.set(0));
    let svg = crate::export(&d, &crate::ExportOptions::default());
    (svg, READER_LAYOUTS.with(std::cell::Cell::get))
}

#[test]
fn latin_type_with_mojikumi_is_not_laid_out_again() {
    assert_eq!(layouts_again(&point("Latin words, digits 2026 and (brackets)."), true), 0);
    let (svg, again) = export_layouts_again(vec![point("Ab0"), point("Ab1"), point("Ab2")]);
    assert_eq!(again, 0, "{svg}");
    assert!(svg.contains(">Ab1</tspan>"), "{svg}");
}

#[test]
fn empty_and_one_character_type() {
    assert_eq!(layouts_again(&point(""), true), 0);
    assert_eq!(layouts_again(&point("A"), true), 0);
    assert_eq!(layouts_again(&point("A"), false), 1);
    let (svg, again) = export_layouts_again(vec![point(""), point("A")]);
    assert_eq!(again, 0, "{svg}");
}

#[test]
fn japanese_type_with_mojikumi_is_laid_out_again() {
    for text in ["「かな」、漢字とLatin。", "雅", "、"] {
        assert_eq!(layouts_again(&point(text), true), 1, "{text}");
    }
    // Without Mojikumi, a reader sets it as the layout does.
    let mut none = point("「かな」、漢字とLatin。");
    none.para.mojikumi = Mojikumi::None;
    assert_eq!(layouts_again(&none, true), 0);
}

#[test]
fn a_japanese_run_or_paragraph_lays_the_whole_object_out_again() {
    let mut runs = point("Latin ");
    runs.runs.push(TextRun::new("漢字", style()));
    assert_eq!(layouts_again(&runs, true), 1);
    let paras = point("Latin\n漢字");
    assert_eq!(layouts_again(&paras, true), 1);
}

#[test]
fn area_and_justified_type_is_laid_out_again() {
    let mut area = point("aa bb cc dd ee ff");
    area.kind = TextKind::Area { frame: shapes::rectangle(Rect::new(0.0, 0.0, 60.0, 160.0)) };
    area.para.justify = Justify::JustifyLeft;
    assert_eq!(layouts_again(&area, true), 1);
    let mut spaced = point("Latin words");
    spaced.para.justification = JustificationOptions { letter_spacing: [0.0, 5.0, 10.0], ..JustificationOptions::default() };
    assert_eq!(layouts_again(&spaced, true), 1);
}

#[test]
fn unkerned_advances_lay_out_again_only_for_runs_without_manual_kerning() {
    let mut wide = point("AVAV");
    wide.runs[0].style.h_scale = 150.0;
    assert_eq!(layouts_again(&wide, false), 1);
    wide.runs[0].style.kerning = Some(0.0);
    assert_eq!(layouts_again(&wide, false), 0);
}

#[test]
fn list_items_with_latin_markers_are_not_laid_out_again() {
    let mut bullets = point("first\nsecond");
    bullets.para.list = Some(ParaList::bulleted(Bullet::Dot));
    assert_eq!(layouts_again(&bullets, true), 0);
    let mut numbers = point("一つ\n二つ");
    numbers.para.list = Some(ParaList::numbered(vectorcraft_doc::text::NumberStyle::Arabic));
    assert_eq!(layouts_again(&numbers, true), 1);
}

#[test]
fn a_mixed_document_lays_out_again_only_the_japanese_type() {
    let (svg, again) = export_layouts_again(vec![point("Latin"), point("「かな」。"), point(""), point("Ab2")]);
    assert_eq!(again, 1, "{svg}");
}
