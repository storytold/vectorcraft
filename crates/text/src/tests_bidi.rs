//! Direction, source-cluster and editing regressions; no RTL font assets required.
use super::*;
use kurbo::{Affine, BezPath, Point, Rect, Shape};
use std::ops::Range;
use std::sync::Arc;
use vectorcraft_doc::{CharStyle, ParaDirection, TextKind, TextRun};

fn text_layout(s: &str) -> TextLayout {
    layout(FontDb::global(), &TextObject::point(Point::ZERO, s, CharStyle::default()))
}

fn arabic_test_face(test: &str) -> Option<Arc<FontFace>> {
    let db = FontDb::global();
    // OpenType (GSUB) shaping only: AAT fonts such as macOS's Geeza Pro shape with `morx`, which
    // takes no context from outside the run, so their joining across a style boundary differs.
    let opentype = |face: &FontFace| {
        use skrifa::raw::TableProvider;
        face.skrifa().is_some_and(|f| f.gsub().is_ok())
    };
    let face = ["Geeza Pro", "Arial", "Noto Sans Arabic"]
        .into_iter()
        .filter_map(|name| db.face(name, "Regular"))
        .find(|face| face.covers('ب') && face.covers('ل') && face.covers('ا') && opentype(face));
    if let Some(face) = &face {
        let path = face.path().map_or_else(|| "bundled font".to_string(), |p| p.display().to_string());
        eprintln!("ASSERTIONS RUN: {test}; Arabic font {} {} ({path})", face.family, face.style);
    } else {
        eprintln!("SKIPPED: {test}; no installed candidate font covers Arabic beh, lam, and alef");
    }
    face
}

fn arabic_style(face: &FontFace) -> CharStyle {
    CharStyle { font_family: face.family.clone(), font_style: face.style.clone(), ..CharStyle::default() }
}

fn assert_same_shaped_glyphs(left: &TextLayout, right: &TextLayout) {
    assert_eq!(left.glyphs.len(), right.glyphs.len());
    for (a, b) in left.glyphs.iter().zip(&right.glyphs) {
        assert_eq!(
            (a.gid, a.byte, a.len, a.font_id, a.advance, a.origin, a.xf, a.rtl),
            (b.gid, b.byte, b.len, b.font_id, b.advance, b.origin, b.xf, b.rtl),
        );
    }
}

fn source_clusters(layout: &TextLayout) -> Vec<(usize, usize)> {
    let mut clusters: Vec<_> = layout.glyphs.iter().map(|glyph| (glyph.byte, glyph.len)).collect();
    clusters.sort_unstable();
    clusters.dedup();
    clusters
}

fn assert_source_coverage(text: &str, layout: &TextLayout) {
    for glyph in &layout.glyphs {
        let end = glyph.byte.checked_add(glyph.len).expect("source cluster end must fit usize");
        assert!(glyph.len > 0 && text.get(glyph.byte..end).is_some(), "invalid UTF-8 source cluster: {glyph:?}");
    }
    let mut covered = 0;
    for (byte, len) in source_clusters(layout) {
        assert_eq!(byte, covered, "source clusters must cover {text:?} without gaps or overlaps");
        covered = byte + len;
    }
    assert_eq!(covered, text.len(), "source clusters must cover all of {text:?}");
}

#[test]
fn hebrew_and_arabic_are_visually_rtl_with_exact_source_clusters() {
    for s in ["שלום", "مرحبا"] {
        let l = text_layout(s);
        assert!(!l.glyphs.is_empty());
        assert!(l.glyphs.iter().all(|g| g.rtl));
        let bytes: Vec<_> = l.glyphs.iter().map(|g| g.byte).collect();
        assert!(bytes.windows(2).all(|w| w[0] >= w[1]), "{s}: {bytes:?}");
        for g in &l.glyphs {
            assert!(s.get(g.byte..g.byte + g.len).is_some(), "{g:?}");
            assert!(g.len <= s.len());
        }
        assert!(caret_position(&l, 0).0.x > caret_position(&l, s.len()).0.x);
    }
}

#[test]
fn mixed_text_preserves_latin_and_number_order() {
    let s = "שלום Rust 123";
    let l = text_layout(s);
    let visible: String = l.glyphs.iter().filter_map(|g| s.get(g.byte..)?.chars().next()).collect();
    assert_eq!(visible, "Rust 123 םולש");
    assert_eq!(s, "שלום Rust 123", "layout must not rewrite source text");
}

/// The characters of `s` in the order a layout of it draws them, left to right, with the bytes
/// `set` given Character Direction `d`.
fn drawn_with_direction(s: &str, set: Range<usize>, d: Option<ParaDirection>) -> String {
    let mut t = TextObject::point(Point::ZERO, s, CharStyle::default());
    edit::style_range(&mut t.runs, set.start, set.end, |st| st.char_direction = d);
    let l = layout(FontDb::global(), &t);
    let mut glyphs: Vec<_> = l.glyphs.iter().collect();
    glyphs.sort_by(|a, b| a.origin.x.total_cmp(&b.origin.x));
    glyphs.iter().filter_map(|g| s.get(g.byte..)?.chars().next()).collect()
}

/// Character Direction Left to Right keeps a date written with spaces in its own order inside a
/// right-to-left paragraph, where the bidi algorithm alone sets its numbers right to left.
#[test]
fn left_to_right_characters_keep_a_date_in_order_in_right_to_left_text() {
    let s = "שלום 21 10 2026";
    let date = s.find('2').unwrap_or_default()..s.len();
    assert_eq!(drawn_with_direction(s, date.clone(), None), "2026 10 21 םולש");
    assert_eq!(drawn_with_direction(s, date, Some(ParaDirection::LeftToRight)), "21 10 2026 םולש");
}

/// Character Direction Right to Left sets a Hebrew word and the exclamation mark after it right to
/// left inside a left-to-right paragraph, where the bidi algorithm alone leaves the mark after the
/// word in the paragraph's direction.
#[test]
fn right_to_left_characters_set_a_word_and_its_mark_right_to_left_in_left_to_right_text() {
    let s = "abc שלום! def";
    let word = s.find('ש').unwrap_or_default()..s.find('!').map_or(0, |i| i + 1);
    assert_eq!(drawn_with_direction(s, word.clone(), None), "abc םולש! def");
    assert_eq!(drawn_with_direction(s, word.clone(), Some(ParaDirection::RightToLeft)), "abc !םולש def");
    // Left to Right on the same characters of a left-to-right paragraph draws them as without it.
    assert_eq!(drawn_with_direction(s, word, Some(ParaDirection::LeftToRight)), "abc םולש! def");
}

#[test]
fn rtl_hit_testing_arrows_and_selection_follow_visual_positions() {
    let s = "שלום";
    let l = text_layout(s);
    for g in &l.glyphs {
        let p = Point::new(g.origin.x + g.advance * 0.9, g.origin.y);
        assert_eq!(hit_byte(&l, p), g.byte);
        let p = Point::new(g.origin.x + g.advance * 0.1, g.origin.y);
        assert_eq!(hit_byte(&l, p), g.byte + g.len);
    }
    assert_eq!(caret_horizontal(&l, 0, false), 2);
    assert_eq!(caret_horizontal(&l, 2, true), 0);
    assert!(!selection_quads(&l, 0, s.len()).is_empty());
}

/// Lines of glyphs, one glyph per cluster, each line given in visual order as (byte, len, advance)
/// and starting at x 0.
fn lines_of_clusters(lines: &[&[(usize, usize, f64)]], rtl: bool) -> TextLayout {
    let mut l = TextLayout::default();
    for (li, clusters) in lines.iter().enumerate() {
        let (glyph_start, baseline) = (l.glyphs.len(), li as f64 * 20.0);
        let mut x = 0.0;
        for &(byte, len, advance) in *clusters {
            l.glyphs.push(PositionedGlyph {
                outline: BezPath::new(),
                run: 0,
                byte,
                origin: Point::new(x, baseline),
                advance,
                len,
                angle: 0.0,
                line: li,
                font_id: 0,
                gid: 0,
                xf: Affine::IDENTITY,
                rtl,
                scale: 1.0,
                image: None,
                colour: None,
            });
            x += advance;
        }
        l.lines.push(LineInfo {
            rtl,
            baseline,
            x0: 0.0,
            x1: x,
            ascent: 10.0,
            descent: 3.0,
            first_baseline: 0.0,
            start: clusters.iter().map(|c| c.0).min().unwrap_or(0),
            end: clusters.iter().map(|c| c.0 + c.1).max().unwrap_or(0),
            glyph_start,
            glyph_end: l.glyphs.len(),
            avail: (0.0, x),
            region: 0,
        });
    }
    l
}

fn arrow_steps(l: &TextLayout, from: usize, right: bool, steps: usize) -> Vec<usize> {
    let mut caret = from;
    (0..steps)
        .map(|_| {
            caret = caret_horizontal(l, caret, right);
            caret
        })
        .collect()
}

#[test]
fn arrows_step_through_both_caret_stops_of_a_cluster_without_advance() {
    // A letter (bytes 0..6), a zero width non-joiner drawn as its own glyph with no advance
    // (6..9) and a letter (9..12): the stops at 6 and 9 share one x.
    let l = lines_of_clusters(&[&[(0, 6, 10.0), (6, 3, 0.0), (9, 3, 10.0)]], false);
    assert_eq!(arrow_steps(&l, 0, true, 4), [6, 9, 12, 12]);
    assert_eq!(arrow_steps(&l, 12, false, 4), [9, 6, 0, 0]);
    // The same clusters right to left: the glyphs run from byte 9 at the left to byte 0 at the right.
    let l = lines_of_clusters(&[&[(9, 3, 10.0), (6, 3, 0.0), (0, 6, 10.0)]], true);
    assert_eq!(arrow_steps(&l, 12, true, 4), [9, 6, 0, 0]);
    assert_eq!(arrow_steps(&l, 0, false, 4), [6, 9, 12, 12]);
}

#[test]
fn arrows_keep_to_their_line_where_a_wrapped_line_starts_at_the_same_x() {
    // A soft wrap at byte 2: the end of the first line is the start of the second, at the same x
    // as the start of the first.
    let l = lines_of_clusters(&[&[(0, 1, 10.0), (1, 1, 10.0)], &[(2, 1, 10.0), (3, 1, 10.0)]], false);
    assert_eq!(arrow_steps(&l, 0, true, 4), [1, 2, 3, 4]);
    assert_eq!(caret_horizontal(&l, 1, false), 0);
}

#[test]
fn wrapping_retains_logical_line_ranges_and_reorders_each_line() {
    let s = "שלום עולם שלום עולם שלום עולם";
    let mut t = TextObject::point(Point::ZERO, s, CharStyle::default());
    t.kind = TextKind::Area { frame: vectorcraft_geom::PathData::from_bezpath(&Rect::new(0.0, 0.0, 55.0, 400.0).to_path(0.1)) };
    let l = layout(FontDb::global(), &t);
    assert!(l.lines.len() > 1);
    assert!(!l.overflow);
    for line in &l.lines {
        assert!(s.get(line.start..line.end).is_some());
        for g in &l.glyphs[line.glyph_start..line.glyph_end] {
            assert!(g.byte >= line.start && g.byte + g.len <= line.end);
        }
    }
}

#[test]
fn paragraph_direction_is_independent_and_neutrals_do_not_force_ltr() {
    let l = text_layout("123 שלום\nEnglish\nمرحبا");
    assert!(l.glyphs.iter().any(|g| g.line == 0 && g.rtl));
    assert!(l.glyphs.iter().filter(|g| g.line == 1).all(|g| !g.rtl));
    assert!(l.glyphs.iter().filter(|g| g.line == 2).all(|g| g.rtl));
}

#[test]
fn hebrew_marks_stay_with_base_clusters() {
    let s = "שָׁלוֹם";
    let l = text_layout(s);
    for g in &l.glyphs {
        assert!(s.get(g.byte..g.byte + g.len).is_some());
    }
    assert!(l.glyphs.iter().all(|g| g.rtl));
}

#[test]
fn arabic_contextual_forms_and_lam_alef_use_the_shaper() {
    let db = FontDb::global();
    // Installed fonts are optional; no proprietary or test font is copied into the repo.
    let Some(face) = arabic_test_face("arabic_contextual_forms_and_lam_alef_use_the_shaper") else { return };
    let st = arabic_style(&face);
    let pair = "لا";
    let lam_alef = layout(db, &TextObject::point(Point::ZERO, pair, st.clone()));
    // Fonts may use a ligature or separate contextual lam/alef glyphs, with component marks.
    assert_source_coverage(pair, &lam_alef);
    for glyph in &lam_alef.glyphs {
        assert!(glyph.rtl, "lam-alef must be RTL: {glyph:?}");
        assert_eq!(glyph.font_id, face.id());
        assert_ne!(glyph.gid, 0, "lam-alef must not use a missing glyph");
        let source = pair.get(glyph.byte..glyph.byte + glyph.len).expect("validated UTF-8 source cluster");
        assert!(source.chars().all(|c| glyph.gid != face.glyph_for(c)), "lam-alef must use contextual glyphs rather than nominal glyphs: {glyph:?}");
    }

    for s in ["سلام", "שלום سلام"] {
        let l = layout(db, &TextObject::point(Point::ZERO, s, st.clone()));
        let arabic: Vec<_> = l.glyphs.iter().filter(|g| s.get(g.byte..).is_some_and(|t| t.starts_with(['س', 'ل', 'ا', 'م']))).collect();
        assert!(
            arabic.iter().any(|g| s.get(g.byte..).and_then(|t| t.chars().next()).is_some_and(|c| g.gid != face.glyph_for(c))),
            "Arabic must use contextual forms: {s}"
        );
        assert!(arabic.iter().all(|g| g.rtl));
    }
}

#[test]
fn identical_style_arabic_run_splits_keep_joining_and_source_attribution() {
    let db = FontDb::global();
    let Some(face) = arabic_test_face("identical_style_arabic_run_splits_keep_joining_and_source_attribution") else { return };
    let style = arabic_style(&face);
    let text = "بب";

    let mut single = TextObject::point(Point::ZERO, text, style.clone());
    single.runs = vec![TextRun { text: text.to_string(), style: style.clone(), inline: None }];
    let mut split = TextObject::point(Point::ZERO, text, style.clone());
    split.runs = vec![TextRun { text: "ب".into(), style: style.clone(), inline: None }, TextRun { text: "ب".into(), style, inline: None }];

    let single = layout(db, &single);
    let split = layout(db, &split);
    assert!(single.glyphs.iter().any(|glyph| glyph.gid != face.glyph_for('ب')), "single run must exercise contextual beh forms");
    assert_same_shaped_glyphs(&single, &split);
    let beh_len = "ب".len();
    assert_eq!(source_clusters(&split), [(0, beh_len), (beh_len, beh_len)], "beh-beh should remain two source clusters");
    for glyph in &single.glyphs {
        assert_eq!(glyph.run, 0);
    }
    // Every component, including zero-advance dots, belongs to its original source run.
    for glyph in &split.glyphs {
        let expected_run = usize::from(glyph.byte >= beh_len);
        assert_eq!(
            (glyph.run, glyph.byte, glyph.len),
            (expected_run, expected_run * beh_len, beh_len),
            "source byte {} belongs to run {expected_run}",
            glyph.byte
        );
    }
}

#[test]
fn different_style_arabic_boundary_receives_joining_context() {
    let db = FontDb::global();
    let Some(face) = arabic_test_face("different_style_arabic_boundary_receives_joining_context") else { return };
    let style = arabic_style(&face);
    let text = "بب";
    let joined = layout(db, &TextObject::point(Point::ZERO, text, style.clone()));
    assert!(joined.glyphs.iter().any(|glyph| glyph.gid != face.glyph_for('ب')), "reference must use contextual beh forms");

    let mut split = TextObject::point(Point::ZERO, text, style.clone());
    split.runs = vec![
        TextRun { text: "ب".into(), style: style.clone(), inline: None },
        TextRun { text: "ب".into(), style: CharStyle { size: style.size * 1.5, ..style }, inline: None },
    ];
    let split = layout(db, &split);
    let beh_len = "ب".len();
    let expected_clusters = [(0, beh_len), (beh_len, beh_len)];
    assert_eq!(source_clusters(&joined), expected_clusters);
    assert_eq!(source_clusters(&split), expected_clusters);
    assert!(joined.glyphs.iter().all(|glyph| glyph.rtl));
    // Compare the full component sequence; a first match by byte can hide or reuse a dot.
    assert_eq!(split.glyphs.len(), joined.glyphs.len(), "style boundary must retain every component glyph");
    for (glyph, actual) in joined.glyphs.iter().zip(&split.glyphs) {
        assert_eq!(actual.gid, glyph.gid, "joining form changed at style boundary for byte {}", glyph.byte);
        assert_eq!(actual.byte, glyph.byte);
        assert_eq!(actual.len, glyph.len);
        assert_eq!(actual.font_id, glyph.font_id);
        assert_eq!(actual.rtl, glyph.rtl);
        assert_eq!(glyph.run, 0);
        assert_eq!(
            (actual.run, actual.byte, actual.len),
            (usize::from(glyph.byte >= beh_len), glyph.byte, beh_len),
            "every component must retain its source run and cluster"
        );
    }
}

#[test]
fn a_change_of_paint_in_a_mixed_direction_line_moves_no_glyph() {
    let db = FontDb::global();
    let text = "AVAV سلام AVAV";
    if db.face_covering('س').is_none_or(|face| !face.covers('ل')) {
        eprintln!("SKIPPED: a_change_of_paint_in_a_mixed_direction_line_moves_no_glyph; no font covers Arabic seen and lam");
        return;
    }
    let plain = layout(db, &TextObject::point(Point::ZERO, text, CharStyle::default()));
    assert!(plain.glyphs.iter().any(|g| g.rtl) && plain.glyphs.iter().any(|g| !g.rtl), "{plain:?}");
    let seen = text.find('س').unwrap_or_default();
    let mut restyled = TextObject::point(Point::ZERO, text, CharStyle::default());
    for byte in [1, seen] {
        let end = byte + text[byte..].chars().next().map_or(1, char::len_utf8);
        edit::style_range(&mut restyled.runs, byte, end, |s| s.fill = Default::default());
    }
    assert_eq!(restyled.runs.len(), 5);
    let restyled_layout = layout(db, &restyled);
    assert_same_shaped_glyphs(&plain, &restyled_layout);
    for glyph in &restyled_layout.glyphs {
        let run_ends = restyled.runs.iter().scan(0, |end, r| {
            *end += r.text.len();
            Some(*end)
        });
        let expected = run_ends.take_while(|end| *end <= glyph.byte).count();
        assert_eq!(glyph.run, expected, "glyph {glyph:?}");
    }
}

#[test]
fn shape_range_handles_empty_runs_and_selected_utf8_subranges() {
    let db = FontDb::global();
    let style = CharStyle::default();
    let text = "Aب";
    let runs = [(0..text.len(), &style)];
    let mut glyphs = Vec::new();

    shape::shape_range(db, text, 0..0, &runs, &[], &OtFeatures::default(), &[], &mut glyphs);
    shape::shape_range(db, text, 0..text.len(), &[], &[], &OtFeatures::default(), &[], &mut glyphs);
    shape::shape_range(db, text, 1..2, &runs, &[], &OtFeatures::default(), &[], &mut glyphs);
    assert!(glyphs.is_empty());

    shape::shape_range(db, text, 1..text.len(), &runs, &[], &OtFeatures::default(), &[], &mut glyphs);
    assert!(!glyphs.is_empty());
    assert!(glyphs.iter().all(|g| (1..text.len()).contains(&g.byte)));
}

#[test]
fn bidi_isolates_numbers_and_brackets_keep_valid_clusters() {
    for s in ["שלום (123)", "مرحبا (Rust 42)", "English \u{2067}שלום 123\u{2069} end"] {
        let l = text_layout(s);
        for g in &l.glyphs {
            assert!(s.get(g.byte..g.byte + g.len).is_some());
        }
        assert!(!selection_quads(&l, 0, s.len()).is_empty());
    }
}

#[test]
fn right_aligned_wrapped_rtl_keeps_trailing_spaces_outside_content() {
    let mut t = TextObject::point(Point::ZERO, "שלום עולם שלום עולם שלום עולם", CharStyle::default());
    t.kind = TextKind::Area { frame: vectorcraft_geom::PathData::from_bezpath(&Rect::new(0.0, 0.0, 80.0, 400.0).to_path(0.1)) };
    t.para.justify = vectorcraft_doc::Justify::Right;
    let l = layout(FontDb::global(), &t);
    assert!(l.lines.len() > 1);
    for line in &l.lines {
        assert!((line.x1 - 80.0).abs() < 1e-6, "{line:?}");
    }
}

#[test]
fn visual_arrows_cross_rtl_paragraph_boundaries() {
    let l = text_layout("שלום\nעולם");
    assert!(l.lines.iter().all(|l| l.rtl));
    assert_eq!(caret_horizontal(&l, 8, false), 9);
    assert_eq!(caret_horizontal(&l, 9, true), 8);
}

#[test]
fn automatic_alignment_follows_each_paragraph_and_explicit_choices_win() {
    let db = FontDb::global();
    for s in ["שלום", "مرحبا", "123 שלום"] {
        let mut t = TextObject::point(Point::ZERO, s, CharStyle::default());
        t.para.justify = vectorcraft_doc::Justify::Auto;
        let l = layout(db, &t);
        assert!((l.lines[0].x1).abs() < 1e-6, "RTL point type grows left from its anchor: {s}");
        t.kind = TextKind::Area { frame: vectorcraft_geom::PathData::from_bezpath(&Rect::new(0.0, 0.0, 200.0, 200.0).to_path(0.1)) };
        let l = layout(db, &t);
        assert!((l.lines[0].x1 - 200.0).abs() < 1e-6);
        t.para.justify = vectorcraft_doc::Justify::Left;
        assert_eq!(layout(db, &t).lines[0].x0, 0.0, "explicit left alignment is preserved");
        t.para.justify = vectorcraft_doc::Justify::Center;
        let l = layout(db, &t);
        assert!((l.lines[0].x0 + l.lines[0].x1 - 200.0).abs() < 1e-6);
    }
    let mut t = TextObject::point(Point::ZERO, "English\nשלום\nمرحبا\nEnglish again", CharStyle::default());
    t.para.justify = vectorcraft_doc::Justify::Auto;
    let l = layout(db, &t);
    assert_eq!(l.lines[0].x0, 0.0);
    assert!(l.lines[1].x0 < 0.0 && l.lines[1].x1.abs() < 1e-6);
    assert!(l.lines[2].x0 < 0.0 && l.lines[2].x1.abs() < 1e-6);
    assert_eq!(l.lines[3].x0, 0.0);
}

/// Paragraph Direction set on the text wins over its first strong character: English set right to
/// left keeps its words but ends at the start of its line (Auto alignment: the right), its full
/// stop on the left; Hebrew set left to right starts on the left.
#[test]
fn paragraph_direction_overrides_the_first_strong_character() {
    use vectorcraft_doc::{Justify, ParaDirection};
    let db = FontDb::global();
    let order = |s: &str, direction| {
        let mut t = TextObject::point(Point::ZERO, s, CharStyle::default());
        (t.para.justify, t.para.direction) = (Justify::Auto, direction);
        let l = layout(db, &t);
        let visible: String = l.glyphs.iter().filter_map(|g| s.get(g.byte..)?.chars().next()).collect();
        (visible, l.lines[0].rtl, l.lines[0].x1)
    };
    let (visible, rtl, x1) = order("Hello.", Some(ParaDirection::RightToLeft));
    assert_eq!((visible.as_str(), rtl), (".Hello", true));
    assert!(x1.abs() < 1e-6, "aligned to the right of the anchor: {x1}");
    assert_eq!(order("Hello.", None).0, "Hello.");
    assert!(!order("Hello.", None).1);
    assert_eq!(order("שלום abc", None).0, "abc םולש", "a Hebrew paragraph starts on the right");
    let (visible, rtl, _) = order("שלום abc", Some(ParaDirection::LeftToRight));
    assert_eq!((visible.as_str(), rtl), ("םולש abc", false), "set left to right, the Hebrew word comes first on the left");
    assert!(crate::paragraph_is_rtl("123 שלום", None) && !crate::paragraph_is_rtl("123 שלום", Some(ParaDirection::LeftToRight)));
}

/// Plain left-to-right text skips the bidi algorithm: nothing is marked right to left.
#[test]
fn left_to_right_text_is_untouched() {
    let l = text_layout("Plain text, 123 (and more).");
    assert!(l.glyphs.iter().all(|g| !g.rtl) && l.lines.iter().all(|l| !l.rtl));
    let bytes: Vec<_> = l.glyphs.iter().map(|g| g.byte).collect();
    assert!(bytes.windows(2).all(|w| w[0] < w[1]));
}

/// Text drawn in visual order (from a PDF) comes back in logical order, with the direction that
/// shows it as it was drawn; left-to-right text is left alone.
#[test]
fn visual_text_comes_back_in_logical_order() {
    let logical = |visual: &str| {
        let (order, rtl) = logical_order(visual)?;
        let chars: Vec<char> = visual.chars().collect();
        Some((order.iter().filter_map(|&i| chars.get(i)).collect::<String>(), rtl))
    };
    assert_eq!(logical("םולש"), Some(("שלום".to_string(), true)));
    assert_eq!(logical("123 םולש"), Some(("שלום 123".to_string(), true)));
    assert_eq!(logical("abc םולש"), Some(("abc שלום".to_string(), false)));
    assert_eq!(logical("plain text"), None);
    // Laid out with that direction, the logical text shows as drawn.
    for visual in ["123 םולש", "abc םולש"] {
        let (text, rtl) = logical(visual).unwrap();
        let mut t = TextObject::point(Point::ZERO, &text, CharStyle::default());
        t.para.direction = Some(if rtl { vectorcraft_doc::ParaDirection::RightToLeft } else { vectorcraft_doc::ParaDirection::LeftToRight });
        let l = layout(FontDb::global(), &t);
        let shown: String = l.glyphs.iter().filter_map(|g| text.get(g.byte..)?.chars().next()).collect();
        assert_eq!(shown, visual);
    }
}

/// Paragraph Direction is a paragraph attribute: in one text object a right-to-left paragraph
/// reads and aligns (Auto) right to left while the next, left to right, starts on the left.
#[test]
fn paragraph_direction_is_per_paragraph() {
    use vectorcraft_doc::{Justify, ParaDirection, ParaStyle};
    let s = "Hello.\nHello.";
    let mut t = TextObject::point(Point::ZERO, s, CharStyle::default());
    let dir = |direction| ParaStyle { justify: Justify::Auto, direction, ..ParaStyle::default() };
    t.set_paragraph_styles(vec![dir(Some(ParaDirection::RightToLeft)), dir(Some(ParaDirection::LeftToRight))]);
    assert_eq!(t.paras.len(), 2, "stored per paragraph");
    let l = layout(FontDb::global(), &t);
    assert_eq!(l.lines.len(), 2);
    let shown = |line: &crate::LineInfo| -> String {
        let mut gs: Vec<_> = l.glyphs.iter().filter(|g| g.byte >= line.start && g.byte < line.end).collect();
        gs.sort_by(|a, b| a.origin.x.total_cmp(&b.origin.x));
        gs.iter().filter_map(|g| s.get(g.byte..)?.chars().next()).filter(|c| *c != '\n').collect()
    };
    assert!(l.lines[0].rtl && !l.lines[1].rtl);
    assert_eq!(shown(&l.lines[0]), ".Hello", "the first paragraph runs right to left");
    assert_eq!(shown(&l.lines[1]), "Hello.", "the second runs left to right");
    assert!(l.lines[0].x1.abs() < 1e-6, "Auto aligns the right-to-left paragraph right: {}", l.lines[0].x1);
    assert!(l.lines[1].x0.abs() < 1e-6, "…and the left-to-right one left: {}", l.lines[1].x0);
}

#[test]
fn craft_arabic_fonts_load_and_noto_sans_arabic_is_the_arabic_fallback() {
    let arabic: Vec<_> = crate::CRAFT_FONTS.iter().filter(|f| f.scripts.contains(&"Arab")).collect();
    if arabic.is_empty() {
        eprintln!("skipping: built without craft-fonts (set CRAFT_FONTS_DIR to a craft-fonts checkout)");
        return;
    }
    let db = FontDb::with_font_dirs(vec![]);
    for f in &arabic {
        assert!(db.has_family(f.family), "{} {} isn't loaded", f.family, f.style);
    }
    let beh = db.face_covering('ب').expect("some face covers U+0628");
    assert_eq!(beh.family, "Noto Sans Arabic");
    // Latin still falls back to a bundled face, not to one of the Arabic families.
    let a = db.face_covering('a').expect("some face covers 'a'");
    assert!(!arabic.iter().any(|f| f.family == a.family), "Latin now falls back to {}", a.family);
}

/// Digits (Character panel menu): the style's numeral system draws every digit, whatever was
/// typed, while the text and its clusters stay as typed. Needs an installed font with the
/// Arabic-Indic digits.
#[test]
fn digits_draw_every_digit_in_the_styles_numeral_system() {
    use vectorcraft_doc::Digits;
    let db = FontDb::global();
    let Some(face) = ["Arial", "Noto Sans Arabic", "Geeza Pro", "Segoe UI", "DejaVu Sans"]
        .into_iter()
        .filter_map(|name| db.face(name, "Regular"))
        .find(|face| face.covers('3') && face.covers('٣') && face.covers('۳'))
    else {
        eprintln!("SKIPPED: digits_draw_every_digit_in_the_styles_numeral_system; no installed font covers 3, ٣ and ۳");
        return;
    };
    eprintln!("ASSERTIONS RUN: digits_draw_every_digit_in_the_styles_numeral_system; {} {}", face.family, face.style);
    let st = |digits: Digits| CharStyle { digits, ..arabic_style(&face) };
    let lay = |s: &str, digits: Digits| layout(db, &TextObject::point(Point::ZERO, s, st(digits)));
    let gids = |l: &TextLayout| l.glyphs.iter().map(|g| (g.gid, g.font_id)).collect::<Vec<_>>();
    let clusters = |l: &TextLayout| l.glyphs.iter().map(|g| (g.byte, g.len)).collect::<Vec<_>>();
    // Typed Western, drawn Arabic-Indic (Hindi) or Eastern Arabic-Indic (Farsi): the glyphs of
    // the typed Arabic-Indic digits, on the typed text's clusters.
    let typed = lay("page 3", Digits::Default);
    let hindi = lay("page 3", Digits::Hindi);
    assert_eq!(gids(&hindi), gids(&lay("page ٣", Digits::Default)));
    assert_ne!(gids(&hindi), gids(&typed));
    assert_eq!(clusters(&hindi), clusters(&typed));
    assert_eq!(gids(&lay("page 3", Digits::Farsi)), gids(&lay("page ۳", Digits::Default)));
    // Typed Arabic-Indic, drawn Western (Arabic): the Western glyphs on the 2-byte clusters.
    let arabic = lay("صفحة ٣", Digits::Arabic);
    assert_eq!(gids(&arabic), gids(&lay("صفحة 3", Digits::Default)));
    assert_eq!(clusters(&arabic), clusters(&lay("صفحة ٣", Digits::Default)));
    // Default leaves each digit as typed.
    assert_ne!(gids(&lay("3 ٣ ۳", Digits::Default)), gids(&lay("3 ٣ ۳", Digits::Hindi)));
}

/// Right-to-left type on a path that overflows shows the start of its text, as a line of area
/// type does, not whatever comes first in visual order.
#[test]
fn overflowing_rtl_type_on_a_path_keeps_the_start_of_its_text() {
    let s = "שלום עולם ".repeat(8);
    let mut t = TextObject::point(Point::ZERO, &s, CharStyle::default());
    let line = kurbo::Line::new((0.0, 0.0), (100.0, 0.0)).to_path(0.1);
    t.kind = TextKind::OnPath { path: vectorcraft_geom::PathData::from_bezpath(&line), start: 0.0, end: None };
    let l = layout(FontDb::global(), &t);
    assert!(l.overflow && !l.glyphs.is_empty());
    let shown: Vec<_> = source_clusters(&l).into_iter().map(|(byte, _)| byte).collect();
    let last = shown.last().copied().unwrap_or(0);
    let start: Vec<_> = s.char_indices().map(|(i, _)| i).take_while(|i| *i <= last).collect();
    assert_eq!(shown, start, "the glyphs shown are the text's start");
    let x = |byte: usize| l.glyphs.iter().find(|g| g.byte == byte).map(|g| g.origin.x).unwrap();
    assert!(x(0) > x(last), "the text runs right to left along the path");
}

/// On a curve with spacing, where a glyph's step depends on where it sits, what fits is measured
/// where the glyphs are placed: the start of the text shows, and every glyph of it.
#[test]
fn overflowing_rtl_type_on_a_spaced_curve_keeps_the_start_of_its_text() {
    let s = "שלום, עולם! אבג ".repeat(6);
    // Straight, then tightly curved: glyphs take other steps there.
    let mut path = BezPath::new();
    path.move_to((0.0, 0.0));
    path.line_to((60.0, 0.0));
    path.extend(kurbo::Arc::new((60.0, 12.0), (12.0, 12.0), -std::f64::consts::FRAC_PI_2, std::f64::consts::PI, 0.0).append_iter(0.1));
    let mut t = TextObject::point(Point::ZERO, &s, CharStyle::default());
    t.kind = TextKind::OnPath { path: vectorcraft_geom::PathData::from_bezpath(&path), start: 0.0, end: None };
    for spacing in (-12..=12).map(|i| f64::from(i) * 5.0) {
        t.path_spacing = spacing;
        let l = layout(FontDb::global(), &t);
        assert!(l.overflow && !l.glyphs.is_empty());
        let shown: Vec<_> = source_clusters(&l).into_iter().map(|(byte, _)| byte).collect();
        let last = shown.last().copied().unwrap_or(0);
        let start: Vec<_> = s.char_indices().map(|(i, _)| i).take_while(|i| *i <= last).collect();
        assert_eq!(shown, start, "the glyphs shown are the text's start ({spacing})");
    }
}

/// A left-to-right line with right-to-left text in it fits as it is laid out: a period that
/// hangs past the end bracket (Roman Hanging Punctuation) stays, and nothing overflows.
#[test]
fn mixed_type_on_a_path_hangs_its_final_period() {
    let s = "bא.";
    let line = kurbo::Line::new((0.0, 0.0), (400.0, 0.0)).to_path(0.1);
    let mut t = TextObject::point(Point::ZERO, s, CharStyle::default());
    t.kind = TextKind::OnPath { path: vectorcraft_geom::PathData::from_bezpath(&line), start: 0.0, end: None };
    t.para.roman_hanging = true;
    let advances: Vec<f64> = layout(FontDb::global(), &t).glyphs.iter().map(|g| g.advance).collect();
    assert_eq!(advances.len(), 3);
    // The end bracket halfway through the period, with path beyond it.
    let end = advances[0] + advances[1] + advances[2] * 0.5;
    t.kind = TextKind::OnPath { path: vectorcraft_geom::PathData::from_bezpath(&line), start: 0.0, end: Some(end / 400.0) };
    let l = layout(FontDb::global(), &t);
    assert!(l.glyphs.iter().any(|g| g.byte == s.len() - 1), "the period shows");
    assert!(!l.overflow, "all of it fits");
}
