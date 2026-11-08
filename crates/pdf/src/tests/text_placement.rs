//! #722: type from a PDF lands where the file draws it. Each file here is written from scratch
//! from the PDF specification (ISO 32000-1 §9, "Text": the text state, text-positioning and
//! text-showing operators, Type 0, Type 3 and TrueType fonts, vertical writing), with Source Sans 3
//! from this repo's assets embedded as its font program. What the file draws (its glyphs opened as
//! outlines) and what the type it opens as draws must match contour by contour.

use std::sync::Arc;

use kurbo::{BezPath, PathEl, Point, Rect, Shape};
use vectorcraft_doc::{Document, NodeKind, TextKind, TextObject};
use vectorcraft_testkit::pdf::{PdfPage, first_extra, pdf_with};
use vectorcraft_text::{FontDb, FontFace};

use crate::tests_import_fidelity::stream;
use crate::*;

const SOURCE_SANS: &[u8] = include_bytes!("../../../../assets/fonts/SourceSans3-Regular.ttf");

fn sans() -> Arc<FontFace> {
    FontDb::global().find_postscript("SourceSans3-Regular").expect("Source Sans 3 is built in")
}

/// Source Sans 3's advance of `c`, in thousandths of an em (as a PDF's widths give it).
fn width(c: char) -> f64 {
    let face = sans();
    (face.advance(face.glyph_for(c)) * 1000.0 / face.units_per_em()).round()
}

/// The font program of Source Sans 3 Regular as a stream (ASCII hex).
fn font_file() -> String {
    let hex: String = SOURCE_SANS.iter().map(|b| format!("{b:02X}")).collect::<String>() + ">";
    stream(&format!("/Length1 {} /Filter /ASCIIHexDecode", SOURCE_SANS.len()), &hex)
}

/// A font descriptor for the font program in object `file`.
fn descriptor(file: usize) -> String {
    format!(
        "<< /Type /FontDescriptor /FontName /SourceSans3-Regular /Flags 32 /FontBBox [-500 -300 1500 1000] /ItalicAngle 0 /Ascent 1024 /Descent -400 /CapHeight 660 /StemV 80 /FontFile2 {file} 0 R >>"
    )
}

/// A simple TrueType font (WinAnsi, Source Sans 3's widths) described by object `desc`.
fn simple(desc: usize) -> String {
    let widths: Vec<String> = (32u8..=126).map(|c| width(c as char).to_string()).collect();
    format!(
        "<< /Type /Font /Subtype /TrueType /BaseFont /SourceSans3-Regular /FirstChar 32 /LastChar 126 /Widths [{}] /FontDescriptor {desc} 0 R /Encoding /WinAnsiEncoding >>",
        widths.join(" ")
    )
}

/// The characters the Type 0 fonts map: printable ASCII, and symbols that stand upright in
/// vertical type and are narrower than an em.
fn cid_chars() -> impl Iterator<Item = char> {
    (32u8..=126).map(char::from).chain("§±×".chars())
}

/// `text` as a string of Source Sans 3's glyph ids, two bytes each (Identity-H or Identity-V,
/// CIDToGIDMap Identity).
fn gids(text: &str) -> String {
    let face = sans();
    let hex: String = text.chars().map(|c| format!("{:04X}", face.glyph_for(c))).collect();
    format!("<{hex}>")
}

/// A ToUnicode map of Source Sans 3's glyph ids for [`cid_chars`].
fn to_unicode() -> String {
    let face = sans();
    let rows: Vec<String> = cid_chars().map(|c| format!("<{:04X}> <{:04X}>", face.glyph_for(c), c as u32)).collect();
    let data = format!(
        "/CIDInit /ProcSet findresource begin 12 dict begin begincmap /CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def /CMapName /Test-UCS def /CMapType 2 def 1 begincodespacerange <0000> <FFFF> endcodespacerange {} beginbfchar\n{}\nendbfchar endcmap CMapName currentdict /CMap defineresource pop end end",
        rows.len(),
        rows.join("\n")
    );
    stream("", &data)
}

/// A CID-keyed TrueType font (the descendant of the Type 0 fonts) of Source Sans 3 with its
/// widths (/W), described by object `desc`.
fn cid_font(desc: usize) -> String {
    let face = sans();
    let w: Vec<String> = cid_chars().map(|c| format!("{} [{}]", face.glyph_for(c), width(c))).collect();
    format!(
        "<< /Type /Font /Subtype /CIDFontType2 /BaseFont /SourceSans3-Regular /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> /FontDescriptor {desc} 0 R /CIDToGIDMap /Identity /DW 1000 /W [{}] >>",
        w.join(" ")
    )
}

/// [`cid_font`] with vertical metrics (/W2: each glyph's position vector half its width across,
/// 880 up) and no /W (every width the default, an em).
fn cid_font_w2(desc: usize) -> String {
    let face = sans();
    let w2: Vec<String> = cid_chars().map(|c| format!("{} [-1000 {} 880]", face.glyph_for(c), width(c) / 2.0)).collect();
    format!(
        "<< /Type /Font /Subtype /CIDFontType2 /BaseFont /SourceSans3-Regular /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> /FontDescriptor {desc} 0 R /CIDToGIDMap /Identity /DW 1000 /DW2 [880 -1000] /W2 [{}] >>",
        w2.join(" ")
    )
}

/// A Type 3 font (object `first`, its glyph procedures the two after it) whose glyph space is
/// 1/`upm` of text space (FontMatrix [1/upm 0 0 1/upm 0 0]): `a` is half an em wide and draws a box
/// 0.4 em wide, `b` an em wide with a box of 0.9 em; both boxes are 0.7 em tall on the baseline.
fn type3(first: usize, upm: f64) -> Vec<String> {
    let s = 1.0 / upm;
    let (h, w) = (upm * 0.5, upm);
    let a = stream("", &format!("{h} 0 d0 0 0 {} {} re f", h * 0.8, upm * 0.7));
    let b = stream("", &format!("{w} 0 d0 0 0 {} {} re f", w * 0.9, upm * 0.7));
    let font = format!(
        "<< /Type /Font /Subtype /Type3 /FontBBox [0 0 {w} {w}] /FontMatrix [{s} 0 0 {s} 0 0] /CharProcs << /a {} 0 R /b {} 0 R >> /Encoding << /Type /Encoding /Differences [97 /a /b] >> /FirstChar 97 /LastChar 98 /Widths [{h} {w}] /Resources << >> >>",
        first + 1,
        first + 2
    );
    vec![font, a, b]
}

/// A 300 × 200 pt page drawing `content`, plus `entries` in the page dictionary, with fonts:
/// - `/F1`: Source Sans 3, embedded, a simple TrueType font;
/// - `/F4`: Source Sans 3 as a Type 0 font, Identity-H (two-byte glyph ids, see [`gids`]);
/// - `/F5`: the same, Identity-V (vertical writing);
/// - `/F6`: the same with vertical metrics (/W2) and no widths (/W), Identity-V;
/// - `/T3`, `/T30`: Type 3 fonts of 1000 and of 100 units an em;
/// - `/HV`: Helvetica, not embedded.
fn page(content: &str, entries: &str) -> Vec<u8> {
    sized_page(300.0, 200.0, content, entries)
}

/// [`page`], `w` × `h` pt.
fn sized_page(w: f64, h: f64, content: &str, entries: &str) -> Vec<u8> {
    let n = first_extra(1);
    let type0 = |enc: &str, cid: usize| {
        format!(
            "<< /Type /Font /Subtype /Type0 /BaseFont /SourceSans3-Regular /Encoding /{enc} /DescendantFonts [{cid} 0 R] /ToUnicode {} 0 R >>",
            n + 4
        )
    };
    let mut objs = vec![
        font_file(),                // n
        descriptor(n),              // n + 1
        simple(n + 1),              // n + 2: F1
        cid_font(n + 1),            // n + 3
        to_unicode(),               // n + 4
        type0("Identity-H", n + 3), // n + 5: F4
        type0("Identity-V", n + 3), // n + 6: F5
        cid_font_w2(n + 1),         // n + 7
        type0("Identity-V", n + 7), // n + 8: F6
    ];
    let t3 = n + objs.len();
    objs.extend(type3(t3, 1000.0));
    objs.extend(type3(t3 + 3, 100.0));
    let refs: Vec<&str> = objs.iter().map(String::as_str).collect();
    let resources = format!(
        "/Font << /F1 {} 0 R /F4 {} 0 R /F5 {} 0 R /F6 {} 0 R /T3 {t3} 0 R /T30 {} 0 R /HV << /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >> >>",
        n + 2,
        n + 5,
        n + 6,
        n + 8,
        t3 + 3
    );
    pdf_with(&[PdfPage { resources, entries: entries.into(), ..PdfPage::new(w, h, content) }], &refs, None)
}

fn open_as(bytes: &[u8], text_as: TextAs) -> Document {
    import_with_report(bytes, &ImportOptions { text_as, ..Default::default() }).unwrap().document
}

fn texts(d: &Document) -> Vec<TextObject> {
    let mut v = vec![];
    d.walk(|n| {
        if let NodeKind::Text(t) = &n.kind {
            v.push((**t).clone());
        }
    });
    v
}

fn plain(d: &Document) -> Vec<String> {
    texts(d).iter().map(TextObject::plain_text).collect()
}

/// The largest difference between two boxes' edges.
fn off(a: Rect, b: Rect) -> f64 {
    [a.x0 - b.x0, a.y0 - b.y0, a.x1 - b.x1, a.y1 - b.y1].iter().fold(0.0f64, |m, v| m.max(v.abs()))
}

/// The bounds of each contour of `bp` (each subpath).
fn contours_of(bp: &BezPath, out: &mut Vec<Rect>) {
    let mut cur = BezPath::new();
    for el in bp.elements() {
        if matches!(el, PathEl::MoveTo(_)) && cur.elements().len() > 1 {
            out.push(std::mem::take(&mut cur).bounding_box());
        }
        if matches!(el, PathEl::MoveTo(_)) {
            cur = BezPath::new();
        }
        cur.push(*el);
    }
    if cur.elements().len() > 1 {
        out.push(cur.bounding_box());
    }
}

/// The contours `d` draws: its paths' and its type's glyphs', in document space.
fn contours(d: &Document) -> Vec<Rect> {
    let mut out = vec![];
    d.walk(|n| match &n.kind {
        NodeKind::Path { path, .. } => contours_of(&path.to_bezpath(), &mut out),
        NodeKind::Text(t) => {
            for g in vectorcraft_text::layout(FontDb::global(), t).glyphs {
                contours_of(&(t.xf * g.outline), &mut out);
            }
        }
        _ => {}
    });
    out
}

/// How far (points) the glyphs of the type `bytes` opens as are from where the file draws them:
/// the furthest any contour the file draws is from the nearest one the type draws, or the other
/// way round.
fn displacement(bytes: &[u8]) -> f64 {
    let drawn = contours(&open_as(bytes, TextAs::Outlines));
    let typed = contours(&open_as(bytes, TextAs::Text));
    assert!(!drawn.is_empty() && !typed.is_empty(), "the file draws glyphs");
    let worst = |a: &[Rect], b: &[Rect]| a.iter().map(|r| b.iter().map(|s| off(*r, *s)).fold(f64::INFINITY, f64::min)).fold(0.0, f64::max);
    worst(&drawn, &typed).max(worst(&typed, &drawn))
}

/// The type `content` opens as lands where the file draws it, as `want` (its objects' text).
#[track_caller]
fn lands(content: &str, entries: &str, want: &[&str]) {
    let bytes = page(content, entries);
    assert_eq!(plain(&open_as(&bytes, TextAs::Text)), want, "{content}");
    let d = displacement(&bytes);
    assert!(d < 0.02, "{content}: glyphs {d:.3} pt from where the file draws them");
}

/// Three lines wrapped in a frame (each wrapped line ends in a space).
const PARAGRAPH: &str = "(The quick brown fox ) Tj T* (jumps over the lazy ) Tj T* (dog and runs away.) Tj";
const PARAGRAPH_TEXT: &str = "The quick brown fox jumps over the lazy dog and runs away.";

/// The text-positioning operators and the text matrix under any CTM: lines start where the file
/// starts them, the glyphs of each where it draws them.
#[test]
fn text_positioning_and_the_ctm_put_type_where_the_file_draws_it() {
    let lines = ["Line one", "Line two", "Line three"];
    for (content, entries, want) in [
        ("BT /F1 24 Tf 20 50 Td (Hello World) Tj ET", "", &["Hello World"][..]),
        ("BT /F1 1 Tf 24 0 0 24 20 50 Tm (Hello World) Tj ET", "", &["Hello World"]),
        ("q 2 0 0 2 10 10 cm BT /F1 12 Tf 10 20 Td (Hello World) Tj ET Q", "", &["Hello World"]),
        // A y-down CTM with a text matrix that turns the type upright again.
        ("q 1 0 0 -1 0 200 cm BT /F1 12 Tf 1 0 0 -1 20 50 Tm (Hello World) Tj ET Q", "", &["Hello World"]),
        ("BT /F1 12 Tf 14 TL 20 150 Td (Line one) Tj T* (Line two) Tj T* (Line three) Tj ET", "", &lines),
        ("BT /F1 12 Tf 20 150 Td (Line one) Tj 0 -16 TD (Line two) Tj T* (Line three) Tj ET", "", &lines),
        // ' moves to the next line; " also sets word and character spacing.
        ("BT /F1 12 Tf 14 TL 20 150 Td (Line one) Tj (Line two) ' 2 1 (Line three) \" ET", "", &lines),
        ("BT /F1 18 Tf 0.866 0.5 -0.5 0.866 40 40 Tm (Turned line) Tj ET", "", &["Turned line"]),
        ("BT /F1 1 Tf 30 0 0 15 20 50 Tm (Squashed) Tj ET", "", &["Squashed"]),
        ("q 1 0 0 0.5 0 0 cm BT /F1 20 Tf 20 100 Td (Squashed) Tj ET Q", "", &["Squashed"]),
        ("BT /F1 24 Tf 60 80 Td (Cropped) Tj ET", "/CropBox [40 30 260 180]", &["Cropped"]),
        ("BT /F1 24 Tf 20 50 Td (Turned page) Tj ET", "/Rotate 90", &["Turned page"]),
    ] {
        lands(content, entries, want);
    }
}

/// Horizontal scaling, character and word spacing, and `TJ`'s adjustments: each glyph keeps the
/// place the file gave it (they used to be evened out over the line, and laid out with the font's
/// kerning where the file had none, so the glyphs between the ends slid).
#[test]
fn spacing_and_scaling_keep_each_glyph_where_the_file_put_it() {
    for (content, want) in [
        ("BT /F1 20 Tf 50 Tz 20 50 Td (Narrow type) Tj ET", "Narrow type"),
        ("BT /F1 20 Tf 150 Tz 20 50 Td (Wide type) Tj ET", "Wide type"),
        ("BT /F1 16 Tf 2 Tc 20 50 Td (Spaced out) Tj ET", "Spaced out"),
        ("BT /F1 16 Tf -0.8 Tc 20 50 Td (Tight type here) Tj ET", "Tight type here"),
        ("BT /F1 16 Tf 6 Tw 20 50 Td (Word space here) Tj ET", "Word space here"),
        ("BT /F1 16 Tf 1.5 Tc 80 Tz 20 50 Td (Spaced and narrow) Tj ET", "Spaced and narrow"),
        ("BT /F1 16 Tf 20 50 Td [(A) 120 (V) -80 (A) 300 (Tower)] TJ ET", "AVATower"),
        ("BT /F1 16 Tf 20 50 Td (AVATAR WAVE) Tj ET", "AVATAR WAVE"),
    ] {
        lands(content, "", &[want]);
    }
    // Two-byte codes of a Type 0 font alike (word spacing applies only to single-byte code 32,
    // ISO 32000-1 §9.3.3: none here).
    for content in ["BT /F4 24 Tf 70 Tz 3 Ts 20 50 Td <> Tj ET", "BT /F4 24 Tf 9 Tw 2 Tc 20 50 Td <> Tj ET"] {
        lands(&content.replace("<>", &gids("Hello World")), "", &["Hello World"]);
    }
}

/// Word spacing stays on the spaces, as tracking of their own; evenly spaced letters keep one
/// tracking value for the line, and a file that didn't kern opens with kerning off.
#[test]
fn word_spacing_is_tracking_on_the_spaces_and_unkerned_type_opens_unkerned() {
    let d = open_as(&page("BT /F1 16 Tf 6 Tw 20 50 Td (Word space here) Tj ET", ""), TextAs::Text);
    let t = texts(&d);
    let letters: Vec<f64> = t[0].runs.iter().filter(|r| r.text != " ").map(|r| r.style.tracking).collect();
    let spaces: Vec<f64> = t[0].runs.iter().filter(|r| r.text == " ").map(|r| r.style.tracking).collect();
    assert_eq!(spaces.len(), 2, "{:?}", t[0].runs);
    for s in &spaces {
        assert!((s - 375.0).abs() < 0.5, "6 pt at 16 pt is 375/1000 em: {spaces:?}");
    }
    assert!(letters.iter().all(|l| l.abs() < 0.5), "the letters as the font spaces them: {letters:?}");
    // "AV", "VA", "WA" kern in Source Sans 3; this file set them on their widths alone.
    let d = open_as(&page("BT /F1 16 Tf 20 50 Td (AVATAR WAVE) Tj ET", ""), TextAs::Text);
    let t = texts(&d);
    assert_eq!(t[0].runs.len(), 1, "{:?}", t[0].runs);
    assert_eq!(t[0].runs[0].style.kerning, Some(0.0), "kerning off, as the file set it");
    assert!(t[0].runs[0].style.tracking.abs() < 0.5);
}

/// Type the font's kerning spaced (as editors write it) keeps the font's kerning.
#[test]
fn kerned_type_keeps_the_fonts_kerning() {
    let bytes = crate::tests_live_text::text_pdf(&[(20.0, 50.0, 24.0, "AVATAR WAVE")], false);
    let d = open_as(&bytes, TextAs::Text);
    let t = texts(&d);
    assert_eq!(t.len(), 1);
    assert!(t[0].runs.iter().all(|r| r.style.kerning.is_none()), "{:?}", t[0].runs);
    assert!(displacement(&bytes) < 0.02);
}

/// Text rise (`Ts`): a superscript or subscript stays in its line as baseline shift (it used to be
/// pulled onto the baseline within 0.15 em of it, or split off as an object of its own), and a line
/// that starts raised keeps the baseline of the rest of its text.
#[test]
fn raised_and_lowered_characters_stay_in_their_line_shifted_off_its_baseline() {
    for (content, want) in [
        ("BT /F1 12 Tf 20 50 Td 5 Ts (Raised line) Tj ET", "Raised line"),
        ("BT /F1 12 Tf 20 50 Td (E = mc) Tj 5 Ts (2) Tj 0 Ts ( here) Tj ET", "E = mc2 here"),
        ("BT /F1 12 Tf 20 50 Td (H) Tj -3 Ts (2) Tj 0 Ts (O water) Tj ET", "H2O water"),
        ("BT /F1 12 Tf 20 50 Td (H) Tj 1.5 Ts (2) Tj 0 Ts (O water) Tj ET", "H2O water"),
        ("BT /F1 12 Tf 20 50 Td (CO) Tj -1.2 Ts /F1 8 Tf (2) Tj 0 Ts /F1 12 Tf ( gas) Tj ET", "CO2 gas"),
        ("BT /F1 12 Tf 20 50 Td 1.5 Ts (1) Tj 0 Ts (Footnote text) Tj ET", "1Footnote text"),
    ] {
        lands(content, "", &[want]);
    }
    // Under a sheared text matrix (slanted type) text rise moves a glyph along the slant, as the
    // type's slant draws its baseline shift: the shifted glyphs land where the file draws them.
    for (content, want) in [
        ("BT /F1 12 Tf 1 0 0.2 1 20 50 Tm (E = mc) Tj 5 Ts (2) Tj ET", "E = mc2"),
        ("BT /F1 12 Tf 1 0 0.2 1 20 50 Tm (E = mc) Tj 5 Ts (2) Tj 0 Ts ( here) Tj ET", "E = mc2 here"),
        ("BT /F1 12 Tf 1 0 -0.3 1 20 50 Tm (H) Tj -3 Ts (2) Tj 0 Ts (O water) Tj ET", "H2O water"),
        ("BT /F1 12 Tf 1 0 0.2 1 20 50 Tm 1.5 Ts (1) Tj 0 Ts (Footnote text) Tj ET", "1Footnote text"),
        ("BT /F1 12 Tf 1 0 0.2 1 20 50 Tm 4 Ts (Raised) Tj ET", "Raised"),
    ] {
        lands(content, "", &[want]);
    }
    let t = texts(&open_as(&page("BT /F1 12 Tf 20 50 Td 1.5 Ts (1) Tj 0 Ts (Footnote text) Tj ET", ""), TextAs::Text));
    let origin = t[0].xf * Point::ZERO;
    assert!((origin.y - 150.0).abs() < 1e-6, "on the text's baseline, not the mark's: {origin:?}");
    assert_eq!(t[0].runs[0].text, "1");
    assert!((t[0].runs[0].style.baseline_shift - 1.5).abs() < 1e-6, "{:?}", t[0].runs[0].style.baseline_shift);
    assert!(t[0].runs[1..].iter().all(|r| r.style.baseline_shift == 0.0));
}

/// Lines wrapped in a frame (area type, as an editor writes it: each line placed with `T*` or `Td`)
/// come back as one area type object whose glyphs are where the lines' were: aligned left, or
/// justified with word spacing (`Tw`) on each wrapped line.
#[test]
fn text_boxes_open_where_their_lines_were() {
    for content in [
        format!("BT /F1 12 Tf 14 TL 20 150 Td {PARAGRAPH} ET"),
        format!("BT /F1 12 Tf 14 TL 4 Ts 80 Tz 20 150 Td {PARAGRAPH} ET"),
        format!("q 1.5 0 0 1.5 5 5 cm BT /F1 8 Tf 10 TL 10 90 Td {PARAGRAPH} ET Q"),
        format!("BT /F1 12 Tf 14 TL 0.866 0.5 -0.5 0.866 40 40 Tm {PARAGRAPH} ET"),
    ] {
        let t = texts(&open_as(&page(&content, ""), TextAs::Text));
        assert_eq!(t.len(), 1, "{content}: one text box");
        assert!(matches!(t[0].kind, TextKind::Area { .. }), "{content}");
        lands(&content, "", &[PARAGRAPH_TEXT]);
    }
    // Justified: every wrapped line spaced out to the frame's width with word spacing.
    let lines = ["The quick brown fox ", "jumps over the lazy ", "dog and runs away."];
    let (size, frame) = (12.0, 110.0);
    let mut content = format!("BT /F1 {size} Tf 20 170 Td ");
    for (i, l) in lines.iter().enumerate() {
        let natural = l.trim_end().chars().map(width).sum::<f64>() * size / 1000.0;
        let spaces = l.trim_end().matches(' ').count() as f64;
        let tw = if i + 1 < lines.len() { (frame - natural) / spaces } else { 0.0 };
        if i > 0 {
            content += "0 -14.4 Td ";
        }
        content += &format!("{tw:.4} Tw ({l}) Tj ");
    }
    content += "ET";
    let t = texts(&open_as(&page(&content, ""), TextAs::Text));
    assert_eq!(t.len(), 1, "one text box: {:?}", t.iter().map(TextObject::plain_text).collect::<Vec<_>>());
    assert_eq!(t[0].para.justify, vectorcraft_doc::Justify::JustifyLeft);
    lands(&content, "", &[PARAGRAPH_TEXT]);
}

/// Vertical writing (Identity-V): the file puts each glyph's origin half its width (/W) left of
/// the column (ISO 32000-1 §9.7.4.3); glyphs narrower than an em stay centred on it.
#[test]
fn vertical_glyphs_narrower_than_an_em_stay_on_their_column() {
    lands(&format!("BT /F5 24 Tf 150 180 Td {} Tj ET", gids("§±×")), "", &["§±×"]);
}

/// Type 3 glyphs (drawn by their procedures, kept as outlines) are scaled by the font's matrix:
/// the same boxes at 1000 and at 100 units an em.
#[test]
fn type3_glyphs_follow_the_font_matrix() {
    for font in ["T3", "T30"] {
        let d = open_as(&page(&format!("BT /{font} 20 Tf 20 50 Td (ab) Tj ET"), ""), TextAs::Text);
        assert!(texts(&d).is_empty(), "Type 3 text keeps its outlines");
        let mut boxes = contours(&d);
        boxes.sort_by(|a, b| a.x0.total_cmp(&b.x0));
        // `a` at x 20 (0.4 em = 8 pt wide), `b` after its half-em advance; 0.7 em = 14 pt tall,
        // standing on the baseline at y 150 (200 - 50).
        let want = [Rect::new(20.0, 136.0, 28.0, 150.0), Rect::new(30.0, 136.0, 48.0, 150.0)];
        assert_eq!(boxes.len(), 2, "{font}: {boxes:?}");
        for (b, w) in boxes.iter().zip(want) {
            assert!(off(*b, w) < 1e-3, "{font}: {b:?} vs {w:?}");
        }
    }
}

/// A font that isn't embedded or installed is set in a stand-in: the line still starts where
/// the file starts it, on its baseline, and is as long as the file's.
#[test]
fn type_in_a_stand_in_font_still_starts_where_the_file_starts_it() {
    let t = texts(&open_as(&page("BT /HV 12 Tf 6 Tw 20 50 Td 5 Ts (Raised word line) Tj ET", ""), TextAs::Text));
    assert_eq!(t.len(), 1);
    assert_eq!(t[0].plain_text(), "Raised word line");
    let origin = t[0].xf * Point::ZERO;
    assert!((origin.x - 20.0).abs() < 1e-6 && (origin.y - 145.0).abs() < 1e-6, "{origin:?}");
}

/// Type on a gentle arc: each glyph turned a little from the one before, the first few within a
/// hair of the first's direction. They sit on the curve; none is shifted off it as well (which
/// moved them off the curve by the shift twice over).
#[test]
fn type_on_a_gentle_arc_sits_on_its_curve_unshifted() {
    let text = "Gentle arc of type here";
    let (size, r) = (12.0, 450.0);
    let (cx, cy) = (150.0, -300.0);
    let mut phi = 100f64.to_radians();
    let mut content = format!("BT /F1 {size} Tf ");
    for c in text.chars() {
        let a = phi - std::f64::consts::FRAC_PI_2;
        let (x, y) = (cx + r * phi.cos(), cy + r * phi.sin());
        let esc = if c == ' ' { " ".to_string() } else { c.to_string() };
        content += &format!("{:.6} {:.6} {:.6} {:.6} {x:.4} {y:.4} Tm ({esc}) Tj ", a.cos(), a.sin(), -a.sin(), a.cos());
        phi -= width(c) * size / 1000.0 / r;
    }
    content += "ET";
    let t = texts(&open_as(&page(&content, ""), TextAs::Text));
    assert_eq!(t.len(), 1, "{:?}", t.iter().map(TextObject::plain_text).collect::<Vec<_>>());
    assert_eq!(t[0].plain_text(), text);
    assert!(matches!(t[0].kind, TextKind::OnPath { .. }), "type on a path");
    assert!(t[0].runs.iter().all(|r| r.style.baseline_shift == 0.0), "{:?}", t[0].runs);
    let d = displacement(&page(&content, ""));
    assert!(d < 0.05, "glyphs {d:.3} pt off the curve");
}

/// A line with as many raised characters as level ones keeps the baseline of its first glyph:
/// "m²" sits on the m's baseline, the 2 shifted up.
#[test]
fn a_tie_keeps_the_first_glyphs_baseline() {
    let content = "BT /F1 12 Tf 20 50 Td (m) Tj 5 Ts (2) Tj ET";
    lands(content, "", &["m2"]);
    let t = texts(&open_as(&page(content, ""), TextAs::Text));
    let origin = t[0].xf * Point::ZERO;
    assert!((origin.y - 150.0).abs() < 1e-6, "on the m's baseline: {origin:?}");
    assert_eq!(t[0].runs.len(), 2, "{:?}", t[0].runs);
    assert_eq!(t[0].runs[0].style.baseline_shift, 0.0);
    assert!((t[0].runs[1].style.baseline_shift - 5.0).abs() < 1e-6, "{:?}", t[0].runs[1].style);
}

/// The cells of a table's row whose baselines differ a little (a third of an em) stay objects of
/// their own: only a character that follows on from the one before is raised in its line.
#[test]
fn table_cells_a_little_apart_stay_apart() {
    for content in [
        "BT /F1 12 Tf 20 50 Td (Cell one) Tj 60 3.6 Td (Cell two) Tj ET",
        "BT /F1 12 Tf 20 50 Td (Cell one) Tj 60 -3.6 Td (Cell two) Tj ET",
        "BT /F1 12 Tf 20 50 Td (Cell one) Tj 3.6 Ts 60 0 Td (Cell two) Tj ET",
    ] {
        lands(content, "", &["Cell one", "Cell two"]);
    }
}

/// Vertical writing with the font's own vertical metrics (/W2): each glyph's position vector puts
/// the column's centre where the font says (ISO 32000-1 §9.7.4.3), not half the default width.
#[test]
fn vertical_type_takes_the_fonts_position_vectors() {
    lands(&format!("BT /F6 24 Tf 150 180 Td {} Tj ET", gids("§±×")), "", &["§±×"]);
}

/// A vertical font's metrics as the file gives them: /W2 first, then half the width from /W, then
/// half /DW; glyphs mapped back to their CIDs through a /CIDToGIDMap stream; a CFF font's glyphs
/// (whose charset maps them) take /DW's.
#[test]
fn vertical_font_metrics_are_read_from_w2_w_and_dw() {
    use craft_pdf::syntax::Pdf;
    use craft_pdf::syntax::object::{Dict, ObjectIdentifier};
    let n = first_extra(1);
    // CID → glyph: 10 → 5, 20 → 6, 30 → 7, 31 → 9, 40 → 8 (others 0).
    let mut map = [0u16; 41];
    for (cid, gid) in [(10, 5), (20, 6), (30, 7), (31, 9), (40, 8)] {
        map[cid] = gid;
    }
    let hex: String = map.iter().map(|g| format!("{g:04X}")).collect::<String>() + ">";
    let descendant = |subtype: &str, gid_map: &str| {
        format!(
            "<< /Type /Font /Subtype /{subtype} /BaseFont /Test /DW 900 /W [10 20 600 30 [400 500]] /W2 [10 [-1000 222 880] 40 41 -1000 333 880] {gid_map} >>"
        )
    };
    let type0 = |enc: &str, d: usize| format!("<< /Type /Font /Subtype /Type0 /BaseFont /Test /Encoding /{enc} /DescendantFonts [{d} 0 R] >>");
    let objs = [
        stream("/Filter /ASCIIHexDecode", &hex),                      // n
        descendant("CIDFontType2", &format!("/CIDToGIDMap {n} 0 R")), // n + 1
        type0("Identity-V", n + 1),                                   // n + 2
        descendant("CIDFontType2", ""),                               // n + 3
        type0("Identity-V", n + 3),                                   // n + 4
        descendant("CIDFontType0", ""),                               // n + 5
        type0("Identity-V", n + 5),                                   // n + 6
        type0("Identity-H", n + 3),                                   // n + 7
    ];
    let refs: Vec<&str> = objs.iter().map(String::as_str).collect();
    let bytes = pdf_with(&[PdfPage::new(100.0, 100.0, "")], &refs, None);
    let pdf = Pdf::new(Arc::new(bytes)).unwrap();
    let font = |k: usize| pdf.xref().get::<Dict<'_>>(ObjectIdentifier::new((n + k) as i32, 0)).unwrap();
    let mapped = crate::import_scan::vertical_metrics(&font(2), true).expect("vertical");
    for (gid, want) in [(5, 222.0), (6, 300.0), (7, 200.0), (9, 250.0), (8, 333.0), (100, 450.0)] {
        assert_eq!(mapped.centre(gid), want, "glyph {gid}");
    }
    // CIDToGIDMap /Identity: glyph ids are CIDs.
    let identity = crate::import_scan::vertical_metrics(&font(4), true).expect("vertical");
    for (gid, want) in [(10, 222.0), (15, 300.0), (31, 250.0), (41, 333.0), (42, 450.0)] {
        assert_eq!(identity.centre(gid), want, "glyph {gid}");
    }
    // A font the file doesn't carry, or a CFF one: its glyphs can't be told by CID.
    let stand_in = crate::import_scan::vertical_metrics(&font(4), false).expect("vertical");
    let cff = crate::import_scan::vertical_metrics(&font(6), true).expect("vertical");
    assert_eq!((stand_in.centre(10), cff.centre(10)), (450.0, 450.0));
    assert!(crate::import_scan::vertical_metrics(&font(7), true).is_none(), "horizontal");
}

/// Area type whose lines the file spaced a hair unevenly (`TJ` adjustments of a few thousandths
/// of an em) still comes back as one text box.
#[test]
fn text_boxes_with_a_hair_of_uneven_spacing_still_open_as_one() {
    let content =
        "BT /F1 12 Tf 14 TL 20 150 Td [(The qu) -4 (ick brown fox )] TJ T* [(jumps o) 5 (ver the) -3 ( lazy )] TJ T* (dog and runs away.) Tj ET";
    let t = texts(&open_as(&page(content, ""), TextAs::Text));
    assert_eq!(t.len(), 1, "one text box: {:?}", t.iter().map(TextObject::plain_text).collect::<Vec<_>>());
    assert!(matches!(t[0].kind, TextKind::Area { .. }));
    assert_eq!(t[0].plain_text(), PARAGRAPH_TEXT);
    assert!(displacement(&page(content, "")) < 0.1);
}

/// Word spacing the same on every line of a text box, its last line too, is the type's: it comes
/// back as one text box, aligned left, its spaces tracked apart.
#[test]
fn text_boxes_word_spaced_throughout_open_as_one() {
    let content = format!("BT /F1 12 Tf 14 TL 2 Tw 20 150 Td {PARAGRAPH} ET");
    let t = texts(&open_as(&page(&content, ""), TextAs::Text));
    assert_eq!(t.len(), 1, "one text box: {:?}", t.iter().map(TextObject::plain_text).collect::<Vec<_>>());
    assert!(matches!(t[0].kind, TextKind::Area { .. }));
    assert_eq!(t[0].para.justify, vectorcraft_doc::Justify::Left);
    let spaces: Vec<f64> = t[0].runs.iter().filter(|r| r.text.trim().is_empty()).map(|r| r.style.tracking).collect();
    assert!(!spaces.is_empty() && spaces.iter().all(|s| (s - 2.0 / 12.0 * 1000.0).abs() < 1.0), "{:?}", t[0].runs);
    lands(&content, "", &[PARAGRAPH_TEXT]);
}

/// A font the file names but doesn't carry, drawn with the installed one of that name, whose
/// advances (2048 units an em) its widths give only to the thousandth of an em: the spacing the
/// file set is measured from its widths, so letters a hair off the font's widths still take one
/// tracking value, not each character its own (a run each). Inter is built in, so this doesn't
/// depend on the fonts installed.
#[test]
fn widths_rounded_off_the_installed_fonts_advances_keep_one_tracking() {
    let inter = FontDb::global().find_postscript("Inter-Regular").expect("Inter is built in");
    let upm = inter.units_per_em();
    assert!(upm != 1000.0, "advances finer than the file's widths: {upm}");
    let widths: Vec<String> = (32u8..=126).map(|c| (inter.advance(inter.glyph_for(c as char)) * 1000.0 / upm).round().to_string()).collect();
    let font = format!(
        "<< /Type /Font /Subtype /TrueType /BaseFont /Inter-Regular /FirstChar 32 /LastChar 126 /Widths [{}] /Encoding /WinAnsiEncoding >>",
        widths.join(" ")
    );
    let resources = format!("/Font << /IN {} 0 R >>", first_extra(1));
    let content = "BT /IN 12 Tf 20 50 Td [(Next) -1000 (line here)] TJ ET";
    let bytes = pdf_with(&[PdfPage { resources, ..PdfPage::new(300.0, 200.0, content) }], &[&font], None);
    let t = texts(&open_as(&bytes, TextAs::Text));
    assert_eq!(t.len(), 1);
    let runs: Vec<&str> = t[0].runs.iter().map(|r| r.text.as_str()).collect();
    assert_eq!(runs, ["Next", " ", "line here"], "{:?}", t[0].runs);
    assert!(t[0].runs[0].style.tracking.abs() < 0.5 && t[0].runs[2].style.tracking.abs() < 0.5, "{:?}", t[0].runs);
    assert!(t[0].runs[1].style.tracking > 500.0, "the em's gap on the space: {:?}", t[0].runs);
}

/// A long line whose file widths are each a little off the font's advances (0.8% wider or
/// narrower, within the 1% that still counts as the same font) opens with its glyphs where the
/// file draws them, in one run: the differences add up along the line (with only the spacing the
/// file set, none, the glyphs would end points away from the file's), so the line takes their
/// share after every character. A line the font kerns (laid out without its kerning, as the file
/// sets it) and one it doesn't (laid out as the type is). Source Sans 3 is embedded: what the file
/// draws and what the type draws are the same outlines, so their contours tell where each glyph is.
#[test]
fn a_long_line_of_widths_a_little_off_the_fonts_advances_lands_in_one_run() {
    let n = first_extra(1);
    let resources = format!("/Font << /F1 {} 0 R >>", n + 2);
    for scale in [1.008, 0.992] {
        let widths: Vec<String> = (32u8..=126).map(|c| (width(c as char) * scale).round().to_string()).collect();
        let font = format!(
            "<< /Type /Font /Subtype /TrueType /BaseFont /SourceSans3-Regular /FirstChar 32 /LastChar 126 /Widths [{}] /FontDescriptor {} 0 R /Encoding /WinAnsiEncoding >>",
            widths.join(" "),
            n + 1
        );
        for line in [
            "Gather the quick brown fox over the lazy dogs while the waves roll in",
            "minimum minimum minimum minimum minimum minimum minimum minimum",
        ] {
            let content = format!("BT /F1 12 Tf 10 50 Td ({line}) Tj ET");
            let page = PdfPage { resources: resources.clone(), ..PdfPage::new(600.0, 200.0, &content) };
            let bytes = pdf_with(&[page], &[&font_file(), &descriptor(n), &font], None);
            let t = texts(&open_as(&bytes, TextAs::Text));
            assert_eq!(t.iter().map(TextObject::plain_text).collect::<Vec<_>>(), [line]);
            assert_eq!(t[0].runs.len(), 1, "{scale} {line:?}: {:?}", t[0].runs);
            let d = displacement(&bytes);
            assert!(d < 12.0 * 0.02, "{scale} {line:?}: glyphs {d:.3} pt from where the file draws them");
        }
    }
}

/// A text-heavy page: `columns` columns of `rows` lines of 10 pt type, alternately plain lines and
/// paragraphs wrapped in a frame (left aligned, and justified with word spacing).
fn text_heavy(columns: usize, rows: usize) -> Vec<u8> {
    const WORDS: [&str; 12] = ["Gather", "the", "quick", "brown", "fox", "over", "lazy", "dogs", "while", "AVATAR", "waves", "kerning"];
    let mut content = String::new();
    for c in 0..columns {
        let x = 20.0 + c as f64 * 200.0;
        content += &format!("BT /F1 10 Tf 12 TL {x} {} Td ", 40.0 + rows as f64 * 12.0);
        for r in 0..rows {
            let words: Vec<&str> = (0..6).map(|k| WORDS[(c * 7 + r * 5 + k) % WORDS.len()]).collect();
            let wrapped = c % 2 == 1 && r % 6 != 5;
            let tw = if c % 4 == 3 && wrapped { 1.5 } else { 0.0 };
            content += &format!("{tw} Tw ({}{}) Tj T* ", words.join(" "), if wrapped { " " } else { "" });
        }
        content += "ET ";
    }
    sized_page(40.0 + columns as f64 * 200.0, 80.0 + rows as f64 * 12.0, &content, "")
}

/// How long a text-heavy file takes to open as type (run by hand: `-- --ignored --nocapture`).
#[test]
#[ignore]
fn perf_text_heavy_open() {
    let bytes = text_heavy(20, 200);
    for _ in 0..2 {
        let start = std::time::Instant::now();
        let d = open_as(&bytes, TextAs::Text);
        let took = start.elapsed();
        let n = texts(&d).len();
        let start = std::time::Instant::now();
        let _ = open_as(&bytes, TextAs::Outlines);
        println!("4000 lines: {n} text objects in {took:?} (as outlines: {:?})", start.elapsed());
    }
}
