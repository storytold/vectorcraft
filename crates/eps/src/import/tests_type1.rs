//! Type 1 fonts embedded in the file: their encrypted part (`eexec`) is read for their glyphs
//! (Adobe Type 1 Font Format, chapters 6 and 7), so their type keeps the font's own widths, and
//! glyphs that stand for no character are drawn from their outlines. The font here is written
//! from scratch: two square and triangle glyphs in hand-made charstrings.

use vectorcraft_doc::{NodeKind, TextObject};

use super::tests::{all, bounds, check, program_warnings, read};
use crate::import::import;

/// A Type 1 charstring number (§6.2).
fn num(v: i32) -> Vec<u8> {
    match v {
        -107..=107 => vec![(v + 139) as u8],
        108..=1131 => vec![((v - 108) / 256 + 247) as u8, ((v - 108) % 256) as u8],
        -1131..=-108 => vec![((-v - 108) / 256 + 251) as u8, ((-v - 108) % 256) as u8],
        _ => [vec![255], v.to_be_bytes().to_vec()].concat(),
    }
}

/// A charstring (`hsbw` 13, `rmoveto` 21, `rlineto` 5, `closepath` 9, `endchar` 14).
fn cs(parts: &[(&[i32], u8)]) -> Vec<u8> {
    parts.iter().flat_map(|(args, op)| args.iter().flat_map(|a| num(*a)).chain([*op])).collect()
}

/// `plain` encrypted with key `r` after 4 leading bytes (§7.1: 55665 for the encrypted part,
/// 4330 for charstrings).
fn encrypt(plain: &[u8], mut r: u16) -> Vec<u8> {
    [0u8; 4]
        .iter()
        .chain(plain)
        .map(|&p| {
            let c = p ^ (r >> 8) as u8;
            r = (u16::from(c).wrapping_add(r)).wrapping_mul(52845).wrapping_add(22719);
            c
        })
        .collect()
}

/// Font `/TestOne`: A a square (100 to 500 across, 500 high) 600 wide, B a triangle (500
/// across, 400 high) 500 wide, and the triangle again under the name `g1`, which says nothing,
/// at code 67 (C). Its encoding is `encoding` (a PostScript expression); its encrypted part
/// written in hexadecimal when `hex`.
fn font(encoding: &str, hex: bool) -> Vec<u8> {
    font_with(encoding, hex, &[], None)
}

/// [`font`] with subroutines `subrs` and, when given, charstring `a` for A.
fn font_with(encoding: &str, hex: bool, subrs: &[Vec<u8>], a: Option<Vec<u8>>) -> Vec<u8> {
    let square = a.unwrap_or_else(|| cs(&[(&[0, 600], 13), (&[100, 0], 21), (&[400, 0], 5), (&[0, 500], 5), (&[-400, 0], 5), (&[], 9), (&[], 14)]));
    let triangle = cs(&[(&[0, 500], 13), (&[0, 0], 21), (&[500, 0], 5), (&[-250, 400], 5), (&[], 9), (&[], 14)]);
    let notdef = cs(&[(&[0, 250], 13), (&[], 14)]);
    let mut private = b"dup /Private 8 dict dup begin\n/RD {string currentfile exch readstring pop} executeonly def\n\
        /ND {noaccess def} executeonly def\n/NP {noaccess put} executeonly def\n/lenIV 4 def\n/password 5839 def\n\
        /MinFeature {16 16} def\n"
        .to_vec();
    private.extend(format!("/Subrs {} array\n", subrs.len()).into_bytes());
    for (i, subr) in subrs.iter().enumerate() {
        let e = encrypt(subr, 4330);
        private.extend(format!("dup {i} {} RD ", e.len()).into_bytes());
        private.extend(e);
        private.extend(b" NP\n");
    }
    private.extend(b"readonly def\n2 index /CharStrings 4 dict dup begin\n");
    for (name, glyph) in [(".notdef", &notdef), ("A", &square), ("B", &triangle), ("g1", &triangle)] {
        let e = encrypt(glyph, 4330);
        private.extend(format!("/{name} {} RD ", e.len()).into_bytes());
        private.extend(e);
        private.extend(b" ND\n");
    }
    private.extend(b"end\nend\nreadonly put\nnoaccess put\ndup /FontName get exch definefont pop\nmark currentfile closefile\n");
    let encrypted = encrypt(&private, 55665);
    let mut out = format!(
        "%!FontType1-1.0: TestOne 001\n12 dict begin\n/FontName /TestOne def /FontType 1 def /PaintType 0 def\n\
         /FontMatrix [0.001 0 0 0.001 0 0] readonly def /FontBBox {{0 0 1000 1000}} readonly def\n\
         /Encoding {encoding} def\ncurrentdict end\ncurrentfile eexec\n"
    )
    .into_bytes();
    if hex {
        for line in encrypted.chunks(32) {
            out.extend(line.iter().map(|b| format!("{b:02x}")).collect::<String>().into_bytes());
            out.push(b'\n');
        }
    } else {
        out.extend(encrypted);
        out.push(b'\n');
    }
    for _ in 0..8 {
        out.extend(b"0".repeat(64));
        out.push(b'\n');
    }
    out.extend(b"cleartomark\n");
    out
}

/// The font's own encoding: A, B and `g1` at codes 65 to 67.
const OWN: &str = "256 array 0 1 255 {1 index exch /.notdef put} for dup 65 /A put dup 66 /B put dup 67 /g1 put readonly";

fn texts(r: &crate::Imported) -> Vec<TextObject> {
    all(&r.document)
        .iter()
        .filter_map(|n| match &n.kind {
            NodeKind::Text(t) => Some((**t).clone()),
            _ => None,
        })
        .collect()
}

fn program(encoding: &str, hex: bool, body: &str) -> crate::Imported {
    let head = b"%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 0 0 100 100\n%%EndComments\n".to_vec();
    import(&[head, font(encoding, hex), format!("{body}\nshowpage\n%%EOF\n").into_bytes()].concat()).unwrap()
}

#[test]
fn embedded_type1_type_keeps_the_fonts_widths() {
    for hex in [true, false] {
        let r = program(OWN, hex, "/TestOne findfont 50 scalefont setfont 10 20 moveto (AB) show");
        assert!(program_warnings(&r.warnings).is_empty(), "{:?}", r.warnings);
        let t = texts(&r);
        assert_eq!(t.len(), 1, "{:?}", all(&r.document));
        assert_eq!(t[0].plain_text(), "AB");
        assert_eq!(t[0].runs[0].style.font_family, "Test One");
        // B starts where A's 600 units end, whatever the font laid out.
        let o: Vec<f64> = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &t[0]).glyphs.iter().map(|g| g.origin.x).collect();
        assert!((o[1] - o[0] - 30.0).abs() < 0.05, "{o:?}");
    }
}

/// Type in an embedded font whose family isn't available keeps the font's name and shows in the
/// fallback font, and the import says so as PDF import does: each such family once, in the order
/// its type comes. A family that is available isn't named.
#[test]
fn type_in_fonts_that_arent_available_is_named_in_a_warning() {
    let r = program(
        OWN,
        false,
        "/TestOne findfont 50 scalefont setfont 10 20 moveto (AB) show 10 60 moveto (BA) show \
         /Nowhere-Bold 12 selectfont 10 80 moveto (x) show /Inter-Regular 12 selectfont 10 90 moveto (y) show",
    );
    assert_eq!(texts(&r).iter().map(|t| t.runs[0].style.font_family.as_str()).collect::<Vec<_>>(), ["Test One", "Test One", "Nowhere", "Inter"]);
    // PDF import's words.
    assert_eq!(r.warnings, ["fonts that aren't available show in the fallback font until they are: Test One, Nowhere"]);
}

#[test]
fn embedded_type1_fonts_measure_and_outline_their_glyphs() {
    let fonts = String::from_utf8(font(OWN, true)).unwrap();
    check(&format!("{fonts} /TestOne findfont 50 scalefont setfont (AB) stringwidth pop 55 sub abs 0.01 lt"));
    check(&format!(
        "{fonts} /TestOne findfont 50 scalefont setfont newpath 10 20 moveto (A) true charpath pathbbox 45 sub abs 0.01 lt \
         exch 35 sub abs 0.01 lt and exch 20 sub abs 0.01 lt and exch 15 sub abs 0.01 lt and"
    ));
    check(&format!("{fonts} /TestOne findfont /CharStrings get /A known"));
}

/// A glyph whose name says no character (`g1`) is drawn from the font.
#[test]
fn embedded_type1_glyphs_without_a_character_are_drawn() {
    let r = program(OWN, true, "/TestOne findfont 50 scalefont setfont 10 20 moveto (C) show");
    assert!(texts(&r).is_empty());
    let paths: Vec<_> = all(&r.document).iter().filter(|n| matches!(n.kind, NodeKind::Path { .. })).map(|n| bounds(n)).collect();
    assert_eq!(paths.len(), 1);
    assert!((paths[0].x0 - 10.0).abs() < 0.01 && (paths[0].width() - 25.0).abs() < 0.01 && (paths[0].height() - 20.0).abs() < 0.01, "{paths:?}");
}

/// A font in the standard encoding takes the standard names of its codes (PLRM 3rd ed.,
/// Appendix E.5), so its glyphs and widths are found.
#[test]
fn standard_encoding_fonts_find_their_glyphs() {
    let fonts = String::from_utf8(font("StandardEncoding", true)).unwrap();
    check(&format!("{fonts} /TestOne findfont 50 scalefont setfont (AB) stringwidth pop 55 sub abs 0.01 lt"));
    let r = read(&format!("{fonts} /TestOne findfont 50 scalefont setfont 10 20 moveto (AB) show"));
    assert_eq!(texts(&r)[0].plain_text(), "AB");
}

/// A font found by name (its program not in the file), re-encoded with glyph names at other
/// codes, sets the characters its names say, not its codes read as Latin-1.
#[test]
fn named_fonts_set_the_characters_their_glyph_names_say() {
    let r = read(
        "/Helvetica findfont dup length dict begin { 1 index /FID ne { def } { pop pop } ifelse } forall\n\
         /Encoding 256 array 0 1 255 { 1 index exch /.notdef put } for dup 1 /H put dup 2 /i put dup 3 /eacute put def\n\
         currentdict end /Helv-Re exch definefont pop /Helv-Re 20 selectfont 10 20 moveto (\\001\\002\\003) show",
    );
    assert_eq!(texts(&r).iter().map(|t| t.plain_text()).collect::<String>(), "Hié");
}

/// A font whose encoding is a copy of `StandardEncoding` (a different array, so not known as the
/// standard encoding) still finds each code's glyph by its character's name, not `.notdef`.
#[test]
fn copied_standard_encodings_find_their_glyphs() {
    let fonts = String::from_utf8(font("StandardEncoding 256 array copy", true)).unwrap();
    check(&format!("{fonts} /TestOne findfont 50 scalefont setfont (AB) stringwidth pop 55 sub abs 0.01 lt"));
    let r = read(&format!("{fonts} /TestOne findfont 50 scalefont setfont 10 20 moveto (AB) show"));
    assert_eq!(texts(&r)[0].plain_text(), "AB");
}

/// Subroutines 1 to 8 each calling the one below four times (`callsubr` 10, `return` 11), and
/// A a square drawn after calling subroutine 8: a few bytes that run some 260,000 operations.
fn heavy_font() -> String {
    let mut subrs = vec![cs(&[(&[], 11)])];
    for k in 1..=8 {
        subrs.push(cs(&[(&[k - 1], 10), (&[k - 1], 10), (&[k - 1], 10), (&[k - 1], 10), (&[], 11)]));
    }
    let a = cs(&[(&[0, 600], 13), (&[8], 10), (&[100, 0], 21), (&[400, 0], 5), (&[0, 500], 5), (&[-400, 0], 5), (&[], 9), (&[], 14)]);
    String::from_utf8(font_with(OWN, true, &subrs, Some(a))).unwrap()
}

/// Running a glyph's charstrings counts the bytes of every subroutine call, for the operation
/// budget; and the glyph still draws.
#[test]
fn running_a_type1_glyph_counts_its_subroutine_calls() {
    let program = heavy_font();
    let at = program.find("eexec\n").unwrap() + "eexec\n".len();
    let font = super::type1::parse(&program.as_bytes()[at..]).unwrap();
    let gid = font.font.glyph_names().find(|(_, n)| *n == "A").unwrap().0.to_u32();
    let font = super::type1::Embedded::Type1(std::rc::Rc::new(font));
    let (outline, w) = font.glyph(gid).unwrap();
    assert!((w - 0.6).abs() < 1e-9 && !outline.elements().is_empty(), "{w}");
    let work = font.take_work();
    assert!(work > 4u64.pow(8), "{work}");
    assert_eq!(font.take_work(), 0);
    check(&format!("{program} /TestOne findfont 50 scalefont setfont (A) stringwidth pop 30 sub abs 0.01 lt"));
}

/// The evaluator limits each glyph, not the file: measuring a glyph whose subroutines run a
/// quarter of a million operations in a loop ends within the operation budget.
#[test]
fn type1_glyphs_measured_in_a_loop_run_out_of_the_budget() {
    super::tests::runs_out_of_budget(&format!("{} /TestOne findfont 50 scalefont setfont {{ (A) stringwidth pop pop }} loop", heavy_font()));
}

/// One long string of a costly glyph pays for each glyph as it is measured: a million copies of
/// the glyph above stop at the operation budget after a few hundred, instead of all running first.
#[test]
fn a_long_string_of_type1_glyphs_runs_out_of_the_budget() {
    super::tests::runs_out_of_budget(&format!(
        "{} /TestOne findfont 50 scalefont setfont /s 1000000 string def 0 1 999999 {{ s exch 65 put }} for s stringwidth pop pop",
        heavy_font()
    ));
}

/// `eexec`'s search for `cleartomark` reads the encrypted part: on a large reusable file in a
/// loop it runs out of the operation budget instead of reading megabytes a pass for free.
#[test]
fn eexec_on_a_large_file_in_a_loop_runs_out_of_the_budget() {
    super::tests::runs_out_of_budget(
        "/s 4000000 string def s 3999980 (cleartomark) putinterval /f s /ReusableStreamDecode filter def\n\
         { f resetfile f eexec cleartomark } loop",
    );
}
