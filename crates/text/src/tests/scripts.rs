//! Superscript, subscript and synthesized small caps (Character panel, Document Setup → Type).

use kurbo::Shape;
use vectorcraft_doc::{CharPosition, CharStyle, ScriptMetrics, TextObject};

use crate::*;

fn layout_of(text: &str, st: CharStyle) -> TextLayout {
    layout(FontDb::global(), &TextObject::point(kurbo::Point::ZERO, text, st))
}

fn glyph_box(l: &TextLayout, i: usize) -> kurbo::Rect {
    l.glyphs[i].outline.bounding_box()
}

#[test]
fn superscript_and_subscript_scale_and_shift_glyphs() {
    let base = CharStyle { size: 20.0, ..CharStyle::default() };
    let normal = layout_of("H", base.clone());
    let m = ScriptMetrics { size: 50.0, position: 40.0 };
    let sup = layout_of("H", CharStyle { position: CharPosition::Superscript(m), ..base.clone() });
    let sub = layout_of("H", CharStyle { position: CharPosition::Subscript(m), ..base.clone() });
    let (n, p, b) = (glyph_box(&normal, 0), glyph_box(&sup, 0), glyph_box(&sub, 0));
    // Half the height and advance.
    assert!((p.height() - n.height() * 0.5).abs() < 0.05, "{p:?} vs {n:?}");
    assert!((sup.glyphs[0].advance - normal.glyphs[0].advance * 0.5).abs() < 1e-6);
    // Raised / lowered by 40% of 20 pt (y points down).
    assert!((p.y1 - (n.y1 - 8.0)).abs() < 0.05, "{p:?}");
    assert!((b.y1 - (n.y1 + 8.0)).abs() < 0.05, "{b:?}");
    // Line metrics keep the full size.
    assert_eq!(sup.lines[0].ascent, normal.lines[0].ascent);
}

#[test]
fn small_caps_draw_lowercase_as_smaller_capitals() {
    let base = CharStyle { size: 20.0, ..CharStyle::default() };
    let cap = layout_of("A", base.clone());
    let cap14 = layout_of("A", CharStyle { size: 14.0, ..base.clone() });
    let small = layout_of("Aa", CharStyle { small_caps: Some(70.0), ..base.clone() });
    // The capital is unchanged; the lowercase letter is a capital A at 70% (14 pt).
    assert_eq!(small.glyphs[0].advance, cap.glyphs[0].advance);
    assert!((small.glyphs[1].advance - cap14.glyphs[0].advance).abs() < 1e-6);
    let (a, s) = (glyph_box(&cap14, 0), glyph_box(&small, 1));
    assert!((s.height() - a.height()).abs() < 0.05, "{s:?} vs {a:?}");
    // Byte offsets still map back to the source characters.
    assert_eq!((small.glyphs[1].byte, small.glyphs[1].len), (1, 1));
    // All Caps wins over Small Caps.
    let all = layout_of("a", CharStyle { all_caps: true, small_caps: Some(70.0), ..base });
    assert_eq!(all.glyphs[0].advance, cap.glyphs[0].advance);
}
