//! Type 1 fonts embedded in the file: their encrypted part (`eexec`) is read for their glyphs
//! (Adobe Type 1 Font Format, chapters 6 and 7), so their type keeps the font's own widths, and
//! glyphs that stand for no character are drawn from their outlines. The font here is written
//! from scratch: two square and triangle glyphs in hand-made charstrings.

use vectorcraft_doc::{NodeKind, TextObject};

use super::tests::{all, bounds, check, read};
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
    let square = cs(&[(&[0, 600], 13), (&[100, 0], 21), (&[400, 0], 5), (&[0, 500], 5), (&[-400, 0], 5), (&[], 9), (&[], 14)]);
    let triangle = cs(&[(&[0, 500], 13), (&[0, 0], 21), (&[500, 0], 5), (&[-250, 400], 5), (&[], 9), (&[], 14)]);
    let notdef = cs(&[(&[0, 250], 13), (&[], 14)]);
    let mut private = b"dup /Private 8 dict dup begin\n/RD {string currentfile exch readstring pop} executeonly def\n\
        /ND {noaccess def} executeonly def\n/NP {noaccess put} executeonly def\n/lenIV 4 def\n/password 5839 def\n\
        /MinFeature {16 16} def\n/Subrs 0 array def\n2 index /CharStrings 4 dict dup begin\n"
        .to_vec();
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
        assert!(r.warnings.is_empty(), "{:?}", r.warnings);
        let t = texts(&r);
        assert_eq!(t.len(), 1, "{:?}", all(&r.document));
        assert_eq!(t[0].plain_text(), "AB");
        assert_eq!(t[0].runs[0].style.font_family, "Test One");
        // B starts where A's 600 units end, whatever the font laid out.
        let o: Vec<f64> = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &t[0]).glyphs.iter().map(|g| g.origin.x).collect();
        assert!((o[1] - o[0] - 30.0).abs() < 0.05, "{o:?}");
    }
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
