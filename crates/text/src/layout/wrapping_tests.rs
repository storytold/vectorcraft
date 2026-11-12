use super::*;
use std::sync::Arc;

fn synthetic_glyphs(specs: &[(usize, usize, f64, char)]) -> Vec<SGlyph> {
    // The face is only a required SGlyph field here; advances and cluster boundaries below are
    // entirely synthetic, so the regressions don't depend on a particular font's shaping.
    static DB: std::sync::OnceLock<FontDb> = std::sync::OnceLock::new();
    let db = DB.get_or_init(|| FontDb::with_font_dirs(vec![]));
    let face = db.face("Source Sans 3", "Regular").expect("bundled test face");
    specs
        .iter()
        .map(|&(byte, len, adv, ch)| SGlyph {
            face: Arc::clone(&face),
            gid: 0,
            byte,
            len,
            run: 0,
            adv,
            tracking: 0.0,
            dx: 0.0,
            dy: 0.0,
            kern_shift: 0.0,
            sx: 1.0,
            sy: 1.0,
            bshift: 0.0,
            rotation: 0.0,
            ascent: 8.0,
            descent: 2.0,
            leading: 0.0,
            cap: 6.0,
            xh: 4.0,
            ch,
            tcy: None,
            inline: None,
            word_end: false,
            lead: 0.0,
            proportional: false,
            vpal: None,
            level: unicode_bidi::Level::ltr(),
            no_break: false,
            vertical_roman: false,
            cursive: false,
            hyphen_language: crate::hyphen::Language::English,
        })
        .collect()
}

fn source_range(glyphs: &[SGlyph], start: usize, end: usize) -> Range<usize> {
    let first = glyphs.get(start).expect("nonempty source range");
    let last = glyphs.get(end - 1).expect("nonempty source range");
    first.byte..last.byte + last.len
}

/// Both composers consume language-specific points even when a font subdivides vowel signs.
#[test]
fn indic_hyphenation_reaches_both_composers_without_splitting_marks() {
    let opts = HyphenationOptions { words_longer_than: 0, after_first: 1, before_last: 1, ..Default::default() };
    for (tag, syllable) in [
        ("hi", "कि"),
        ("mr", "कि"),
        ("gu", "કિ"),
        ("bn", "কি"),
        ("pa", "ਕਿ"),
        ("ta", "கி"),
        ("te", "కి"),
        ("kn", "ಕಿ"),
        ("ml", "കി"),
        ("or", "କି"),
        ("as", "কি"),
    ] {
        let text = syllable.repeat(8);
        // Deliberately subdivide each grapheme, as a fallback font may do. Marks are word
        // characters but must never become a break, even at a shaping cluster boundary.
        let specs: Vec<_> = text.char_indices().map(|(byte, ch)| (byte, ch.len_utf8(), 1.0, ch)).collect();
        let mut glyphs = synthetic_glyphs(&specs);
        for glyph in &mut glyphs {
            glyph.hyphen_language = crate::hyphen::Language::from_tag(Some(tag));
            glyph.sx = 0.001;
        }
        let cands = candidates(&text, &glyphs, Some(&opts), Kinsoku::None);
        assert_eq!(cands.iter().filter(|b| b.hyphen > 0.0).map(|b| b.end).collect::<Vec<_>>(), (2..16).step_by(2).collect::<Vec<_>>(), "{tag}");
        let (end, hyphen) = break_line(
            &text,
            &glyphs,
            0,
            4.5,
            &[],
            0.0,
            Some((&opts, 0.0)),
            Burasagari::None,
            Kinsoku::None,
            KinsokuShoriType::PushOutOnly,
            false,
            false,
            &[],
        );
        assert_eq!((end, hyphen), (4, true), "{tag}");
        let params = Params { justify_last: false, ragged: true, tolerance: 1.0, uniform_from: 0, hyphen_limit: 0 };
        let lines = compose(&glyphs, &|_| 4.5, &|_| 0.0, &|_| 0.0, &cands, &[], &params).expect("every-line composition");
        assert!(lines.iter().any(|(_, hyphen)| *hyphen), "{tag}");
        assert!(lines.iter().all(|(end, _)| end % 2 == 0), "{tag}: {lines:?}");
        let limited = Params { hyphen_limit: 1, ..params };
        if let Some(lines) = compose(&glyphs, &|_| 4.5, &|_| 0.0, &|_| 0.0, &cands, &[], &limited) {
            assert!(lines.windows(2).all(|pair| !pair[0].1 || !pair[1].1), "{tag}: {lines:?}");
        }
    }
}

#[test]
fn composition_trim_stays_along_the_line_when_a_character_rotates() {
    let db = FontDb::with_font_dirs(vec![]);
    let style = CharStyle::default();
    let opts = LayoutOptions::default();
    for angle in [0.0_f64, std::f64::consts::FRAC_PI_4, std::f64::consts::FRAC_PI_2] {
        let origin = Point::new(100.0, 80.0);
        let pre = Affine::translate(origin.to_vec2()) * Affine::rotate(angle);
        let draw = |rotation: f64, lead: f64| {
            let mut glyph = synthetic_glyphs(&[(0, 1, 30.0, 'A')]).remove(0);
            glyph.sx = 0.02;
            glyph.sy = 0.02;
            glyph.dx = 2.0;
            glyph.dy = -1.0;
            glyph.bshift = 4.0;
            glyph.kern_shift = 3.0;
            glyph.rotation = rotation;
            glyph.lead = lead;
            let mut cx = Ctx {
                on_path: angle != 0.0,
                db: &db,
                text: "A",
                runs: vec![(0..1, &style)],
                set_runs: Vec::new(),
                inlines: vec![None],
                faces: vec![std::cell::OnceCell::new()],
                default: style.clone(),
                opts: &opts,
                vertical: false,
                system_layout: false,
                fractional_widths: true,
                upright_cluster: None,
                tcy_block: None,
                out: TextLayout::default(),
            };
            cx.emit(&glyph, pre, origin, angle, 30.0 - lead, 0);
            cx.out.glyphs.remove(0)
        };
        let expected = Vec2::new(-angle.cos(), -angle.sin()) * 5.0;
        for rotation in [0.0, 45.0, 90.0, -90.0] {
            let (plain, trimmed) = (draw(rotation, 0.0), draw(rotation, 5.0));
            assert!(!trimmed.outline.elements().is_empty(), "fixture has drawn geometry");
            let shift = trimmed.xf * Point::ZERO - plain.xf * Point::ZERO;
            assert!((shift - expected).hypot() < 1e-9, "line {angle}, character {rotation}: {shift:?} versus {expected:?}");
            let pivot_shift = trimmed.xf * trimmed.pivot - plain.xf * plain.pivot;
            assert!((pivot_shift - expected).hypot() < 1e-9, "composition moves the turning point along the line: {pivot_shift:?}");
            let unrotated = draw(0.0, 5.0);
            let actual = (trimmed.xf * trimmed.unturned).as_coeffs();
            assert!(actual.iter().zip(unrotated.xf.as_coeffs()).all(|(a, b)| (a - b).abs() < 1e-9), "removing rotation keeps composition placement");
        }
    }
}

#[test]
fn greedy_wrap_keeps_an_oversized_first_cluster_and_its_source_range() {
    let text = "بَت";
    let glyphs = synthetic_glyphs(&[(0, 4, 2.0, 'ب'), (0, 4, 2.0, 'ب'), (4, 2, 1.0, 'ت')]);

    let (end, hyphenated) =
        break_line(text, &glyphs, 0, 3.0, &[], 0.0, None, Burasagari::None, Kinsoku::Hard, KinsokuShoriType::PushOutOnly, false, false, &[]);

    assert!(!hyphenated);
    assert_eq!(end, 2, "oversized base-plus-mark cluster must stay together");
    assert_eq!(source_range(&glyphs, 0, end), 0..4);
    assert_eq!(glyphs.get(end).map(|g| g.byte), Some(4));
}

#[test]
fn greedy_wrap_keeps_an_oversized_cluster_after_a_line_start_atomic() {
    let text = "بَتَث";
    let glyphs = synthetic_glyphs(&[(0, 4, 1.0, 'ب'), (4, 4, 2.0, 'ت'), (4, 4, 2.0, 'ت'), (8, 2, 1.0, 'ث')]);

    let (first_end, _) =
        break_line(text, &glyphs, 0, 3.0, &[], 0.0, None, Burasagari::None, Kinsoku::Hard, KinsokuShoriType::PushOutOnly, false, false, &[]);
    assert_eq!(first_end, 1);
    assert_eq!(source_range(&glyphs, 0, first_end), 0..4);

    let (second_end, hyphenated) =
        break_line(text, &glyphs, first_end, 3.0, &[], 0.0, None, Burasagari::None, Kinsoku::Hard, KinsokuShoriType::PushOutOnly, false, false, &[]);

    assert!(!hyphenated);
    assert_eq!(second_end, 3, "oversized cluster at the next line start must stay together");
    assert_eq!(source_range(&glyphs, first_end, second_end), 4..8);
    assert_eq!(glyphs.get(second_end).map(|g| g.byte), Some(8));
}

#[test]
fn narrow_arabic_area_wrap_keeps_base_and_mark_together_when_supported() {
    let db = FontDb::global();
    let Some(face) = db.face_covering('ب').filter(|face| face.covers('َ')) else {
        // Arabic integration coverage depends on an installed font; synthetic tests above are
        // the font-independent regression oracle.
        return;
    };
    let text = "بَت";
    let style = CharStyle { font_family: face.family.clone(), font_style: face.style.clone(), size: 20.0, ..CharStyle::default() };
    let bidi = para_bidi(text, None);
    let levels = bidi.as_ref().map_or(&[][..], |info| &info.levels);
    let mut shaped = Vec::new();
    shape_range(db, text, 0..text.len(), &[(0..text.len(), &style)], &[], &OtFeatures::default(), levels, &mut shaped);
    let Some(first) = shaped.first() else { return };
    let first_end = shaped.iter().take_while(|g| g.byte == first.byte).count();
    if first.byte != 0 || first_end < 2 || first_end >= shaped.len() {
        return;
    }
    // Pick a narrow width that fits glyphs before one positive-advance glyph but not that next
    // glyph. This ensures the old per-glyph loop would have returned inside the first cluster.
    let mut prefix = shaped.first().map_or(0.0, |g| g.adv);
    let width = (1..first_end).find_map(|k| {
        let advance = shaped.get(k)?.adv;
        if prefix > EPS && advance > EPS {
            Some(prefix)
        } else {
            prefix += advance;
            None
        }
    });
    let Some(width) = width else { return };

    let mut object = TextObject::point(Point::ZERO, text, style);
    object.kind = TextKind::Area { frame: PathData::from_bezpath(&Rect::new(0.0, 0.0, width, 100.0).to_path(0.1)) };
    let result = layout_with(db, &object, &LayoutOptions::default());

    let next_cluster_byte = shaped.get(first_end).map(|g| g.byte);
    assert_eq!(result.lines.first().map(|line| line.end), next_cluster_byte);
}

/// No Break keeps the marked characters on one line, including across a space, and moves a
/// marked run that does not fit to the next line whole.
#[test]
fn no_break_keeps_marked_characters_on_one_line() {
    let text = "A B";
    let plain = synthetic_glyphs(&[(0, 1, 10.0, 'A'), (1, 1, 4.0, ' '), (2, 1, 10.0, 'B')]);
    let (end, hyphen) =
        break_line(text, &plain, 0, 16.0, &[], 0.0, None, Burasagari::None, Kinsoku::None, KinsokuShoriType::PushOutOnly, false, false, &[]);
    assert!(!hyphen);
    assert_eq!(end, 2, "without the mark the line ends after the space");

    let mut marked = plain;
    for g in &mut marked {
        g.no_break = true;
    }
    let (end, hyphen) =
        break_line(text, &marked, 0, 16.0, &[], 0.0, None, Burasagari::None, Kinsoku::None, KinsokuShoriType::PushOutOnly, false, false, &[]);
    assert!(!hyphen);
    assert_eq!(end, 3, "the marked words stay together even though they are wider than the line");

    // 日 and 本 may break between them. Marked, they move together when they do not fit after "A ".
    let text = "A 日本";
    let mut glyphs = synthetic_glyphs(&[(0, 1, 8.0, 'A'), (1, 1, 2.0, ' '), (2, 3, 10.0, '日'), (5, 3, 10.0, '本')]);
    glyphs[2].no_break = true;
    glyphs[3].no_break = true;
    let (end, _) =
        break_line(text, &glyphs, 0, 22.0, &[], 0.0, None, Burasagari::None, Kinsoku::None, KinsokuShoriType::PushOutOnly, false, false, &[]);
    assert_eq!(end, 2, "the marked pair moves to the next line together");
    let (end, _) =
        break_line(text, &glyphs, end, 15.0, &[], 0.0, None, Burasagari::None, Kinsoku::None, KinsokuShoriType::PushOutOnly, false, false, &[]);
    assert_eq!(end, 4, "on its own line the pair stays together even when it is wider than the line");

    let breaks = candidates(text, &glyphs, None, Kinsoku::None);
    assert!(breaks.iter().all(|b| b.end != 3), "no break between the marked characters: {breaks:?}");
}

/// A soft hyphen inside a No Break run is not a place a line may break, so the word is measured
/// across it, as wide as [`break_line`] keeps it.
#[test]
fn word_width_measures_across_a_soft_hyphen_inside_a_no_break_run() {
    let mut glyphs = synthetic_glyphs(&[(0, 1, 10.0, 'A'), (1, 2, 0.0, '\u{00ad}'), (3, 1, 10.0, 'B'), (4, 1, 10.0, 'C')]);
    // The hyphen a soft hyphen adds comes from the face, scaled to well under 10 pt.
    for g in &mut glyphs {
        g.sx = 0.001;
    }
    let plain = breaking::word_width(&glyphs, 0, &[], Kinsoku::None);
    assert!(plain < 30.0, "without the mark the word ends at the soft hyphen: {plain}");
    for g in &mut glyphs {
        g.no_break = true;
    }
    let marked = breaking::word_width(&glyphs, 0, &[], Kinsoku::None);
    assert!((marked - 30.0).abs() < 1e-9, "the marked run is measured whole: {marked}");
}

/// A forced line break ends the word [`breaking::word_width`] measures, inside a No Break run too,
/// and a soft hyphen just before it adds no hyphen, as [`break_line`] sets the line.
#[test]
fn word_width_stops_at_a_forced_line_break() {
    let mut glyphs = synthetic_glyphs(&[(0, 1, 10.0, 'A'), (1, 3, 5.0, '\u{2028}'), (4, 1, 10.0, 'B'), (5, 1, 10.0, 'C')]);
    for g in &mut glyphs {
        g.no_break = true;
    }
    let marked = breaking::word_width(&glyphs, 0, &[], Kinsoku::None);
    assert!((marked - 10.0).abs() < 1e-9, "the marked run ends at the forced line break: {marked}");

    let mut glyphs = synthetic_glyphs(&[(0, 1, 10.0, 'A'), (1, 2, 0.0, '\u{00ad}'), (3, 3, 5.0, '\u{2028}'), (6, 1, 10.0, 'B')]);
    // The hyphen a soft hyphen adds comes from the face, scaled to well under 10 pt.
    for g in &mut glyphs {
        g.sx = 0.001;
    }
    let plain = breaking::word_width(&glyphs, 0, &[], Kinsoku::None);
    assert!((plain - 10.0).abs() < 1e-9, "no hyphen before the forced line break: {plain}");
    for g in &mut glyphs {
        g.no_break = true;
    }
    let marked = breaking::word_width(&glyphs, 0, &[], Kinsoku::None);
    assert!((marked - 10.0).abs() < 1e-9, "the marked run ends at the forced line break: {marked}");
}

/// Synthetic full-width characters `text`, each 10 pt wide on a 10 pt em.
fn full_width(text: &str) -> Vec<SGlyph> {
    let specs: Vec<(usize, usize, f64, char)> = text.char_indices().map(|(byte, ch)| (byte, ch.len_utf8(), 10.0, ch)).collect();
    let mut glyphs = synthetic_glyphs(&specs);
    for g in &mut glyphs {
        g.sx = 10.0 / g.face.units_per_em();
        g.sy = g.sx;
    }
    glyphs
}

/// Kinsoku Shori Type, in a measure of 15 full-width characters: a closing bracket that would
/// start the next line is pushed in onto the line with Push In First, and the character before
/// it is pushed out with it with Push Out First and Push Out Only. Push Out First pushes in when
/// pushing out would leave the line two characters short (an opening bracket goes too). A line
/// that cannot close up enough pushes out, and with Kinsoku None the types do nothing.
#[test]
fn kinsoku_shori_type_pushes_a_closing_bracket_in_or_out() {
    use KinsokuShoriType::{PushInFirst, PushOutFirst, PushOutOnly};
    let hung = |text: &str, width: f64, kinsoku: Kinsoku, ty: KinsokuShoriType, hanging: bool| {
        break_line(text, &full_width(text), 0, width, &[], 0.0, None, Burasagari::None, kinsoku, ty, hanging, false, &[]).0
    };
    let end = |text: &str, width: f64, kinsoku: Kinsoku, ty: KinsokuShoriType| hung(text, width, kinsoku, ty, false);
    let text = "一二三四五六七八九十一二三四五」六";
    assert_eq!(end(text, 150.0, Kinsoku::Hard, PushInFirst), 16, "」 is pushed in");
    assert_eq!(end(text, 150.0, Kinsoku::Hard, PushOutFirst), 14, "五 is pushed out with 」");
    assert_eq!(end(text, 150.0, Kinsoku::Hard, PushOutOnly), 14, "五 is pushed out with 」");
    assert_eq!(end(text, 150.0, Kinsoku::Soft, PushInFirst), 16, "Soft keeps 」 off the line start too");
    assert_eq!(end(text, 150.0, Kinsoku::None, PushInFirst), 15, "no kinsoku: 」 starts the next line");

    let bracketed = "一二三四五六七八九十一二三「四」六";
    assert_eq!(end(bracketed, 150.0, Kinsoku::Hard, PushOutFirst), 16, "pushing out 「四 would leave two ems");
    assert_eq!(end(bracketed, 150.0, Kinsoku::Hard, PushInFirst), 16);
    assert_eq!(end(bracketed, 150.0, Kinsoku::Hard, PushOutOnly), 13);

    // Latin brackets and quotation marks are pushed in too: an opening one that would end the
    // line, and a closing one after a Latin letter that would start the next.
    for opening in ["一二三四五六七八九十一二三四(五六", "一二三四五六七八九十一二三四\u{201c}五六"] {
        assert_eq!(end(opening, 150.0, Kinsoku::Hard, PushInFirst), 16, "{opening}: the bracket and 五 are pushed in");
        assert_eq!(end(opening, 150.0, Kinsoku::Hard, PushOutOnly), 14, "{opening}: the bracket is pushed out with 五");
    }
    let latin = "一二三四五六七八九十一二三四a)";
    assert_eq!(end(latin, 150.0, Kinsoku::Hard, PushInFirst), 16, ") is pushed in");
    assert_eq!(end(latin, 150.0, Kinsoku::Hard, PushOutOnly), 14, "a is pushed out with )");
    // A push-in does not end at a soft hyphen: taking `b` and the soft hyphen in would fit, the
    // whole word `bc` does not, so the line pushes out.
    let shy = format!("{}a)b\u{ad}c", "一".repeat(31));
    assert_eq!(end(&shy, 320.0, Kinsoku::Hard, PushInFirst), 31);
    assert_eq!(end(&shy, 320.0, Kinsoku::Hard, PushOutOnly), 31);
    // Nor at a space after an opening bracket, which would leave the bracket ending the line.
    let spaced = format!("{}「「 A", "一".repeat(14));
    assert_eq!(end(&spaced, 150.0, Kinsoku::Hard, PushInFirst), 14);
    assert_eq!(end(&spaced, 150.0, Kinsoku::Hard, PushOutOnly), 14);
    // With hanging punctuation on, the gap before a full stop that may hang at the line's end is
    // not counted: the other nine gaps close by 9 pt, short of the 10 pt the full stop needs. With
    // it off, all ten gaps close and the full stop is pushed in.
    let stop = format!("{}五.", "一".repeat(9));
    assert_eq!(hung(&stop, 100.0, Kinsoku::Hard, PushInFirst, true), 9);
    assert_eq!(hung(&stop, 100.0, Kinsoku::Hard, PushOutOnly, true), 9);
    assert_eq!(end(&stop, 100.0, Kinsoku::Hard, PushInFirst), 11);
    assert_eq!(end(&stop, 100.0, Kinsoku::Hard, PushOutOnly), 9);

    // Two gaps close by at most 2 pt, not the 5 pt 」 needs.
    let short = "一二」三";
    assert_eq!(end(short, 25.0, Kinsoku::Hard, PushInFirst), 1, "too little to close up: 二 is pushed out");

    // A line holding a tab pushes out. Here the tab reaches the default stop at 36 pt, so pushing
    // out leaves the line one em short, though the tab glyph's own advance is 10 pt.
    let tabbed = "一\t二三四五六七八九十一二」六";
    assert_eq!(end(tabbed, 146.0, Kinsoku::Hard, PushOutOnly), 12);
    assert_eq!(end(tabbed, 146.0, Kinsoku::Hard, PushOutFirst), 12);
    assert_eq!(end(tabbed, 146.0, Kinsoku::Hard, PushInFirst), 12);
}

/// Kinsoku Shori Type counts only the gaps justification closes: a line holding right-to-left
/// text pushes out, the gaps close by a tenth of the em at the characters' Horizontal Scale, and
/// no gap closes inside a shaped cluster.
#[test]
fn kinsoku_shori_type_counts_the_gaps_that_close() {
    use KinsokuShoriType::{PushInFirst, PushOutOnly};
    // A line holding right-to-left text is reordered before it is drawn, so the gaps that close
    // differ from the ones counted here: it pushes out.
    let mixed = "一二三א五六七八九十一二三四五」六";
    let mut glyphs = full_width(mixed);
    glyphs[3].level = unicode_bidi::Level::rtl();
    let mixed_end =
        |ty: KinsokuShoriType| break_line(mixed, &glyphs, 0, 150.0, &[], 0.0, None, Burasagari::None, Kinsoku::Hard, ty, false, false, &[]).0;
    assert_eq!(mixed_end(PushInFirst), 14, "right-to-left text on the line: 五 is pushed out with 」");
    assert_eq!(mixed_end(PushOutOnly), 14);

    // The gaps close by a tenth of the em at the characters' Horizontal Scale: at 200% Vertical
    // Scale five gaps still close by 5 pt, short of the 10 pt 」 needs.
    let tall = "一二三四五」六";
    let mut glyphs = full_width(tall);
    for g in &mut glyphs {
        g.sy *= 2.0;
    }
    let tall_end =
        |ty: KinsokuShoriType| break_line(tall, &glyphs, 0, 50.0, &[], 0.0, None, Burasagari::None, Kinsoku::Hard, ty, false, false, &[]).0;
    assert_eq!(tall_end(PushInFirst), 4, "Vertical Scale does not widen the gaps: 五 is pushed out with 」");

    // No gap closes inside a shaped cluster. 二 is two glyphs here, so the line has five gaps to
    // close by 5 pt, short of the 6 pt it overflows by.
    let mut glyphs = full_width(tall);
    glyphs[1].adv = 5.0;
    let half = glyphs[1].clone();
    glyphs.insert(2, half);
    let split_end =
        |width: f64| break_line(tall, &glyphs, 0, width, &[], 0.0, None, Burasagari::None, Kinsoku::Hard, PushInFirst, false, false, &[]).0;
    assert_eq!(split_end(54.0), 5, "五 is pushed out with 」");
    assert_eq!(split_end(55.0), 7, "five gaps close by 5 pt: 」 is pushed in");
}

/// Mojikumi: the greedy breaker measures a line as it is set, with its last character narrowed
/// (`line_end`): 」 set half width ends a measure of three and a half characters. Only the last
/// glyph of the last cluster counts, and only where the line may end.
#[test]
fn greedy_wrap_counts_the_narrower_line_end() {
    use KinsokuShoriType::{PushInFirst, PushOutFirst, PushOutOnly};
    let text = "一二三」四五六";
    let glyphs = full_width(text);
    let mut credit = vec![0.0; glyphs.len()];
    credit[3] = 5.0;
    let end = |glyphs: &[SGlyph], width: f64, burasagari: Burasagari, line_end: &[f64]| {
        break_line(text, glyphs, 0, width, &[], 0.0, None, burasagari, Kinsoku::Hard, PushOutOnly, false, false, line_end).0
    };
    assert_eq!(end(&glyphs, 35.0, Burasagari::None, &credit), 4, "」 half width fits");
    assert_eq!(end(&glyphs, 35.0, Burasagari::None, &[]), 2, "」 full width does not: 三 goes with it");
    assert_eq!(end(&glyphs, 34.0, Burasagari::None, &credit), 2, "half width is still too wide");
    // A credit that isn't finite counts for nothing, and one larger than the glyph for its advance.
    assert_eq!(end(&glyphs, 35.0, Burasagari::None, &[0.0, 0.0, 0.0, f64::INFINITY]), 2);
    assert_eq!(end(&glyphs, 35.0, Burasagari::None, &[0.0, 0.0, 0.0, f64::NAN]), 2);
    assert_eq!(end(&glyphs, 30.0, Burasagari::None, &[0.0, 0.0, 0.0, 100.0]), 4, "at most 10 pt off");
    assert_eq!(end(&glyphs, 29.0, Burasagari::None, &[0.0, 0.0, 0.0, 100.0]), 2);

    // The line may not end after 三 (、 follows it): its credit does not keep it on the line, so 、
    // does not hang after a line already wider than the measure.
    let comma = "一二三、";
    let glyphs = full_width(comma);
    let end = |line_end: &[f64]| {
        break_line(comma, &glyphs, 0, 25.0, &[], 0.0, None, Burasagari::Standard, Kinsoku::Hard, PushOutOnly, false, false, line_end).0
    };
    assert_eq!(end(&[0.0, 0.0, 5.0, 0.0]), 2);

    // Kinsoku Shori Type measures both lines as they are set. Pushing 。 in leaves the line 5 pt
    // wider than the measure with 。 half width, which its five gaps close.
    let pushed = "一二三四」。六";
    let glyphs = full_width(pushed);
    let end = |ty: KinsokuShoriType, line_end: &[f64]| {
        break_line(pushed, &glyphs, 0, 50.0, &[], 0.0, None, Burasagari::None, Kinsoku::Hard, ty, false, false, line_end).0
    };
    let stop = [0.0, 0.0, 0.0, 0.0, 0.0, 5.0, 0.0];
    assert_eq!(end(PushInFirst, &stop), 6, "。 is pushed in");
    assert_eq!(end(PushInFirst, &[]), 3, "full width, the gaps can't close enough");
    // Push Out First: pushing out to after 」, set half width, leaves the line 15 pt short, more
    // than an em, so 四 is pushed in.
    let pushed = format!("{}二」「四", "一".repeat(10));
    let glyphs = full_width(&pushed);
    let end = |line_end: &[f64]| {
        break_line(&pushed, &glyphs, 0, 130.0, &[], 0.0, None, Burasagari::None, Kinsoku::Hard, PushOutFirst, false, false, line_end).0
    };
    let mut bracket = vec![0.0; glyphs.len()];
    bracket[11] = 5.0;
    assert_eq!(end(&bracket), 14, "四 is pushed in");
    assert_eq!(end(&[]), 12, "full width, pushing out leaves the line one em short: 「四 go on");

    // 三 is two glyphs of one cluster: the credit of its first glyph does not count.
    let text = "一二三";
    let mut glyphs = full_width(text);
    glyphs[2].adv = 5.0;
    let half = glyphs[2].clone();
    glyphs.insert(3, half);
    let end =
        |line_end: &[f64]| break_line(text, &glyphs, 0, 26.0, &[], 0.0, None, Burasagari::None, Kinsoku::None, PushOutOnly, false, false, line_end).0;
    assert_eq!(end(&[0.0, 0.0, 4.0, 0.0]), 2, "the cluster stays whole and goes to the next line");
    assert_eq!(end(&[0.0, 0.0, 0.0, 4.0]), 4, "its last glyph's credit keeps it on the line");
}

/// Mojikumi: the every-line composer measures each candidate line with its last character narrowed
/// (`end_credit`), never by more than that glyph's advance, and a line ending in a hyphen ends with
/// the hyphen.
#[test]
fn composition_counts_the_narrower_line_end() {
    let text = "一二三」四五六";
    let glyphs = full_width(text);
    let cands = candidates(text, &glyphs, None, Kinsoku::Hard);
    let params = Params { justify_last: false, ragged: true, tolerance: f64::INFINITY, uniform_from: 0, hyphen_limit: 0 };
    let lines =
        |width: f64, end_credit: &dyn Fn(usize) -> f64, cands: &[Breakpoint]| compose(&glyphs, &|_| width, &|_| 0.0, end_credit, cands, &[], &params);
    let half = |t: usize| if t == 4 { 5.0 } else { 0.0 };
    assert_eq!(lines(35.0, &half, &cands), Some(vec![(4, false), (7, false)]));
    let solid = lines(35.0, &|_| 0.0, &cands).expect("composed");
    assert!(solid.iter().all(|&(end, _)| end != 4), "」 full width does not end the first line: {solid:?}");
    // At most the glyph's advance; a credit that isn't finite counts for nothing.
    assert_eq!(lines(30.0, &|t| if t == 4 { 100.0 } else { 0.0 }, &cands).and_then(|l| l.first().copied()), Some((4, false)));
    assert!(lines(35.0, &|_| f64::NAN, &cands).is_some_and(|l| l.iter().all(|&(end, _)| end != 4)));
    // No credit for a line ending in a hyphen.
    let hyphenated = [Breakpoint { end: 4, hyphen: 1.0 }];
    assert_eq!(lines(38.0, &half, &hyphenated), None);
    assert_eq!(lines(38.0, &half, &[Breakpoint { end: 4, hyphen: 0.0 }]), Some(vec![(4, false), (7, false)]));
}
