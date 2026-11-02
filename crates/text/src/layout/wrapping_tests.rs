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
            dx: 0.0,
            dy: 0.0,
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
        })
        .collect()
}

fn source_range(glyphs: &[SGlyph], start: usize, end: usize) -> Range<usize> {
    let first = glyphs.get(start).expect("nonempty source range");
    let last = glyphs.get(end - 1).expect("nonempty source range");
    first.byte..last.byte + last.len
}

#[test]
fn greedy_wrap_keeps_an_oversized_first_cluster_and_its_source_range() {
    let text = "بَت";
    let glyphs = synthetic_glyphs(&[(0, 4, 2.0, 'ب'), (0, 4, 2.0, 'ب'), (4, 2, 1.0, 'ت')]);

    let (end, hyphenated) =
        break_line(text, &glyphs, 0, 3.0, &[], 0.0, None, Burasagari::None, Kinsoku::Hard, KinsokuShoriType::PushOutOnly, false, false);

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
        break_line(text, &glyphs, 0, 3.0, &[], 0.0, None, Burasagari::None, Kinsoku::Hard, KinsokuShoriType::PushOutOnly, false, false);
    assert_eq!(first_end, 1);
    assert_eq!(source_range(&glyphs, 0, first_end), 0..4);

    let (second_end, hyphenated) =
        break_line(text, &glyphs, first_end, 3.0, &[], 0.0, None, Burasagari::None, Kinsoku::Hard, KinsokuShoriType::PushOutOnly, false, false);

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
        break_line(text, &plain, 0, 16.0, &[], 0.0, None, Burasagari::None, Kinsoku::None, KinsokuShoriType::PushOutOnly, false, false);
    assert!(!hyphen);
    assert_eq!(end, 2, "without the mark the line ends after the space");

    let mut marked = plain;
    for g in &mut marked {
        g.no_break = true;
    }
    let (end, hyphen) =
        break_line(text, &marked, 0, 16.0, &[], 0.0, None, Burasagari::None, Kinsoku::None, KinsokuShoriType::PushOutOnly, false, false);
    assert!(!hyphen);
    assert_eq!(end, 3, "the marked words stay together even though they are wider than the line");

    // 日 and 本 may break between them. Marked, they move together when they do not fit after "A ".
    let text = "A 日本";
    let mut glyphs = synthetic_glyphs(&[(0, 1, 8.0, 'A'), (1, 1, 2.0, ' '), (2, 3, 10.0, '日'), (5, 3, 10.0, '本')]);
    glyphs[2].no_break = true;
    glyphs[3].no_break = true;
    let (end, _) = break_line(text, &glyphs, 0, 22.0, &[], 0.0, None, Burasagari::None, Kinsoku::None, KinsokuShoriType::PushOutOnly, false, false);
    assert_eq!(end, 2, "the marked pair moves to the next line together");
    let (end, _) = break_line(text, &glyphs, end, 15.0, &[], 0.0, None, Burasagari::None, Kinsoku::None, KinsokuShoriType::PushOutOnly, false, false);
    assert_eq!(end, 4, "on its own line the pair stays together even when it is wider than the line");

    let breaks = candidates(text, &glyphs, None, Kinsoku::None);
    assert!(breaks.iter().all(|b| b.end != 3), "no break between the marked characters: {breaks:?}");
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
        break_line(text, &full_width(text), 0, width, &[], 0.0, None, Burasagari::None, kinsoku, ty, hanging, false).0
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
    let mixed_end = |ty: KinsokuShoriType| break_line(mixed, &glyphs, 0, 150.0, &[], 0.0, None, Burasagari::None, Kinsoku::Hard, ty, false, false).0;
    assert_eq!(mixed_end(PushInFirst), 14, "right-to-left text on the line: 五 is pushed out with 」");
    assert_eq!(mixed_end(PushOutOnly), 14);

    // The gaps close by a tenth of the em at the characters' Horizontal Scale: at 200% Vertical
    // Scale five gaps still close by 5 pt, short of the 10 pt 」 needs.
    let tall = "一二三四五」六";
    let mut glyphs = full_width(tall);
    for g in &mut glyphs {
        g.sy *= 2.0;
    }
    let tall_end = |ty: KinsokuShoriType| break_line(tall, &glyphs, 0, 50.0, &[], 0.0, None, Burasagari::None, Kinsoku::Hard, ty, false, false).0;
    assert_eq!(tall_end(PushInFirst), 4, "Vertical Scale does not widen the gaps: 五 is pushed out with 」");

    // No gap closes inside a shaped cluster. 二 is two glyphs here, so the line has five gaps to
    // close by 5 pt, short of the 6 pt it overflows by.
    let mut glyphs = full_width(tall);
    glyphs[1].adv = 5.0;
    let half = glyphs[1].clone();
    glyphs.insert(2, half);
    let split_end = |width: f64| break_line(tall, &glyphs, 0, width, &[], 0.0, None, Burasagari::None, Kinsoku::Hard, PushInFirst, false, false).0;
    assert_eq!(split_end(54.0), 5, "五 is pushed out with 」");
    assert_eq!(split_end(55.0), 7, "five gaps close by 5 pt: 」 is pushed in");
}
