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
        })
        .collect()
}

fn source_range(glyphs: &[SGlyph], start: usize, end: usize) -> Range<usize> {
    let first = glyphs.get(start).expect("nonempty source range");
    let last = glyphs.get(end - 1).expect("nonempty source range");
    first.byte..last.byte + last.len
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
                inlines: vec![None],
                default: style.clone(),
                opts: &opts,
                vertical: false,
                system_layout: false,
                fractional_widths: true,
                upright_cluster: None,
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

    let (end, hyphenated) = break_line(text, &glyphs, 0, 3.0, &[], 0.0, None, Burasagari::None, Kinsoku::Hard);

    assert!(!hyphenated);
    assert_eq!(end, 2, "oversized base-plus-mark cluster must stay together");
    assert_eq!(source_range(&glyphs, 0, end), 0..4);
    assert_eq!(glyphs.get(end).map(|g| g.byte), Some(4));
}

#[test]
fn greedy_wrap_keeps_an_oversized_cluster_after_a_line_start_atomic() {
    let text = "بَتَث";
    let glyphs = synthetic_glyphs(&[(0, 4, 1.0, 'ب'), (4, 4, 2.0, 'ت'), (4, 4, 2.0, 'ت'), (8, 2, 1.0, 'ث')]);

    let (first_end, _) = break_line(text, &glyphs, 0, 3.0, &[], 0.0, None, Burasagari::None, Kinsoku::Hard);
    assert_eq!(first_end, 1);
    assert_eq!(source_range(&glyphs, 0, first_end), 0..4);

    let (second_end, hyphenated) = break_line(text, &glyphs, first_end, 3.0, &[], 0.0, None, Burasagari::None, Kinsoku::Hard);

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
    let (end, hyphen) = break_line(text, &plain, 0, 16.0, &[], 0.0, None, Burasagari::None, Kinsoku::None);
    assert!(!hyphen);
    assert_eq!(end, 2, "without the mark the line ends after the space");

    let mut marked = plain;
    for g in &mut marked {
        g.no_break = true;
    }
    let (end, hyphen) = break_line(text, &marked, 0, 16.0, &[], 0.0, None, Burasagari::None, Kinsoku::None);
    assert!(!hyphen);
    assert_eq!(end, 3, "the marked words stay together even though they are wider than the line");

    // 日 and 本 may break between them. Marked, they move together when they do not fit after "A ".
    let text = "A 日本";
    let mut glyphs = synthetic_glyphs(&[(0, 1, 8.0, 'A'), (1, 1, 2.0, ' '), (2, 3, 10.0, '日'), (5, 3, 10.0, '本')]);
    glyphs[2].no_break = true;
    glyphs[3].no_break = true;
    let (end, _) = break_line(text, &glyphs, 0, 22.0, &[], 0.0, None, Burasagari::None, Kinsoku::None);
    assert_eq!(end, 2, "the marked pair moves to the next line together");
    let (end, _) = break_line(text, &glyphs, end, 15.0, &[], 0.0, None, Burasagari::None, Kinsoku::None);
    assert_eq!(end, 4, "on its own line the pair stays together even when it is wider than the line");

    let breaks = candidates(text, &glyphs, None, Kinsoku::None);
    assert!(breaks.iter().all(|b| b.end != 3), "no break between the marked characters: {breaks:?}");
}
