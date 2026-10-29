//! Editable text import regressions on original synthetic version-12 `.af` streams and pinned
//! public Affinity 3 files. Font rendering depends on installed fonts; these are structure and
//! conversion oracles, not claims of pixel-identical typography.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[path = "../../affinity/tests/support/corpus.rs"]
mod corpus;
#[path = "../../affinity/tests/support/text_fixture.rs"]
mod fixture;
use fixture::*;
use vectorcraft_affinity::synth::{F, tag};
use vectorcraft_doc::{
    Document, Node, NodeKind, TextObject,
    text::{CharStyle, Justify, TextKind},
};
use vectorcraft_engine::cmd::fileio;

fn texts(d: &Document) -> Vec<&TextObject> {
    fn walk<'a>(n: &'a Node, out: &mut Vec<&'a TextObject>) {
        if let NodeKind::Text(t) = &n.kind {
            out.push(t);
        }
        if let Some(children) = n.children() {
            for child in children {
                walk(child, out);
            }
        }
    }
    let mut out = Vec::new();
    for n in &d.layers {
        walk(n, &mut out);
    }
    out
}
fn import(blocks: Vec<F>, frame: bool, dpi: f64, transform: [f64; 6]) -> fileio::Loaded {
    let loaded = fileio::load("original-text.af", &document(blocks, frame, dpi, transform)).unwrap();
    assert!(!loaded.preview_only);
    loaded
}

#[test]
fn unicode_mixed_fonts_line_breaks_and_styles_remain_editable_after_save() {
    let loaded = import(
        vec![block(
            utf8("é😀\u{2028}ab\u{2029}尾\0"),
            vec![run(2, Some(attrs("First", "FirstPS", 700, true, 24.0))), run(8, Some(attrs("Second", "SecondPS", 300, false, 20.0)))],
            &[1],
        )],
        false,
        96.0,
        [2.0, 0.0, 30.0, 0.0, 2.0, 40.0],
    );
    let t = texts(&loaded.doc)[0];
    assert_eq!(t.runs.iter().map(|r| r.text.as_str()).collect::<Vec<_>>(), ["é😀", "\nab\n尾"]);
    assert_eq!(t.runs[0].style.font_family, "First");
    assert_eq!(t.runs[0].style.font_style, "Bold Italic");
    assert_eq!(t.runs[1].style.font_style, "Light");
    assert_eq!((t.runs[0].style.size, t.runs[0].style.leading, t.runs[0].style.tracking), (36.0, Some(45.0), 25.0));
    assert_eq!(t.para.justify, Justify::Center);
    assert!(matches!(t.kind, TextKind::Point));
    assert_eq!(t.xf.as_coeffs(), [1.0, 0.0, 0.0, 1.0, 187.5, 87.0]);
    let saved = vectorcraft_format::save_file(&loaded.doc);
    let restored = vectorcraft_format::load(&saved).unwrap();
    assert_eq!(texts(&restored)[0], t);
}

#[test]
fn absent_font_name_uses_named_default_and_postscript_name_is_preserved() {
    let loaded =
        import(vec![block(utf8("ab"), vec![run(1, Some(attrs("", "OnlyPostScript", 400, false, 12.0))), run(2, None)], &[])], false, 72.0, IDENTITY);
    let t = texts(&loaded.doc)[0];
    assert_eq!(t.runs[0].style.font_family, "OnlyPostScript");
    assert_eq!(t.runs[1].style.font_family, CharStyle::default().font_family);
    assert_eq!(t.runs[1].style.fill.color().unwrap().to_rgb(), [0.0, 0.0, 0.0]);
    assert!(loaded.warnings.iter().any(|w| w.contains("without a font name")));
    assert!(loaded.warnings.iter().any(|w| w.contains("without character attributes")));
}

#[test]
fn frame_type_keeps_dimensions_alignment_and_unit_conversion() {
    let loaded = import(vec![block(utf8("text"), vec![run(4, Some(attrs("Frame", "", 500, false, 20.0)))], &[3])], true, 144.0, IDENTITY);
    let t = texts(&loaded.doc)[0];
    assert_eq!(t.runs[0].style.size, 10.0);
    assert_eq!(t.runs[0].style.font_style, "Medium");
    assert_eq!(t.para.justify, Justify::JustifyLeft);
    let TextKind::Area { frame } = &t.kind else { panic!("expected area type") };
    assert_eq!(frame.bounds().unwrap().width(), 100.0);
    assert_eq!(frame.bounds().unwrap().height(), 50.0);
}

#[test]
fn a_frame_too_short_for_its_first_line_is_made_taller_with_a_warning() {
    // #1423: a title taller than its frame (60 pt type in a 50 pt frame) drew nothing.
    let lines = |t: &TextObject| vectorcraft_text::layout(vectorcraft_text::FontDb::global(), t).lines.len();
    let title = |size: f64| import(vec![block(utf8("T"), vec![run(1, Some(attrs("Frame", "", 400, false, size)))], &[3])], true, 144.0, IDENTITY);
    // Its baseline (at cap height) is inside the frame, its descenders hang out of it; and an
    // 80 pt title whose capitals don't fit either.
    for (size, warning, max) in [(120.0, "made taller so the lines show", 100.0), (160.0, "too short for their first line", 150.0)] {
        let short = title(size);
        let t = texts(&short.doc)[0];
        let TextKind::Area { frame } = &t.kind else { panic!("expected area type") };
        let b = frame.bounds().unwrap();
        assert!((b.x0, b.y0, b.width()) == (0.0, 0.0, 100.0) && b.height() > 50.0 && b.height() < max, "{size}: taller from its top: {b:?}");
        assert_eq!(lines(t), 1, "the line shows");
        assert!(short.warnings.iter().any(|w| w.contains(warning)), "{size}: {:?}", short.warnings);
    }
    // A frame that holds its line keeps its height, and nothing is said.
    let fits = title(20.0);
    let t = texts(&fits.doc)[0];
    let TextKind::Area { frame } = &t.kind else { panic!("expected area type") };
    assert_eq!((frame.bounds().unwrap().height(), lines(t)), (50.0, 1));
    assert!(!fits.warnings.iter().any(|w| w.contains("too short")), "{:?}", fits.warnings);
}

#[test]
fn each_font_weight_and_italic_maps_to_the_expected_style() {
    let cases = [
        (100, "Thin"),
        (200, "ExtraLight"),
        (300, "Light"),
        (400, "Regular"),
        (500, "Medium"),
        (600, "SemiBold"),
        (700, "Bold"),
        (800, "ExtraBold"),
        (900, "Black"),
    ];
    for (weight, expected) in cases {
        for italic in [false, true] {
            let loaded = import(vec![block(utf8("x"), vec![run(1, Some(attrs("Family", "", weight, italic, 12.0)))], &[])], false, 72.0, IDENTITY);
            let style = &texts(&loaded.doc)[0].runs[0].style.font_style;
            let expected = if !italic {
                expected.into()
            } else if expected == "Regular" {
                "Italic".into()
            } else {
                format!("{expected} Italic")
            };
            assert_eq!(*style, expected);
        }
    }
}

#[test]
fn caps_and_feature_overrides_pass_through_to_editable_type() {
    let mut a = attrs("Family", "", 400, false, 12.0);
    if let F::Obj(_, fields) = &mut a {
        fields.iter_mut().find(|(t, _)| *t == tag(b"Objs")).unwrap().1 = F::Shared(vec![
            solid(1.0),
            F::Null,
            F::Null,
            F::Null,
            F::Null,
            F::Null,
            F::Null,
            F::Obj(
                tag(b"OtAt"),
                vec![(
                    tag(b"Setn"),
                    F::Objs(
                        tag(b"OTFS"),
                        vec![
                            vec![(tag(b"Feat"), F::U32(u32::from_be_bytes(*b"CAP\x01"))), (tag(b"Valu"), F::I32(1))],
                            vec![(tag(b"Feat"), F::U32(u32::from_be_bytes(*b"liga"))), (tag(b"Valu"), F::I32(0))],
                            vec![(tag(b"Feat"), F::U32(u32::from_be_bytes(*b"ss01"))), (tag(b"Valu"), F::I32(1))],
                            // Character variants aren't rendered yet (stylistic sets are, #1607).
                            vec![(tag(b"Feat"), F::U32(u32::from_be_bytes(*b"cv01"))), (tag(b"Valu"), F::I32(1))],
                        ],
                    ),
                )],
            ),
        ]);
    }
    let loaded = import(vec![block(utf8("Mixed"), vec![run(5, Some(a))], &[])], false, 72.0, IDENTITY);
    let t = texts(&loaded.doc)[0];
    assert_eq!(t.runs[0].text, "Mixed");
    assert!(t.runs[0].style.all_caps);
    assert_eq!(t.runs[0].style.features, ["-liga", "ss01", "cv01"]);
    assert!(loaded.warnings.iter().any(|w| w.contains("retained but not rendered")));
    let restored = vectorcraft_format::load(&vectorcraft_format::save_file(&loaded.doc)).unwrap();
    assert_eq!(texts(&restored)[0], t);
}

#[test]
fn pinned_affinity3_text_opens_as_editable_unicode_with_resolved_emoji_font() {
    let Some(corpus) = corpus::pinned() else { return };
    let load = |name: &str| {
        let loaded = fileio::load(name, &std::fs::read(corpus.join(name)).unwrap()).unwrap();
        assert!(!loaded.preview_only);
        loaded
    };
    let loaded = load("patchy-text-runs.af");
    let t = texts(&loaded.doc)[0];
    assert_eq!(t.runs.iter().map(|r| r.text.as_str()).collect::<String>(), "ABcdÉ😀X");
    assert_eq!(t.runs[3].style.font_family, "Segoe UI Emoji");
    assert_eq!(t.runs[4].style.font_family, "Courier New");
    let loaded = load("patchy-text-caps.af");
    let ts = texts(&loaded.doc);
    assert!(ts[0].runs[0].style.all_caps);
    assert_eq!(ts[1].runs[0].style.features, ["smcp"]);
    for name in ["patchy-text-artistic.af", "patchy-text-frame.af", "patchy-text-rotated.af", "patchy-text-indent.af", "patchy-text-para-spacing.af"]
    {
        let loaded = load(name);
        assert!(!texts(&loaded.doc).is_empty(), "{name}");
        let restored = vectorcraft_format::load(&vectorcraft_format::save_file(&loaded.doc)).unwrap();
        assert_eq!(texts(&restored), texts(&loaded.doc), "{name}");
    }
}

#[test]
fn installed_postscript_identity_overrides_ambiguous_family_and_weight() {
    // This is an OS font metadata check. No font file is committed or required; the structural
    // fallback cases above always run, including when the optional font repository is absent.
    let db = vectorcraft_text::FontDb::global();
    let Some((family, face_style)) = db.by_postscript_name("DejaVuSans") else {
        eprintln!("DejaVuSans is not installed; exact PostScript face test skipped");
        return;
    };
    let loaded =
        import(vec![block(utf8("x"), vec![run(1, Some(attrs("Ambiguous Family", "DejaVuSans", 900, true, 12.0)))], &[])], false, 72.0, IDENTITY);
    let style = &texts(&loaded.doc)[0].runs[0].style;
    assert_eq!(style.font_family, family);
    assert_eq!(style.font_style, face_style);
}
