use super::*;

/// Character Direction is saved only when set, and styles written before it existed read as
/// Default.
#[test]
fn character_direction_is_saved_only_when_set() {
    let st = CharStyle::default();
    assert_eq!(st.char_direction, None);
    assert!(serde_json::to_value(&st).unwrap().get("charDirection").is_none());
    for (d, key) in [(ParaDirection::LeftToRight, "leftToRight"), (ParaDirection::RightToLeft, "rightToLeft")] {
        let set = CharStyle { char_direction: Some(d), ..CharStyle::default() };
        assert_eq!(serde_json::to_value(&set).unwrap()["charDirection"], key);
        assert_eq!(serde_json::from_value::<CharStyle>(serde_json::to_value(&set).unwrap()).unwrap(), set);
    }
    let old = serde_json::to_value(&st).unwrap();
    assert_eq!(serde_json::from_value::<CharStyle>(old).unwrap().char_direction, None);
}

/// Ruby saves with the type and a file saved before it (no `ruby` key) reads with none.
#[test]
fn ruby_is_saved_and_missing_ruby_reads_as_none() {
    let mut t = TextObject::point(Point::ZERO, "漢字", CharStyle::default());
    t.ruby = vec![RubySpan { size_percent: 40.0, overhang: false, ..RubySpan::new(0, 6, "かんじ") }];
    let json = serde_json::to_string(&t).unwrap();
    assert!(json.contains("かんじ"));
    let back: TextObject = serde_json::from_str(&json).unwrap();
    assert_eq!(back.ruby, t.ruby);
    assert!((back.ruby[0].size_percent - 40.0).abs() < 1e-9 && !back.ruby[0].overhang);
    // Old files have no `ruby` key: none.
    let mut old: serde_json::Value = serde_json::from_str(&json).unwrap();
    old.as_object_mut().unwrap().remove("ruby");
    assert!(serde_json::from_value::<TextObject>(old).unwrap().ruby.is_empty());
    // A damaged file's range past the text and its empty reading are clamped away.
    let mut damaged = t.clone();
    damaged.ruby = vec![RubySpan::new(0, 10_000, "かんじ"), RubySpan::new(3, 2, " "), RubySpan::new(0, 3, "")];
    damaged.normalize_ruby();
    assert_eq!((damaged.ruby[0].start, damaged.ruby[0].end), (0, 6));
}

/// Digits map every digit of the three sets into the chosen one, leave the rest alone, and
/// are saved only when set.
#[test]
fn digits_map_between_the_numeral_systems() {
    let map = |d: Digits, s: &str| -> String { s.chars().map(|c| d.map(c)).collect() };
    for s in ["0123456789", "٠١٢٣٤٥٦٧٨٩", "۰۱۲۳۴۵۶۷۸۹"] {
        assert_eq!(map(Digits::Default, s), s);
        assert_eq!(map(Digits::Arabic, s), "0123456789");
        assert_eq!(map(Digits::Hindi, s), "٠١٢٣٤٥٦٧٨٩");
        assert_eq!(map(Digits::Farsi, s), "۰۱۲۳۴۵۶۷۸۹");
    }
    // Letters, punctuation and other scripts' digits (Devanagari ३) are not digits of these sets.
    assert_eq!(map(Digits::Hindi, "a-Z ३ ٪ ۔"), "a-Z ३ ٪ ۔");
    let st = CharStyle::default();
    assert!(serde_json::to_value(&st).unwrap().get("digits").is_none());
    let farsi = CharStyle { digits: Digits::Farsi, ..CharStyle::default() };
    assert_eq!(serde_json::to_value(&farsi).unwrap()["digits"], "farsi");
    // Styles written before the attribute existed (no `digits` key) read as Default.
    let old = serde_json::to_value(&st).unwrap();
    assert_eq!(serde_json::from_value::<CharStyle>(old).unwrap().digits, Digits::Default);
    assert_eq!(serde_json::from_value::<CharStyle>(serde_json::to_value(&farsi).unwrap()).unwrap(), farsi);
}

/// Auto leading follows the paragraph's Auto Leading; a set leading doesn't.
#[test]
fn paragraph_effective_leading_follows_auto_leading() {
    let st = CharStyle { size: 10.0, leading: None, ..CharStyle::default() };
    let mut para = ParaStyle::default();
    assert_eq!(para.effective_leading(&st), st.effective_leading(), "default 120%");
    para.justification.auto_leading = 200.0;
    assert_eq!(para.effective_leading(&st), 20.0);
    para.justification.auto_leading = 0.0;
    assert_eq!(para.effective_leading(&st), 0.0);
    let set = CharStyle { leading: Some(14.0), ..st.clone() };
    assert_eq!(para.effective_leading(&set), 14.0);
    para.justification.auto_leading = f64::NAN;
    assert_eq!(para.effective_leading(&st), st.effective_leading(), "not a number");
}

/// A file can't feed layout justification values `text.setFormat` would refuse: each is
/// clamped to its range, one that isn't finite takes its default, and a Minimum, Desired and
/// Maximum out of order go back to their defaults.
#[test]
fn justification_options_read_from_a_file_stay_in_range() {
    let d = JustificationOptions::default();
    let read: ParaStyle = serde_json::from_value(serde_json::json!({
        "justification": {
            "word_spacing": [-5.0, 1e300, 1e308], "letter_spacing": [10.0, 5.0, 20.0],
            "glyph_scaling": [1.0, 100.0, 9e9], "auto_leading": -1e12
        }
    }))
    .unwrap();
    let j = read.justification;
    assert_eq!(j.word_spacing, [0.0, 1000.0, 1000.0]);
    assert_eq!(j.letter_spacing, d.letter_spacing, "out of order");
    assert_eq!(j.glyph_scaling, [50.0, 100.0, 200.0]);
    assert_eq!(j.auto_leading, 0.0);
    let mut j = JustificationOptions {
        word_spacing: [f64::NAN, 100.0, f64::INFINITY],
        letter_spacing: [-1.0, 0.0, f64::NEG_INFINITY],
        auto_leading: f64::NAN,
        ..d.clone()
    };
    j.keep_in_range();
    assert_eq!((j.word_spacing, j.letter_spacing, j.auto_leading), (d.word_spacing, [-1.0, 0.0, 0.0], d.auto_leading));
    let mut same = d.clone();
    same.keep_in_range();
    assert_eq!(same, d);
}

/// Numbered lists: each number style writes its numbers and rolls over (letters after Z,
/// roman numerals in subtractive forms, zero-padded numbers past their digits), the marker adds
/// a period, and the count runs through list paragraphs, skips empty and bulleted ones and
/// starts again after a paragraph in no list. A list from before number styles reads as
/// bulleted, and a bulleted list saves no number style.
#[test]
fn number_styles_write_their_numbers_and_roll_over() {
    let numbers = |style: NumberStyle, ns: &[u32]| ns.iter().map(|&n| style.format(n)).collect::<Vec<_>>();
    assert_eq!(numbers(NumberStyle::Arabic, &[1, 9, 10, 123]), ["1", "9", "10", "123"]);
    assert_eq!(
        numbers(NumberStyle::UpperLetters, &[1, 2, 26, 27, 28, 52, 53, 3999, 4000]),
        ["A", "B", "Z", "AA", "BB", "ZZ", "AAA", &"U".repeat(154), "4000"]
    );
    assert_eq!(numbers(NumberStyle::LowerLetters, &[1, 26, 27]), ["a", "z", "aa"]);
    assert_eq!(
        numbers(NumberStyle::UpperRoman, &[1, 2, 3, 4, 5, 9, 14, 40, 90, 400, 1994, 3999, 4000]),
        ["I", "II", "III", "IV", "V", "IX", "XIV", "XL", "XC", "CD", "MCMXCIV", "MMMCMXCIX", "4000"]
    );
    assert_eq!(numbers(NumberStyle::LowerRoman, &[1, 4, 9, 12]), ["i", "iv", "ix", "xii"]);
    assert_eq!(numbers(NumberStyle::TwoDigits, &[1, 9, 10, 99, 100]), ["01", "09", "10", "99", "100"]);
    assert_eq!(numbers(NumberStyle::ThreeDigits, &[1, 10, 999, 1000]), ["001", "010", "999", "1000"]);
    assert_eq!(ParaList::numbered(NumberStyle::UpperRoman).marker(4), "IV.");
    assert_eq!(ParaList::bulleted(Bullet::Square).marker(4), "\u{25AA}");
    let (num, dot) = (Some(ParaList::numbered(NumberStyle::Arabic)), Some(ParaList::bulleted(Bullet::Dot)));
    let mut count = 0;
    let markers: Vec<Option<String>> = [(num, false), (num, true), (dot, false), (num, false), (None, false), (num, false)]
        .iter()
        .map(|&(l, empty)| ParaList::next_marker(l, empty, &mut count))
        .collect();
    assert_eq!(markers, [Some("1.".into()), None, Some("\u{2022}".into()), Some("2.".into()), None, Some("1.".into())]);
    assert_eq!(count, 1);
    let read = |v: serde_json::Value| serde_json::from_value::<ParaList>(v).unwrap();
    assert_eq!(read(serde_json::json!({"bullet": "dash"})), ParaList::bulleted(Bullet::Dash));
    assert_eq!(read(serde_json::json!({"number": "threeDigits"})), ParaList::numbered(NumberStyle::ThreeDigits));
    assert_eq!(serde_json::to_value(ParaList::bulleted(Bullet::Dash)).unwrap(), serde_json::json!({"bullet": "dash"}));
    assert_eq!(
        serde_json::to_value(ParaList::numbered(NumberStyle::LowerRoman)).unwrap(),
        serde_json::json!({"bullet": "dot", "number": "lowerRoman"})
    );
}

/// Typed list detection: "1.", "1)", "A." and "A)" and a space at the start of a paragraph
/// start a numbered list, and "*", "#", ">" and "+" and a space start a bulleted list with the
/// filled dot. Other text, the same characters later in a paragraph or without the space, and
/// a paragraph already in a list start nothing.
#[test]
fn typed_list_starts_are_found_at_a_paragraphs_start() {
    let (arabic, letters, dot) =
        (ParaList::numbered(NumberStyle::Arabic), ParaList::numbered(NumberStyle::UpperLetters), ParaList::bulleted(Bullet::Dot));
    let typed = ["1.", "1)", "A.", "A)", "*", "#", ">", "+"].map(ParaList::typed);
    assert_eq!(typed, [arabic, arabic, letters, letters, dot, dot, dot, dot].map(Some));
    for other in ["", "2.", "a.", "a)", "I.", "1", "A", "1:", "-", "•", "1.)", " 1.", "*1", "１."] {
        assert_eq!(ParaList::typed(other), None, "{other:?}");
    }
    let mut t = TextObject::point(Point::ZERO, "Fruit\n1. Apples\nA) x * y\n+ ", CharStyle::default());
    assert_eq!(t.typed_list_at(9), Some((6, arabic)));
    assert_eq!(t.typed_list_at(19), Some((16, letters)));
    assert_eq!(t.typed_list_at(27), Some((25, dot)));
    // Not at the paragraph's start, not right after the space, or past the end.
    for at in [0, 5, 8, 10, 15, 23, 25, 26, 99] {
        assert_eq!(t.typed_list_at(at), None, "{at}");
    }
    let mut styles = t.paragraph_styles();
    styles[1].list = Some(ParaList::bulleted(Bullet::Square));
    t.set_paragraph_styles(styles);
    assert_eq!(t.typed_list_at(9), None, "a list paragraph stays as it is");
    assert_eq!(t.typed_list_at(19), Some((16, letters)));
}

#[test]
fn character_controls_reject_collapsed_and_nonfinite_transforms() {
    let mut t = TextObject::point(Point::ZERO, "Text", CharStyle::default());
    for matrix in [[0.0; 6], [1.0, 1.0, 2.0, 2.0, 0.0, 0.0], [f64::INFINITY, 0.0, 0.0, 1.0, 0.0, 0.0], [1.0, 0.0, 0.0, 1.0, f64::NAN, 0.0]] {
        t.xf = Affine::new(matrix);
        assert!(t.effective_char_style(&t.first_style()).is_none());
    }
    t.xf = Affine::rotate(0.83);
    assert_eq!(t.effective_char_style(&t.first_style()).unwrap().size, 12.0);
    t.xf = Affine::scale(1e308);
    assert!(t.effective_char_style(&t.first_style()).is_none(), "overflowing point size");
}

#[test]
fn point_text_basics() {
    let t = TextObject::point(Point::new(10.0, 20.0), "Hello", CharStyle::default());
    assert_eq!(t.plain_text(), "Hello");
    let b = t.bounds().unwrap();
    assert!(b.x0 >= 10.0 - 1e-9 && b.y1 > 20.0);
    assert_eq!(CharStyle::default().effective_leading(), 14.399999999999999);
}

fn justified(j: Justify) -> ParaStyle {
    ParaStyle { justify: j, ..ParaStyle::default() }
}

#[test]
fn paragraph_styles_split_merge_and_normalize() {
    let mut t = TextObject::point(Point::ZERO, "one\ntwo\nthree", CharStyle::default());
    assert_eq!((t.paragraph_count(), t.paras.len()), (3, 0), "one style for all: nothing stored");
    assert_eq!(t.paragraphs_in(0, 0), 0..1);
    assert_eq!(t.paragraphs_in(4, 4), 1..2);
    assert_eq!(t.paragraphs_in(2, 9), 0..3);
    assert_eq!(t.paragraphs_in(99, 99), 2..3, "clamped to the text");
    t.edit_paras(Some(1..2), |p| p.justify = Justify::Center);
    let js = |t: &TextObject| t.paragraph_styles().iter().map(|p| p.justify).collect::<Vec<_>>();
    assert_eq!(js(&t), [Justify::Left, Justify::Center, Justify::Left]);
    // Return inside "two": the new paragraph continues its style.
    t.splice_paras(5, 5, "\n");
    t.runs[0].text.insert(5, '\n');
    assert_eq!(js(&t), [Justify::Left, Justify::Center, Justify::Center, Justify::Left]);
    // Deleting the break between "one" and "t": the first paragraph's style wins.
    t.splice_paras(3, 4, "");
    t.runs[0].text.remove(3);
    assert_eq!(t.plain_text(), "onet\nwo\nthree");
    assert_eq!(js(&t), [Justify::Left, Justify::Center, Justify::Left]);
    // An edit that skips the splice is repaired by normalizing (the last style continues).
    t.runs[0].text.push_str("\nfour");
    t.normalize_paras();
    assert_eq!(js(&t), [Justify::Left, Justify::Center, Justify::Left, Justify::Left]);
    // `para` is paragraph 0's; all alike collapses to `para` alone.
    t.edit_paras(Some(0..1), |p| p.justify = Justify::Right);
    assert_eq!(t.para.justify, Justify::Right);
    t.edit_paras(None, |p| p.justify = Justify::Right);
    assert!(t.paras.is_empty() && t.para.justify == Justify::Right);
    // A short list repeats its last entry, a long one is cut.
    t.set_paragraph_styles(vec![justified(Justify::Left), justified(Justify::Center)]);
    assert_eq!(js(&t), [Justify::Left, Justify::Center, Justify::Center, Justify::Center]);
    t.set_paragraph_styles(vec![justified(Justify::Center); 9]);
    assert!(t.paras.is_empty() && t.para.justify == Justify::Center);
    // Old documents (no `paras`) load; new ones keep paragraph 0 in `para`.
    t.edit_paras(Some(3..4), |p| p.space_before = 4.0);
    let json = serde_json::to_value(&t).unwrap();
    assert_eq!(json["para"]["justify"], json["paras"][0]["justify"]);
    let mut old = json.clone();
    old.as_object_mut().unwrap().remove("paras");
    let back: TextObject = serde_json::from_value(old).unwrap();
    assert!(back.paras.is_empty() && back.para_at(3).justify == Justify::Center);
    let back: TextObject = serde_json::from_value(json).unwrap();
    assert_eq!(back.para_at(3).space_before, 4.0);
}

/// 120 × 40 area type at (40, 40), its text drawn at twice its size.
fn area_type() -> TextObject {
    let mut t = TextObject::point(Point::ZERO, "Some text", CharStyle::default());
    t.kind = TextKind::Area { frame: vectorcraft_geom::shapes::rectangle(Rect::new(0.0, 0.0, 60.0, 20.0)) };
    t.xf = Affine::translate((40.0, 40.0)) * Affine::scale(2.0);
    t
}

#[test]
fn area_resizes_its_frame_and_keeps_its_type() {
    let mut t = area_type();
    assert_eq!(t.bounds(), Some(Rect::new(40.0, 40.0, 160.0, 80.0)));
    // Bottom-right handle to (200, 140), about the top-left corner.
    let o = Affine::translate((40.0, 40.0));
    assert!(t.transform_area(o * Affine::scale_non_uniform(160.0 / 120.0, 100.0 / 40.0) * o.inverse()));
    assert_eq!(t.xf, Affine::translate((40.0, 40.0)) * Affine::scale(2.0), "the type keeps its size");
    let b = t.bounds().unwrap();
    assert!((b.x1 - 200.0).abs() < 1e-9 && (b.y1 - 140.0).abs() < 1e-9 && b.x0 == 40.0, "{b:?}");
    assert_eq!(t.area_frame().unwrap().bounds(), Some(b));
    // Point type has no area; a degenerate transform or a non-finite frame changes nothing.
    let mut p = TextObject::point(Point::ZERO, "x", CharStyle::default());
    assert!(!p.transform_area(Affine::scale(2.0)) && p.area_frame().is_none());
    let before = t.clone();
    assert!(!t.transform_area(Affine::scale(f64::INFINITY)));
    t.xf = Affine::scale(0.0);
    assert!(!t.transform_area(Affine::scale(2.0)));
    t.xf = before.xf;
    assert_eq!(t, before);
}

#[test]
fn area_anchors_move_in_document_space() {
    let mut t = area_type();
    // The bottom-right corner (anchor 2) 20 pt right: 10 pt in text space at 2×.
    assert!(t.move_area_anchors(&[(0, 2)], Vec2::new(20.0, 0.0)));
    let TextKind::Area { frame } = &t.kind else { panic!() };
    assert_eq!(frame.subpaths[0].anchors[2].p, Point::new(70.0, 20.0));
    assert_eq!(frame.subpaths[0].anchors[1].p, Point::new(60.0, 0.0), "the others stay");
    // A missing anchor changes nothing.
    let before = t.clone();
    assert!(!t.move_area_anchors(&[(0, 1), (0, 9)], Vec2::new(5.0, 5.0)));
    assert!(!t.move_area_anchors(&[(3, 0)], Vec2::new(5.0, 5.0)));
    assert_eq!(t, before);
}

/// Builds from before [`Justify::Auto`] read new type's alignment as Left or Right (and ignore
/// `justify_auto`); this build reads Auto back.
#[test]
fn auto_alignment_saves_readable_by_older_builds() {
    #[derive(Deserialize, Debug, PartialEq)]
    enum OldJustify {
        Left,
        Center,
        Right,
        JustifyLeft,
        JustifyCenter,
        JustifyRight,
        JustifyAll,
    }
    #[derive(Deserialize)]
    struct OldPara {
        justify: OldJustify,
    }
    #[derive(Deserialize)]
    struct OldText {
        para: OldPara,
    }
    for (direction, physical) in
        [(Some(ParaDirection::RightToLeft), OldJustify::Right), (Some(ParaDirection::LeftToRight), OldJustify::Left), (None, OldJustify::Left)]
    {
        let mut t = TextObject::point(Point::ZERO, "שלום", CharStyle::default());
        t.para = ParaStyle { justify: Justify::Auto, direction, ..Default::default() };
        let json = serde_json::to_string(&t).unwrap();
        let old: OldText = serde_json::from_str(&json).unwrap();
        assert_eq!(old.para.justify, physical, "{json}");
        assert_eq!(serde_json::from_str::<TextObject>(&json).unwrap(), t);
    }
    // Other alignments save as before, without the flag.
    let p = ParaStyle { justify: Justify::Right, ..Default::default() };
    let json = serde_json::to_string(&p).unwrap();
    assert!(!json.contains("justify_auto"), "{json}");
    assert_eq!(serde_json::from_str::<ParaStyle>(&json).unwrap(), p);
    // Files from before the flag keep their alignment.
    assert_eq!(serde_json::from_str::<ParaStyle>(r#"{"justify":"Center"}"#).unwrap().justify, Justify::Center);
}
