//! Real text: type exported as selectable, searchable text in embedded subset fonts with a
//! ToUnicode map, where the outlines would be; fonts whose licence forbids it stay outlines.

use serde_json::json;
use vectorcraft_color::{Color, Gradient, GradientPaint, Paint};
use vectorcraft_doc::{CharStyle, Document, Node, NodeKind, TextKind, TextObject, TextRun};
use vectorcraft_geom::{Affine, BezPath, PathData, Point, Rect};
use vectorcraft_text::{FALLBACK_FAMILY, FontDb};

use crate::*;

fn style(size: f64) -> CharStyle {
    CharStyle { size, fill: Paint::solid(Color::BLACK), ..Default::default() }
}

/// A document holding `texts`.
fn doc(texts: Vec<TextObject>) -> Document {
    let mut d = Document::new(400.0, 300.0);
    let l = d.layers[0].id;
    for (i, t) in texts.into_iter().enumerate() {
        let n = Node::new(d.alloc_id(), NodeKind::Text(Box::new(t)));
        d.insert(Some(l), i, n).unwrap();
    }
    d
}

/// The export of `d` with real text (or outlines), content streams left readable.
fn pdf(d: &Document, outline_text: bool) -> ExportReport {
    let settings: PdfSettings =
        serde_json::from_value(json!({"compression": {"compressText": false}, "advanced": {"outlineText": outline_text}})).unwrap();
    export_with_report(d, &PdfOptions { settings, ..Default::default() }).unwrap()
}

fn import_as(bytes: &[u8], text_as: TextAs) -> Document {
    import_with_report(bytes, &ImportOptions { text_as, ..Default::default() }).unwrap().document
}

/// The text of the type objects of `d`.
fn texts(d: &Document) -> Vec<String> {
    let mut out = vec![];
    d.walk(|n| {
        if let NodeKind::Text(t) = &n.kind {
            out.push(t.plain_text());
        }
    });
    out
}

/// The bounds of the paths of `d` (the outlines type imports as), all together.
fn ink(d: &Document) -> Rect {
    let mut b: Option<Rect> = None;
    d.walk(|n| {
        if matches!(n.kind, NodeKind::Path { .. } | NodeKind::Compound { .. })
            && let Some(r) = n.geometric_bounds()
        {
            b = Some(b.map_or(r, |b| b.union(r)));
        }
    });
    b.unwrap()
}

#[test]
fn type_is_written_as_text_in_embedded_subset_fonts() {
    let d = doc(vec![TextObject::point(Point::new(20.0, 60.0), "Hello World", style(24.0))]);
    let r = pdf(&d, false);
    assert!(r.warnings.is_empty(), "{:?}", r.warnings);
    let text = String::from_utf8_lossy(&r.bytes);
    assert!(text.contains("/FontFile2") || text.contains("/FontFile3"), "an embedded font program");
    assert!(text.contains("/ToUnicode") && text.contains("/Type0"));
    // A subset: the font's name carries the subset tag.
    assert!(text.contains(&format!("+{}", FALLBACK_FAMILY.replace(' ', ""))), "a subset");
    assert!(text.contains(" Tf") && text.contains("TJ"));
    // Selectable and searchable: reopened, it is type again, spaces included.
    assert_eq!(texts(&import_as(&r.bytes, TextAs::Text)), ["Hello World"]);
    // Outlined text has no font.
    assert!(!String::from_utf8_lossy(&pdf(&d, true).bytes).contains("/Font"));
}

/// A curve type runs along.
fn arc() -> PathData {
    let mut p = BezPath::new();
    p.move_to((20.0, 250.0));
    p.curve_to((120.0, 150.0), (260.0, 150.0), (380.0, 250.0));
    PathData::from_bezpath(&p)
}

#[test]
fn real_text_lands_within_half_a_point_of_the_outlines() {
    let plain = TextObject::point(Point::new(20.0, 60.0), "Hg fly", style(36.0));
    let mut turned = TextObject::point(Point::new(60.0, 160.0), "Wave AV", CharStyle { h_scale: 80.0, tracking: 120.0, ..style(30.0) });
    turned.xf *= Affine::rotate(-0.5) * Affine::skew(0.2, 0.0);
    let mut mixed = TextObject::point(Point::new(200.0, 60.0), "", style(20.0));
    mixed.runs = vec![
        TextRun { text: "x".into(), style: style(20.0), inline: None },
        TextRun { text: "2".into(), style: CharStyle { baseline_shift: 8.0, v_scale: 140.0, ..style(12.0) }, inline: None },
        TextRun { text: " ok".into(), style: CharStyle { rotation: 20.0, ..style(20.0) }, inline: None },
    ];
    let mut on_path = TextObject::point(Point::ZERO, "along the arc", style(18.0));
    on_path.kind = TextKind::OnPath { path: arc(), start: 0.0, end: None };
    on_path.xf = Affine::IDENTITY;
    for t in [plain, turned, mixed, on_path] {
        let what = t.plain_text();
        let d = doc(vec![t]);
        let real = import_as(&pdf(&d, false).bytes, TextAs::Outlines);
        let outlined = import_as(&pdf(&d, true).bytes, TextAs::Outlines);
        let (a, b) = (ink(&real), ink(&outlined));
        let off = [a.x0 - b.x0, a.y0 - b.y0, a.x1 - b.x1, a.y1 - b.y1].iter().fold(0.0f64, |m, v| m.max(v.abs()));
        assert!(off < 0.5, "{what}: {a:?} vs {b:?}");
        assert_eq!(texts(&import_as(&pdf(&d, false).bytes, TextAs::Text)).concat().replace(' ', ""), what.replace(' ', ""), "{what}");
    }
}

/// A named instance of a variable font is real text in that instance: its embedded subset draws
/// the instance's glyphs where the outlines would be, not the default instance's (#296).
#[test]
fn variable_font_instances_are_real_text_in_their_own_glyphs() {
    use vectorcraft_text::test_fonts::{VARIABLE_CHARS, VARIABLE_FAMILY, variable_font};
    FontDb::global().add_font(variable_font().unwrap());
    let mut widths = vec![];
    for s in ["Regular", "Bold"] {
        let st = CharStyle { font_family: VARIABLE_FAMILY.into(), font_style: s.into(), ..style(48.0) };
        let d = doc(vec![TextObject::point(Point::new(20.0, 80.0), VARIABLE_CHARS, st)]);
        let r = pdf(&d, false);
        assert!(r.warnings.is_empty(), "{s}: {:?}", r.warnings);
        assert_eq!(texts(&import_as(&r.bytes, TextAs::Text)), [VARIABLE_CHARS], "{s}: real text");
        let (real, outlined) = (ink(&import_as(&r.bytes, TextAs::Outlines)), ink(&import_as(&pdf(&d, true).bytes, TextAs::Outlines)));
        let off =
            [real.x0 - outlined.x0, real.y0 - outlined.y0, real.x1 - outlined.x1, real.y1 - outlined.y1].iter().fold(0.0f64, |m, v| m.max(v.abs()));
        assert!(off < 0.5, "{s}: {real:?} vs {outlined:?}");
        widths.push(real.width());
    }
    assert!(widths[1] > widths[0] * 1.2, "Bold is wider: {widths:?}");
}

#[test]
fn gradients_on_real_text_stay_where_they_were() {
    let fill = Paint::Gradient(Box::new(GradientPaint::new(Gradient::default())));
    let mut t = TextObject::point(Point::new(30.0, 80.0), "Gradient", CharStyle { fill, ..style(40.0) });
    t.xf *= Affine::rotate(0.3);
    let d = doc(vec![t]);
    let geom = |outline: bool| {
        let back = import_as(&pdf(&d, outline).bytes, TextAs::Outlines);
        let mut g = None;
        back.walk(|n| {
            if let Paint::Gradient(p) = n.appearance.fill_paint() {
                g = g.or(p.geom);
            }
        });
        g.unwrap()
    };
    let (real, outlined) = (geom(false), geom(true));
    assert!((real.start - outlined.start).hypot() < 0.1 && (real.end - outlined.end).hypot() < 0.1, "{real:?} vs {outlined:?}");
}

/// The bundled fallback face as another family `family` (13 characters, like the bundled name),
/// whose licence flags (OS/2 `fsType`) are `fs`.
fn licensed(family: &str, fs: u16) -> String {
    let face = FontDb::global().face(FALLBACK_FAMILY, "Regular").unwrap();
    let mut bytes = face.file_data().to_vec();
    let utf16 = |s: &str| s.encode_utf16().flat_map(u16::to_be_bytes).collect::<Vec<u8>>();
    for (from, to) in [(utf16(FALLBACK_FAMILY), utf16(family)), (FALLBACK_FAMILY.as_bytes().to_vec(), family.as_bytes().to_vec())] {
        assert_eq!(from.len(), to.len());
        let mut at = 0;
        while let Some(i) = bytes[at..].windows(from.len()).position(|w| w == from) {
            bytes[at + i..at + i + from.len()].copy_from_slice(&to);
            at += i + from.len();
        }
    }
    let tables = u16::from_be_bytes([bytes[4], bytes[5]]) as usize;
    let os2 = (0..tables).map(|i| 12 + 16 * i).find(|r| &bytes[*r..*r + 4] == b"OS/2").unwrap();
    let off = u32::from_be_bytes(bytes[os2 + 8..os2 + 12].try_into().unwrap()) as usize;
    bytes[off + 8..off + 10].copy_from_slice(&fs.to_be_bytes());
    FontDb::global().add_font(bytes);
    assert!(FontDb::global().has_family(family));
    family.into()
}

#[test]
fn fonts_whose_licence_forbids_embedding_fall_back_to_outlines() {
    let locked = licensed("Locked Sans 3", 0x0002);
    let whole = licensed("Entire Sans 3", 0x0100);
    let d = doc(vec![
        TextObject::point(Point::new(20.0, 40.0), "Open", style(24.0)),
        TextObject::point(Point::new(20.0, 100.0), "Shut", CharStyle { font_family: locked.clone(), ..style(24.0) }),
        TextObject::point(Point::new(20.0, 160.0), "Full", CharStyle { font_family: whole.clone(), ..style(24.0) }),
    ]);
    let r = pdf(&d, false);
    assert!(r.warnings.iter().any(|w| w.contains(&locked) && w.contains("doesn't allow embedding")), "{:?}", r.warnings);
    assert!(r.warnings.iter().any(|w| w.contains(&whole) && w.contains("whole")), "{:?}", r.warnings);
    // Only the open font is text; the others are drawn as their outlines.
    let back = import_as(&r.bytes, TextAs::Text);
    assert_eq!(texts(&back), ["Open"]);
    let mut paths = 0;
    back.walk(|n| paths += usize::from(matches!(n.kind, NodeKind::Path { .. } | NodeKind::Compound { .. })));
    assert!(paths >= 2, "{paths} outlined words");
}

#[test]
fn subset_thresholds_below_100_percent_are_accepted_with_a_warning() {
    let d = doc(vec![TextObject::point(Point::new(20.0, 60.0), "Subset", style(24.0))]);
    let settings: PdfSettings = serde_json::from_value(json!({"advanced": {"outlineText": false, "fontSubsetPercent": 40}})).unwrap();
    let r = export_with_report(&d, &PdfOptions { settings, ..Default::default() }).unwrap();
    assert!(r.warnings.iter().any(|w| w.contains("subset")), "{:?}", r.warnings);
    assert_eq!(texts(&import_as(&r.bytes, TextAs::Text)), ["Subset"]);
}

#[test]
fn pdf_a_takes_real_text() {
    let d = doc(vec![TextObject::point(Point::new(20.0, 60.0), "Archive", style(24.0))]);
    let settings: PdfSettings = serde_json::from_value(json!({"standard": "pdfA2b", "advanced": {"outlineText": false}})).unwrap();
    let bytes = export(&d, &PdfOptions { settings, created: Some(0), ..Default::default() }).unwrap();
    assert_eq!(texts(&import_as(&bytes, TextAs::Text)), ["Archive"]);
}

/// A named instance comes back as live type in that style (not the default instance's).
#[test]
fn a_variable_font_instance_comes_back_in_its_own_style() {
    use vectorcraft_text::test_fonts::{VARIABLE_CHARS, VARIABLE_FAMILY, variable_font};
    FontDb::global().add_font(variable_font().unwrap());
    let st = CharStyle { font_family: VARIABLE_FAMILY.into(), font_style: "Bold".into(), ..style(48.0) };
    let d = doc(vec![TextObject::point(Point::new(20.0, 80.0), VARIABLE_CHARS, st)]);
    let back = crate::import_with_report(&pdf(&d, false).bytes, &ImportOptions { text_as: TextAs::Text, ..Default::default() }).unwrap().document;
    let mut styles = vec![];
    back.walk(|n| {
        if let NodeKind::Text(t) = &n.kind {
            styles.extend(t.runs.iter().map(|r| r.style.font_style.clone()));
        }
    });
    assert_eq!(styles, ["Bold"]);
}

/// The export of `text` (one point type object) with its ToUnicode map rewritten so that the
/// digits map to U+FFFD, as some files have it (#708). Each `<003D>` of a digit becomes `<FFFD>`, so
/// the file's offsets still hold.
fn digits_unnamed(text: &str) -> Vec<u8> {
    let d = doc(vec![TextObject::point(Point::new(20.0, 50.0), text, style(24.0))]);
    let bytes = pdf(&d, false).bytes;
    let at = bytes.windows(11).position(|w| w == b"beginbfchar").expect("a readable ToUnicode map");
    let end = at + bytes[at..].windows(9).position(|w| w == b"endbfchar").unwrap();
    let mut out = bytes.clone();
    for i in at..end.saturating_sub(5) {
        // `<0030>` … `<0039>` after a space: a digit's character (each line's code comes first).
        if &bytes[i..i + 4] == b"<003" && bytes[i + 4].is_ascii_digit() && bytes[i + 5] == b'>' && bytes[i - 1] == b' ' {
            out[i + 1..i + 5].copy_from_slice(b"FFFD");
        }
    }
    assert_ne!(out, bytes, "digits were mapped");
    out
}

/// #708, #811: a glyph whose ToUnicode value is U+FFFD isn't type showing "�". This subset font
/// has no character map of its own, but the font is installed: each digit is found by its outline
/// there, so the whole text stays one type object, with no outlines left behind.
#[test]
fn unnamed_glyphs_of_an_installed_font_are_found_by_their_outline() {
    let bytes = digits_unnamed("Tel 02-12345");
    let report = import_with_report(&bytes, &ImportOptions { text_as: TextAs::Text, ..Default::default() }).unwrap();
    assert_eq!(texts(&report.document), ["Tel 02-12345"]);
    let mut paths = 0;
    report.document.walk(|n| paths += usize::from(matches!(n.kind, NodeKind::Path { .. } | NodeKind::Compound { .. })));
    assert_eq!(paths, 0, "no outlines left: {:?}", report.warnings);
    assert!(!report.warnings.iter().any(|w| w.contains("U+FFFD")), "{:?}", report.warnings);
}

/// #811: an outline is named only by a glyph that draws it exactly; one moved off its place, or an
/// empty glyph (which every space draws), stays unnamed and keeps its outlines.
#[test]
fn glyphs_are_named_only_by_an_exact_outline() {
    let db = FontDb::global();
    let face = db.face("Source Sans 3", "Regular").unwrap();
    let k = 1000.0 / face.units_per_em();
    let pdf_space = |c: char| Affine::new([k, 0.0, 0.0, -k, 0.0, 0.0]) * db.outline(&face, face.glyph_for(c)).as_ref().clone();
    for c in ['R', 'a', '7', '&', 'é'] {
        assert_eq!(crate::import::identify_glyph(&face, &pdf_space(c)), Some(c));
    }
    assert_eq!(crate::import::identify_glyph(&face, &(Affine::translate((100.0, 0.0)) * pdf_space('R'))), None);
    assert_eq!(crate::import::identify_glyph(&face, &BezPath::new()), None);
}

/// #708: where the embedded font program has a character map, a glyph's character comes from it.
#[test]
fn an_embedded_fonts_own_cmap_names_its_glyphs() {
    let data = std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/fonts/SourceSans3-Regular.ttf")).unwrap();
    let chars = crate::import::font_chars(&data).unwrap();
    use skrifa::MetadataProvider;
    let font = skrifa::FontRef::new(&data).unwrap();
    for c in ['0', '7', 'A', 'z'] {
        let g = font.charmap().map(c).unwrap().to_u32();
        assert_eq!(chars.get(&g), Some(&c));
    }
    assert!(crate::import::font_chars(b"not a font").is_none());
}

/// Type whose text matrix shears its glyphs (slanted as a whole, as Illustrator writes type
/// turned and skewed) reopens as type that still leans: drawn, it covers what its outlines do.
#[test]
fn slanted_type_reopens_as_type_that_still_leans() {
    for (turn, lean) in [(0.0, 0.25), (-0.2, 0.25), (0.3, -0.15)] {
        let mut t = TextObject::point(Point::new(80.0, 150.0), "LEAN", style(60.0));
        t.xf *= Affine::rotate(turn) * Affine::skew(lean, 0.0);
        let d = doc(vec![t]);
        let reopened = import_as(&pdf(&d, false).bytes, TextAs::Text);
        assert_eq!(texts(&reopened), ["LEAN"], "{turn} {lean}: reopened as type");
        let (a, b) = (ink(&import_as(&pdf(&reopened, true).bytes, TextAs::Outlines)), ink(&import_as(&pdf(&d, true).bytes, TextAs::Outlines)));
        let off = [a.x0 - b.x0, a.y0 - b.y0, a.x1 - b.x1, a.y1 - b.y1].iter().fold(0.0f64, |m, v| m.max(v.abs()));
        assert!(off < 0.5, "{turn} {lean}: {a:?} vs {b:?}");
    }
}

/// Type set in an installed font's older version (a library keeps both, one name) reopens as type
/// in that version: the file's glyphs are matched to it rather than kept as outlines, and the type
/// names the version, which the family and style alone don't pick.
#[test]
fn type_in_another_installed_version_reopens_in_that_version() {
    use vectorcraft_text::test_fonts::{TWIN_FAMILY, TWIN_VERSIONS, twin_font};
    // The newer version first: the one the family and style resolve to.
    FontDb::global().add_font(twin_font(true).unwrap());
    FontDb::global().add_font(twin_font(false).unwrap());
    for version in [None, Some(TWIN_VERSIONS[0])] {
        let st = CharStyle { font_family: TWIN_FAMILY.into(), font_version: version.map(Into::into), ..style(40.0) };
        let d = doc(vec![TextObject::point(Point::new(20.0, 80.0), "Hamburgefonstiv", st)]);
        let back = import_as(&pdf(&d, false).bytes, TextAs::Text);
        let mut found = vec![];
        back.walk(|n| {
            if let NodeKind::Text(t) = &n.kind {
                found.push((t.plain_text(), t.first_style().font_version.clone()));
            }
        });
        assert_eq!(found, [("Hamburgefonstiv".to_string(), version.map(String::from))], "{version:?}");
    }
}
