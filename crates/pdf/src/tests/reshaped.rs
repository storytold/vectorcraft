//! #5235: type in a script whose letters reorder as they are set (here Devanagari) opens as type
//! only when that type draws what the file drew; otherwise the file's glyphs stay, as outlines.
//! The file is written from scratch from the PDF specification (ISO 32000-1 §9.7, composite
//! fonts), its font program a test font built in code over Source Sans 3
//! ([`vectorcraft_text::test_fonts::matra_font`]): Ka draws Source Sans 3's `a` and the vowel sign
//! I, set before its consonant, draws its `i`.

use vectorcraft_doc::{Document, NodeKind};
use vectorcraft_testkit::pdf::{PdfPage, first_extra, pdf_with};
use vectorcraft_text::FontDb;
use vectorcraft_text::test_fonts::{MATRA_FAMILY, matra_font};

use crate::tests_import_fidelity::stream;
use crate::*;

const KA: char = 'क';
const SIGN_I: char = '\u{093F}';

/// The test font's glyph of `c` (Source Sans 3's glyph of the Latin letter that draws it) and its
/// advance in thousandths of an em.
fn glyph(c: char) -> (u32, f64) {
    let face = FontDb::global().face(MATRA_FAMILY, "Regular").expect("the test font is added");
    let g = face.glyph_for(c);
    (g, (face.advance(g) * 1000.0 / face.units_per_em()).round())
}

/// A 300 × 200 pt page showing the glyphs of `drawn`, in that order, in the test font at 40 pt
/// from (20, 100): a Type 0 font (Identity-H, two-byte glyph ids) with the font program embedded
/// and a ToUnicode map naming each glyph's character. `sign_width`: the advance the file gives
/// the vowel sign, in thousandths of an em (`None`: the font's).
fn page(drawn: &[char], sign_width: Option<f64>) -> Vec<u8> {
    page_with(|id| format!("[{}] TJ", drawn.iter().map(|&c| id(c)).collect::<String>()), "", sign_width)
}

/// [`page`] showing glyphs with the text operators `ops` makes (after the position of the first
/// glyph) from a function giving a character's glyph id in hex, and `extra` more resources: a
/// graphics state, to set between glyphs or before them.
fn page_with(ops: impl Fn(&dyn Fn(char) -> String) -> String, extra: &str, sign_width: Option<f64>) -> Vec<u8> {
    FontDb::global().add_font(matra_font().expect("the test font builds"));
    let program = matra_font().unwrap();
    let hex: String = program.iter().map(|b| format!("{b:02X}")).collect::<String>() + ">";
    let n = first_extra(1);
    let rows: Vec<String> = [KA, SIGN_I].iter().map(|&c| format!("<{:04X}> <{:04X}>", glyph(c).0, c as u32)).collect();
    let width = |c| sign_width.filter(|_| c == SIGN_I).unwrap_or(glyph(c).1);
    let widths: Vec<String> = [KA, SIGN_I].iter().map(|&c| format!("{} [{}]", glyph(c).0, width(c))).collect();
    let objs = [
        stream(&format!("/Length1 {} /Filter /ASCIIHexDecode", program.len()), &hex),
        format!(
            "<< /Type /FontDescriptor /FontName /MatratestSans-Regular /Flags 4 /FontBBox [-500 -300 1500 1000] /ItalicAngle 0 /Ascent 1024 /Descent -400 /CapHeight 660 /StemV 80 /FontFile2 {n} 0 R >>"
        ),
        format!(
            "<< /Type /Font /Subtype /CIDFontType2 /BaseFont /MatratestSans-Regular /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> /FontDescriptor {} 0 R /CIDToGIDMap /Identity /DW 1000 /W [{}] >>",
            n + 1,
            widths.join(" ")
        ),
        stream(
            "",
            &format!(
                "/CIDInit /ProcSet findresource begin 12 dict begin begincmap /CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def /CMapName /Test-UCS def /CMapType 2 def 1 begincodespacerange <0000> <FFFF> endcodespacerange {} beginbfchar\n{}\nendbfchar endcmap CMapName currentdict /CMap defineresource pop end end",
                rows.len(),
                rows.join("\n")
            ),
        ),
    ];
    let refs: Vec<&str> = objs.iter().map(String::as_str).collect();
    let resources = format!(
        "/Font << /F1 << /Type /Font /Subtype /Type0 /BaseFont /MatratestSans-Regular /Encoding /Identity-H /DescendantFonts [{} 0 R] /ToUnicode {} 0 R >> >> {extra}",
        n + 2,
        n + 3
    );
    let content = format!("BT /F1 40 Tf 20 100 Td {} ET", ops(&|c| format!("<{:04X}>", glyph(c).0)));
    pdf_with(&[PdfPage { resources, ..PdfPage::new(300.0, 200.0, &content) }], &refs, None)
}

fn open(bytes: &[u8]) -> ImportReport {
    import_with_report(bytes, &ImportOptions { text_as: TextAs::Text, ..Default::default() }).unwrap()
}

fn texts(d: &Document) -> Vec<String> {
    let mut v = vec![];
    d.walk(|n| {
        if let NodeKind::Text(t) = &n.kind {
            v.push(t.plain_text());
        }
    });
    v
}

fn outlines(d: &Document) -> usize {
    let mut n = 0;
    d.walk(|node| n += usize::from(is_glyph_outlines(node)));
    n
}

/// The file draws ककि as it is set: Ka, then the vowel sign before the Ka it follows in reading
/// order. Read glyph by glyph that is Ka, the sign, Ka, which reads किक: set as type, the sign
/// would move in front of the first Ka. The file's three glyphs stay as they are drawn.
#[test]
fn a_syllable_drawn_out_of_reading_order_keeps_the_files_glyphs() {
    let report = open(&page(&[KA, SIGN_I, KA], None));
    assert_eq!(texts(&report.document), [""; 0], "no type that draws something else");
    assert_eq!(outlines(&report.document), 1, "the glyphs, as one path");
    assert!(report.warnings.iter().any(|w| w == RESHAPED_TEXT_NOTE), "{:?}", report.warnings);
    assert!(report.warnings.iter().all(|w| !is_outlines_note(w) || w == RESHAPED_TEXT_NOTE), "{:?}", report.warnings);
}

/// Letters the file draws in reading order make type that draws the same: they stay type.
#[test]
fn letters_drawn_in_reading_order_stay_type() {
    let report = open(&page(&[KA, KA], None));
    assert_eq!(texts(&report.document), [format!("{KA}{KA}")]);
    assert_eq!(outlines(&report.document), 0);
    assert!(!report.warnings.iter().any(|w| is_outlines_note(w)), "{:?}", report.warnings);
}

/// A mark the file gives no advance (as fonts do for the signs they set over a letter) belongs to
/// the line of its letter in a font written horizontally: it is not a glyph of vertical type, set
/// as an object of its own. Here the line it stays on then reads कि, whose sign type would set
/// before the Ka, so the line keeps the file's glyphs, the mark with them.
#[test]
fn a_mark_without_an_advance_stays_with_its_line() {
    let report = open(&page(&[KA, SIGN_I], Some(0.0)));
    assert_eq!(texts(&report.document), [""; 0], "the mark is no type of its own");
    assert_eq!(outlines(&report.document), 1, "both glyphs, as one path");
}

/// The file's glyphs are outlines of one path when drawn alike, and a path each when a stroke
/// property other than the width (here the dash) differs between them: the first glyph's would
/// otherwise be drawn on all.
#[test]
fn outlined_glyphs_stroked_differently_stay_apart() {
    // Stroked (`1 Tr`) at the same width, one glyph solid and the next dashed.
    let alike = page_with(|id| format!("1 Tr 2 w {} Tj {} Tj {} Tj", id(KA), id(SIGN_I), id(KA)), "", None);
    let apart = page_with(|id| format!("1 Tr 2 w {} Tj [3 2] 0 d {} Tj [] 0 d {} Tj", id(KA), id(SIGN_I), id(KA)), "", None);
    assert_eq!(outlines(&open(&alike).document), 1, "drawn alike: one path");
    assert_eq!(outlines(&open(&apart).document), 3, "solid, dashed, solid: a path each");
}

/// Translucent glyphs the file fills one by one composite where they overlap, which one fill of
/// their joined outlines does not do: each keeps a path of its own.
#[test]
fn outlined_translucent_glyphs_stay_apart() {
    let bytes = page_with(|id| format!("/G gs {} Tj {} Tj {} Tj", id(KA), id(SIGN_I), id(KA)), "/ExtGState << /G << /ca 0.5 >> >>", None);
    let report = open(&bytes);
    assert_eq!(texts(&report.document), [""; 0]);
    assert_eq!(outlines(&report.document), 3, "a path each");
}
